import { createRequire } from "node:module";

import {
  BrowserSnapshotError,
} from "./browser-controller-snapshot.mjs";
import { BrowserFrameSessions, BrowserFrameTargets, captureBrowserFrames, registerBrowserFrameTargets, withBrowserActionFrame } from "./browser-controller-frames.mjs";
import {
  BrowserActionError,
  assertNotCancelled,
  performBrowserAction,
} from "./browser-controller-actions.mjs";
import {
  BrowserFileTransferError,
  DEFAULT_MINIMUM_DOWNLOAD_FREE_BYTES,
  assertBrowserDownloadHeadroom,
  configureBrowserDownloads,
  cancelBrowserDownload,
  uploadBrowserFiles,
} from "./browser-controller-files.mjs";
import {
  BrowserPermissionError,
  setBrowserPermission,
} from "./browser-controller-permissions.mjs";
import {
  BrowserEventError,
  BrowserEventJournal,
} from "./browser-controller-events.mjs";
import {
  BrowserCompatibilityError,
  navigateBrowser,
  waitForBrowserState,
} from "./browser-controller-compatibility.mjs";
import {
  BrowserHistoryError,
  navigateBrowserHistory,
} from "./browser-controller-history.mjs";
import { BrowserDialogDefaults } from "./browser-controller-dialogs.mjs";
import { acquireBrowserCookieWriterFence } from "./browser-controller-cookie-fence.mjs";
import { BrowserCdpAuthorityRegistry } from "./browser-controller-cdp-authority-registry.mjs";

const DEFAULT_DEBUGGER_ENDPOINT = "http://127.0.0.1:9222";
const DEFAULT_REQUEST_TIMEOUT_MS = 5_000;
export const MAX_CDP_FRAME_BYTES = 4 * 1024 * 1024;
const PERSISTENT_COOKIE_WRITER_TARGET_TYPES = new Set(["worker", "shared_worker"]);
const require = createRequire(import.meta.url);

function productionWebSocketConstructor() {
  const module = require("ws");
  return module.WebSocket ?? module.default ?? module;
}

export function createCappedWebSocket(
  url,
  { WebSocketClass = productionWebSocketConstructor() } = {},
) {
  return new WebSocketClass(url, { maxPayload: MAX_CDP_FRAME_BYTES });
}

export class BrowserControllerError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserControllerError";
    this.code = code;
  }
}

export class BrowserCdpClient {
  constructor({
    debuggerEndpoint = DEFAULT_DEBUGGER_ENDPOINT,
    requestTimeoutMs = DEFAULT_REQUEST_TIMEOUT_MS,
    fetchImpl = globalThis.fetch,
    webSocketFactory = createCappedWebSocket,
    connectionFactory,
    downloadDirectory,
    minimumDownloadFreeBytes = DEFAULT_MINIMUM_DOWNLOAD_FREE_BYTES,
    uploadRoots = [],
    fileSystem,
    eventJournal = new BrowserEventJournal(),
  } = {}) {
    this.debuggerEndpoint = new URL(debuggerEndpoint);
    this.requestTimeoutMs = requestTimeoutMs;
    this.fetchImpl = fetchImpl;
    this.webSocketFactory = webSocketFactory;
    this.connectionFactory = connectionFactory;
    this.downloadDirectory = downloadDirectory;
    this.minimumDownloadFreeBytes = minimumDownloadFreeBytes;
    this.uploadRoots = uploadRoots;
    this.fileSystem = fileSystem;
    this.eventJournal = eventJournal;
    this.connection = null;
    this.unsubscribeFromConnection = null;
    this.browserGeneration = 0;
    this.authorityRegistry = new BrowserCdpAuthorityRegistry();
    this.connectionAttempt = null;
    this.connectionAttemptSerial = 0;
    this.sessionsByTarget = new Map();
    this.targetsBySession = new Map();
    this.targetsByFrame = new BrowserFrameTargets();
    this.frameSessions = this.createFrameSessions();
    this.targetsByDownload = new Map();
    this.downloadCancellationReasons = new Map();
    this.downloadDiskCheckPending = false;
    this.downloadDiskCheckRequested = false;
    this.documentIdsByTarget = new Map();
    this.snapshotStateByTarget = new Map();
    this.dialogDefaults = new BrowserDialogDefaults();
    this.networkRequestsBySession = new Map();
    this.cookieWriterFence = null;
    this.cookieWriterFenceInUse = false;
  }

  createFrameSessions() {
    const targetsBySession = this.targetsBySession;
    return new BrowserFrameSessions(
      this.targetsByFrame,
      (id) => targetsBySession.get(id),
    );
  }

  resetAuthorityState() {
    const oldFrameSessions = this.frameSessions;
    this.sessionsByTarget = new Map();
    this.targetsBySession = new Map();
    this.targetsByFrame = new BrowserFrameTargets();
    this.frameSessions = this.createFrameSessions();
    this.targetsByDownload = new Map();
    this.downloadCancellationReasons = new Map();
    this.downloadDiskCheckPending = false;
    this.downloadDiskCheckRequested = false;
    this.documentIdsByTarget = new Map();
    this.snapshotStateByTarget = new Map();
    this.dialogDefaults = new BrowserDialogDefaults();
    this.networkRequestsBySession = new Map();
    this.cookieWriterFence = null;
    this.cookieWriterFenceInUse = false;
    void oldFrameSessions?.close();
  }

  requireAuthority(connection) {
    const authority = this.authorityRegistry.currentFor(connection, { ready: true });
    if (!authority) {
      throw new BrowserControllerError(
        "browser_cdp_disconnected",
        "browser CDP authority is no longer current",
      );
    }
    return authority;
  }

  assertAuthority(authority, { ready = true } = {}) {
    if (!this.authorityRegistry.isCurrent(authority, { ready })) {
      throw new BrowserControllerError(
        "browser_cdp_disconnected",
        "browser CDP authority was replaced",
      );
    }
  }

  observeTargetAuthority(authority, target) {
    this.assertAuthority(authority, { ready: false });
    const observed = this.authorityRegistry.observeTarget(
      authority,
      target?.targetId,
      target?.webSocketDebuggerUrl,
    );
    if (!observed) {
      this.assertAuthority(authority, { ready: false });
      throw new BrowserControllerError(
        "browser_target_not_found",
        `browser target ${JSON.stringify(target?.targetId)} has no current debugger authority`,
      );
    }
    if (observed.rotated) {
      const targetId = target.targetId;
      const sessionId = this.sessionsByTarget.get(targetId);
      if (sessionId) this.targetsBySession.delete(sessionId);
      this.sessionsByTarget.delete(targetId);
      this.documentIdsByTarget.delete(targetId);
      this.snapshotStateByTarget.delete(targetId);
      this.dialogDefaults.delete(targetId);
      this.targetsByFrame.removeTarget(targetId);
      void this.frameSessions.removeTarget(targetId);
    }
    return observed;
  }

  captureTargetOperation(connection, targetId, documentId) {
    const authority = this.requireAuthority(connection);
    const target = this.authorityRegistry.getTarget(authority, targetId);
    if (
      !target ||
      target.documentId !== documentId ||
      this.documentIdsByTarget.get(targetId) !== documentId
    ) {
      throw new BrowserControllerError(
        "stale_document_reference",
        `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
      );
    }
    return {
      authority,
      targetId,
      documentId,
      targetGeneration: target.targetGeneration,
      documentGeneration: target.documentGeneration,
      sessionId: target.sessionId,
    };
  }

  assertTargetOperation(operation) {
    this.assertAuthority(operation.authority);
    if (
      !this.authorityRegistry.isTargetCurrent(operation.authority, operation.targetId, {
        targetGeneration: operation.targetGeneration,
        documentGeneration: operation.documentGeneration,
        documentId: operation.documentId,
        sessionId: operation.sessionId,
      }) ||
      this.documentIdsByTarget.get(operation.targetId) !== operation.documentId
    ) {
      throw new BrowserControllerError(
        "stale_document_reference",
        `browser target ${JSON.stringify(operation.targetId)} moved away from the requested document`,
      );
    }
  }

  retireConnection(connection) {
    const authority = this.authorityRegistry.currentFor(connection, { ready: false });
    if (!authority) return false;
    this.authorityRegistry.abort(authority);
    if (this.connection === connection) {
      this.connection = null;
      this.unsubscribeFromConnection?.();
      this.unsubscribeFromConnection = null;
      this.resetAuthorityState();
    }
    return true;
  }

  async reconcile(rawViewport) {
    const viewport = canonicalViewport(rawViewport);
    const connection = await this.ensureConnection();
    const authority = this.requireAuthority(connection);
    try {
      const { targetInfos = [] } = await connection.send("Target.getTargets");
      this.assertAuthority(authority);
      const pages = targetInfos.filter(
        (target) => target?.type === "page" && typeof target.targetId === "string",
      ).filter((target) => !this.observeTargetAuthority(authority, target)?.stale);
      const writerTargets = targetInfos.filter(
        (target) => PERSISTENT_COOKIE_WRITER_TARGET_TYPES.has(target?.type)
          && typeof target.targetId === "string",
      ).filter((target) => !this.observeTargetAuthority(authority, target)?.stale);
      const persistentTargetIds = new Set(
        [...targetInfos.filter((target) =>
          (target?.type === "page" || PERSISTENT_COOKIE_WRITER_TARGET_TYPES.has(target?.type))
          && typeof target.targetId === "string",
        )].map((target) => target.targetId),
      );
      for (const targetId of this.sessionsByTarget.keys()) {
        if (!persistentTargetIds.has(targetId)) {
          const sessionId = this.sessionsByTarget.get(targetId);
          this.targetsBySession.delete(sessionId);
          this.networkRequestsBySession.delete(sessionId);
          this.sessionsByTarget.delete(targetId);
          this.documentIdsByTarget.delete(targetId);
          this.snapshotStateByTarget.delete(targetId);
          this.dialogDefaults.delete(targetId);
          this.targetsByFrame.removeTarget(targetId);
          await this.frameSessions.removeTarget(targetId);
          this.authorityRegistry.removeTarget(authority, targetId);
        }
      }
      const inspected = await Promise.all(
        pages.map((target) => this.inspectPage(connection, target, viewport, authority)),
      );
      await Promise.all(
        writerTargets.map((target) => this.ensureWriterTargetSession(connection, target.targetId, authority)),
      );
      this.assertAuthority(authority);
      const focused = inspected.find((tab) => tab.focused)?.target_id ?? null;
      return {
        browser_generation: authority.browserGeneration,
        tabs: inspected.map(({ focused: _focused, ...tab }) => tab),
        focused_target_id: focused,
        viewport,
        event_cursor: this.eventJournal.cursor(),
      };
    } catch (error) {
      if (!connection.isOpen() || !this.authorityRegistry.isCurrent(authority)) {
        this.retireConnection(connection);
      }
      throw normalizeControllerError(error);
    }
  }

  async close() {
    this.connectionAttemptSerial += 1;
    const connection = this.connection;
    this.connection = null;
    this.unsubscribeFromConnection?.();
    this.unsubscribeFromConnection = null;
    this.connectionAttempt = null;
    this.authorityRegistry.retireCurrent();
    this.resetAuthorityState();
    if (connection) {
      await connection.close();
    }
  }

  async withCookieWritersQuiesced(operation) {
    if (typeof operation !== "function" || this.cookieWriterFenceInUse) {
      throw new BrowserControllerError(
        "browser_cookie_writer_fence_busy",
        "browser cookie writer fence is already in use",
      );
    }
    const connection = await this.ensureConnection();
    const authority = this.requireAuthority(connection);
    if (!this.cookieWriterFence) {
      const { targetInfos = [] } = await connection.send("Target.getTargets");
      this.assertAuthority(authority);
      const pageTargets = targetInfos.filter(
        (target) => target?.type === "page" && typeof target.targetId === "string",
      );
      const pageSessions = await Promise.all(
        pageTargets.map((target) => this.ensureTargetSession(connection, target.targetId, authority)),
      );
      const writerTargets = targetInfos.filter(
        (target) => PERSISTENT_COOKIE_WRITER_TARGET_TYPES.has(target?.type)
          && typeof target.targetId === "string",
      );
      const knownWriterTargets = writerTargets.flatMap(({ targetId }) => {
        const sessionId = this.sessionsByTarget.get(targetId);
        return sessionId ? [{ targetId, sessionId }] : [];
      });
      this.cookieWriterFence = await acquireBrowserCookieWriterFence({
        connection,
        pageSessions,
        knownWriterTargets,
        waitForNetworkIdle: (sessions) => this.waitForNetworkIdle(sessions, authority),
        waitForWriterTargetsGone: (targetIds) => this.waitForWriterTargetsGone(connection, targetIds, authority),
      });
    }
    this.assertAuthority(authority);
    const fence = this.cookieWriterFence;
    this.cookieWriterFenceInUse = true;
    let retained = false;
    try {
      const result = await operation({ retain: () => { retained = true; } });
      this.assertAuthority(authority);
      return result;
    } finally {
      this.cookieWriterFenceInUse = false;
      if (!retained && this.cookieWriterFence === fence) {
        this.cookieWriterFence = null;
        await fence.release();
      }
    }
  }

  async waitForNetworkIdle(sessionIds, authority = this.requireAuthority(this.connection)) {
    const deadline = Date.now() + this.requestTimeoutMs;
    while (sessionIds.some((sessionId) => (this.networkRequestsBySession.get(sessionId)?.size ?? 0) > 0)) {
      this.assertAuthority(authority);
      if (Date.now() >= deadline) {
        throw new BrowserControllerError(
          "browser_cookie_writer_fence_timeout",
          "browser network did not quiesce before cookie import",
        );
      }
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }

  async waitForWriterTargetsGone(connection, targetIds, authority = this.requireAuthority(connection)) {
    const pending = new Set(targetIds);
    const deadline = Date.now() + this.requestTimeoutMs;
    while (true) {
      const { targetInfos = [] } = await connection.send("Target.getTargets");
      this.assertAuthority(authority);
      const live = targetInfos.some((target) => pending.has(target?.targetId));
      if (!live) return;
      const remainingMs = deadline - Date.now();
      if (remainingMs <= 0) {
        throw new BrowserControllerError(
          "browser_cookie_writer_fence_timeout",
          "browser writer targets did not close before cookie import",
        );
      }
      await new Promise((resolve) => setTimeout(resolve, Math.min(25, remainingMs)));
    }
  }

  async ensureConnection() {
    if (this.connection) {
      if (this.connection.isOpen() && this.authorityRegistry.currentFor(this.connection, { ready: true })) {
        return this.connection;
      }
      this.retireConnection(this.connection);
    }
    if (this.connectionAttempt) {
      return this.connectionAttempt;
    }
    const serial = ++this.connectionAttemptSerial;
    const attempt = (async () => {
      const connection = this.connectionFactory
        ? await this.connectionFactory()
        : await connectToBrowser({
            debuggerEndpoint: this.debuggerEndpoint,
            requestTimeoutMs: this.requestTimeoutMs,
            fetchImpl: this.fetchImpl,
            webSocketFactory: this.webSocketFactory,
          });
      if (serial !== this.connectionAttemptSerial) {
        await connection.close().catch(() => {});
        throw new BrowserControllerError(
          "browser_cdp_disconnected",
          "browser CDP connection attempt was superseded",
        );
      }
      const authority = this.authorityRegistry.begin(connection);
      this.browserGeneration = authority.browserGeneration;
      this.connection = connection;
      if (typeof connection.subscribe === "function") {
        this.unsubscribeFromConnection = connection.subscribe(
          (message) => this.recordConnectionEvent(message, authority),
        );
      }
      try {
        await connection.send("Target.setDiscoverTargets", { discover: true });
        this.assertAuthority(authority, { ready: false });
        this.authorityRegistry.commit(authority);
        this.eventJournal.recordCdp(
          { method: "Chariox.browserConnected", params: {} },
          this.eventContext(authority),
        );
        return connection;
      } catch (error) {
        this.retireConnection(connection);
        await connection.close().catch(() => {});
        throw error;
      }
    })();
    this.connectionAttempt = attempt;
    try {
      return await attempt;
    } finally {
      if (this.connectionAttempt === attempt) this.connectionAttempt = null;
    }
  }

  async inspectPage(connection, target, viewport, authority = this.requireAuthority(connection)) {
    const observed = this.observeTargetAuthority(authority, target);
    if (observed?.stale) {
      throw new BrowserControllerError(
        "browser_cdp_disconnected",
        `browser target ${JSON.stringify(target.targetId)} was observed on a retired debugger endpoint`,
      );
    }
    const sessionId = await this.ensureTargetSession(connection, target.targetId, authority);
    this.assertAuthority(authority);
    await connection.send(
      "Emulation.setDeviceMetricsOverride",
      deviceMetricsFor(viewport),
      sessionId,
    );
    const [frameTree, focus] = await Promise.all([
      connection.send("Page.getFrameTree", {}, sessionId),
      connection.send(
        "Runtime.evaluate",
        {
          expression: "document.visibilityState === 'visible'",
          returnByValue: true,
          awaitPromise: false,
        },
        sessionId,
      ),
    ]);
    this.assertAuthority(authority);
    const documentId = frameTree?.frameTree?.frame?.loaderId;
    if (typeof documentId !== "string" || !documentId) {
      throw new BrowserControllerError(
        "browser_document_identity_missing",
        `browser target ${JSON.stringify(target.targetId)} has no top-level loader identity`,
      );
    }
    this.documentIdsByTarget.set(target.targetId, documentId);
    if (!this.authorityRegistry.setDocument(authority, target.targetId, documentId)) {
      this.assertAuthority(authority);
    }
    await registerBrowserFrameTargets(connection, sessionId, target.targetId, documentId, this.targetsByFrame);
    this.assertAuthority(authority);
    return {
      target_id: target.targetId,
      document_id: documentId,
      url: typeof target.url === "string" ? target.url : "",
      title: typeof target.title === "string" ? target.title : "",
      focused: focus?.result?.value === true,
    };
  }

  async manageTab(rawRequest, { signal } = {}) {
    assertNotCancelled(signal);
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const action = rawRequest?.action;
    if (action !== "activate" && action !== "close") {
      throw new BrowserControllerError(
        "browser_tab_action_invalid",
        "browser tab action must be activate or close",
      );
    }
    const connection = await this.ensureConnection();
    const authority = this.requireAuthority(connection);
    const { targetInfos = [] } = await connection.send("Target.getTargets");
    this.assertAuthority(authority);
    const target = targetInfos.find(
      (candidate) => candidate?.type === "page" && candidate.targetId === targetId,
    );
    if (!target) {
      throw new BrowserControllerError(
        "browser_target_not_found",
        `browser target ${JSON.stringify(targetId)} is not available`,
      );
    }
    this.observeTargetAuthority(authority, target);
    this.captureTargetOperation(connection, targetId, documentId);
    assertNotCancelled(signal);
    if (action === "activate") {
      await connection.send("Target.activateTarget", { targetId });
    } else {
      const result = await connection.send("Target.closeTarget", { targetId });
      if (result?.success !== true) {
        throw new BrowserControllerError(
          "browser_tab_close_failed",
          `browser target ${JSON.stringify(targetId)} did not close`,
        );
      }
      await this.waitForTargetClosure(connection, targetId, authority);
    }
    this.assertAuthority(authority);
    return {
      browser_generation: authority.browserGeneration,
      target_id: targetId,
      document_id: documentId,
      action,
    };
  }

  async waitForTargetClosure(connection, targetId, authority = this.requireAuthority(connection)) {
    const deadline = Date.now() + this.requestTimeoutMs;
    while (true) {
      const { targetInfos = [] } = await connection.send("Target.getTargets");
      this.assertAuthority(authority);
      const targetStillOpen = targetInfos.some(
        (candidate) => candidate?.type === "page" && candidate.targetId === targetId,
      );
      if (!targetStillOpen) return;
      const remainingMs = deadline - Date.now();
      if (remainingMs <= 0) {
        throw new BrowserControllerError(
          "browser_tab_close_failed",
          `browser target ${JSON.stringify(targetId)} did not close within ${this.requestTimeoutMs}ms`,
        );
      }
      await new Promise((resolve) => setTimeout(resolve, Math.min(25, remainingMs)));
    }
  }

  async snapshot(rawRequest) {
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(
      rawRequest?.document_id,
      "document_id",
    );
    const connection = await this.ensureConnection();
    const authority = this.requireAuthority(connection);
    const { targetInfos = [] } = await connection.send("Target.getTargets");
    this.assertAuthority(authority);
    const target = targetInfos.find(
      (candidate) =>
        candidate?.type === "page" && candidate.targetId === targetId,
    );
    if (!target) {
      throw new BrowserControllerError(
        "browser_target_not_found",
        `browser target ${JSON.stringify(targetId)} is not available`,
      );
    }
    this.observeTargetAuthority(authority, target);
    const sessionId = await this.ensureTargetSession(connection, targetId, authority);
    const operation = this.captureTargetOperation(connection, targetId, documentId);
    const previous = this.snapshotStateByTarget.get(targetId);
    const snapshotRevision =
      previous?.documentId === documentId ? previous.revision + 1 : 1;
    this.snapshotStateByTarget.set(targetId, {
      documentId,
      revision: snapshotRevision,
    });
    try {
      const result = await captureBrowserFrames({
        connection,
        sessionId,
        targetId,
        documentId,
        browserGeneration: authority.browserGeneration,
        snapshotRevision,
      });
      this.assertTargetOperation(operation);
      return result;
    } catch (error) {
      if (this.authorityRegistry.isCurrent(authority) && this.snapshotStateByTarget.get(targetId)?.revision === snapshotRevision) {
        if (previous) {
          this.snapshotStateByTarget.set(targetId, previous);
        } else {
          this.snapshotStateByTarget.delete(targetId);
        }
      }
      throw normalizeControllerError(error);
    }
  }

  async performAction(rawRequest, { signal } = {}) {
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const connection = await this.ensureConnection();
    const authority = this.requireAuthority(connection);
    const { targetInfos = [] } = await connection.send("Target.getTargets");
    this.assertAuthority(authority);
    const target = targetInfos.find(
      (candidate) =>
        candidate?.type === "page" && candidate.targetId === targetId,
    );
    if (!target) {
      throw new BrowserControllerError(
        "browser_target_not_found",
        `browser target ${JSON.stringify(targetId)} is not available`,
      );
    }
    this.observeTargetAuthority(authority, target);
    const sessionId = await this.ensureTargetSession(connection, targetId, authority);
    const operation = this.captureTargetOperation(connection, targetId, documentId);
    try {
      const result = await withBrowserActionFrame({
        connection,
        sessionId,
        targetId,
        documentId,
        nodeRef: rawRequest?.node_ref,
        action: rawRequest?.action,
        timeoutMs: rawRequest?.timeout_ms,
        signal,
      }, performBrowserAction);
      this.assertTargetOperation(operation);
      return {
        browser_generation: authority.browserGeneration,
        ...result,
      };
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async navigate(rawRequest, { signal } = {}) {
    assertNotCancelled(signal);
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const { connection, sessionId, authority } = await this.resolvePageTarget(targetId, documentId);
    try {
      const result = await navigateBrowser({
        connection,
        sessionId,
        targetId,
        documentId,
        url: rawRequest?.url,
        signal,
      });
      this.assertAuthority(authority);
      this.documentIdsByTarget.set(targetId, result.document_id);
      this.authorityRegistry.setDocument(authority, targetId, result.document_id);
      this.snapshotStateByTarget.delete(targetId);
      return {
        browser_generation: authority.browserGeneration,
        ...result,
      };
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async manageHistory(rawRequest, { signal } = {}) {
    assertNotCancelled(signal);
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const { connection, sessionId, authority } = await this.resolvePageTarget(targetId, documentId);
    try {
      const result = await navigateBrowserHistory({
        connection,
        sessionId,
        targetId,
        documentId,
        action: rawRequest?.action,
        signal,
      });
      this.assertAuthority(authority);
      this.documentIdsByTarget.set(targetId, result.document_id);
      this.authorityRegistry.setDocument(authority, targetId, result.document_id);
      this.snapshotStateByTarget.delete(targetId);
      return {
        browser_generation: authority.browserGeneration,
        ...result,
      };
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async wait(rawRequest) {
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const { connection, sessionId, authority, operation } = await this.resolvePageTarget(targetId, documentId);
    try {
      const result = await waitForBrowserState({
          connection,
          sessionId,
          targetId,
          documentId,
          kind: rawRequest?.kind,
          selector: rawRequest?.selector,
          timeoutMs: rawRequest?.timeout_ms,
        });
      this.assertTargetOperation(operation);
      return { browser_generation: authority.browserGeneration, ...result };
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async handleDialog(rawRequest, { signal } = {}) {
    assertNotCancelled(signal);
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const action = rawRequest?.action;
    if (action !== "accept" && action !== "dismiss") {
      throw new BrowserControllerError(
        "browser_dialog_invalid",
        "browser dialog action must be accept or dismiss",
      );
    }
    const promptText = rawRequest?.prompt_text;
    if (
      promptText != null &&
      (typeof promptText !== "string" || !utf8ByteLengthAtMost(promptText, 2_048))
    ) {
      throw new BrowserControllerError(
        "browser_dialog_invalid",
        "browser dialog prompt text must not exceed 2048 UTF-8 bytes",
      );
    }
    const connection = await this.ensureConnection();
    const authority = this.requireAuthority(connection);
    const { targetInfos = [] } = await connection.send("Target.getTargets");
    this.assertAuthority(authority);
    const target = targetInfos.find(
      (candidate) => candidate?.type === "page" && candidate.targetId === targetId,
    );
    if (!target) {
      throw new BrowserControllerError(
        "browser_target_not_found",
        `browser target ${JSON.stringify(targetId)} is not available`,
      );
    }
    this.observeTargetAuthority(authority, target);
    const sessionId = await this.ensureTargetSession(connection, targetId, authority);
    const operation = this.captureTargetOperation(connection, targetId, documentId);
    const defaultPrompt = action === "accept" && promptText == null
      ? this.dialogDefaults.get(targetId, documentId)
      : undefined;
    if (defaultPrompt === null) {
      throw new BrowserControllerError(
        "browser_dialog_invalid",
        "browser dialog default exceeds 2048 UTF-8 bytes; supply an explicit prompt value or dismiss",
      );
    }
    const answer = promptText ?? defaultPrompt;
    assertNotCancelled(signal);
    await connection.send(
      "Page.handleJavaScriptDialog",
      {
        accept: action === "accept",
        ...(action === "accept" && answer != null ? { promptText: answer } : {}),
      },
      sessionId,
    );
    this.assertTargetOperation(operation);
    return {
      browser_generation: authority.browserGeneration,
      target_id: targetId,
      document_id: documentId,
      action,
    };
  }

  async configureDownloads(rawRequest, { signal } = {}) {
    assertNotCancelled(signal);
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const { connection, sessionId, authority, operation } = await this.resolvePageTarget(targetId, documentId);
    try {
      const result = await configureBrowserDownloads({
          connection,
          sessionId,
          targetId,
          documentId,
          downloadDirectory: this.downloadDirectory,
          minimumFreeBytes: this.minimumDownloadFreeBytes,
          fileSystem: this.fileSystem,
          signal,
        });
      this.assertTargetOperation(operation);
      return {
        browser_generation: authority.browserGeneration,
        ...result,
      };
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async cancelDownload(rawRequest) {
    const connection = await this.ensureConnection();
    const authority = this.requireAuthority(connection);
    try {
      const result = await cancelBrowserDownload({
        connection,
        browserGeneration: authority.browserGeneration,
        requestedBrowserGeneration: rawRequest?.browser_generation,
        guid: rawRequest?.guid,
        targetsByDownload: this.targetsByDownload,
      });
      this.assertAuthority(authority);
      return result;
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async uploadFiles(rawRequest, { signal } = {}) {
    assertNotCancelled(signal);
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const { connection, sessionId, authority, operation } = await this.resolvePageTarget(targetId, documentId);
    try {
      const result = await withBrowserActionFrame({
          connection,
          sessionId,
          targetId,
          documentId,
          nodeRef: rawRequest?.node_ref,
          filePaths: rawRequest?.file_paths,
          uploadRoots: this.uploadRoots,
          fileSystem: this.fileSystem,
          signal,
        }, uploadBrowserFiles);
      this.assertTargetOperation(operation);
      return {
        browser_generation: authority.browserGeneration,
        ...result,
      };
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async setPermission(rawRequest, { signal } = {}) {
    assertNotCancelled(signal);
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const { connection, sessionId, authority, target, operation } = await this.resolvePageTarget(targetId, documentId);
    try {
      const result = await setBrowserPermission({
          connection,
          sessionId,
          targetId,
          documentId,
          targetUrl: target.url,
          permission: rawRequest?.permission,
          setting: rawRequest?.setting,
          signal,
        });
      this.assertTargetOperation(operation);
      return {
        browser_generation: authority.browserGeneration,
        ...result,
      };
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  pollEvents(rawRequest) {
    try {
      return this.eventJournal.poll({
        browserGeneration: rawRequest?.browser_generation,
        cursor: rawRequest?.cursor,
        limit: rawRequest?.limit,
      });
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async resolvePageTarget(targetId, documentId) {
    const connection = await this.ensureConnection();
    const authority = this.requireAuthority(connection);
    const { targetInfos = [] } = await connection.send("Target.getTargets");
    this.assertAuthority(authority);
    const target = targetInfos.find(
      (candidate) => candidate?.type === "page" && candidate.targetId === targetId,
    );
    if (!target) {
      throw new BrowserControllerError(
        "browser_target_not_found",
        `browser target ${JSON.stringify(targetId)} is not available`,
      );
    }
    const observed = this.observeTargetAuthority(authority, target);
    if (observed?.stale) {
      throw new BrowserControllerError(
        "browser_cdp_disconnected",
        `browser target ${JSON.stringify(targetId)} was observed on a retired debugger endpoint`,
      );
    }
    const sessionId = await this.ensureTargetSession(connection, targetId, authority);
    return {
      connection,
      target,
      authority,
      sessionId,
      operation: documentId === undefined
        ? null
        : this.captureTargetOperation(connection, targetId, documentId),
    };
  }

  async ensureTargetSession(connection, targetId, authority = this.requireAuthority(connection)) {
    this.assertAuthority(authority);
    let sessionId = this.sessionsByTarget.get(targetId);
    if (sessionId && this.authorityRegistry.isSessionCurrent(authority, targetId, sessionId)) {
      return sessionId;
    }
    if (sessionId) {
      this.sessionsByTarget.delete(targetId);
      this.targetsBySession.delete(sessionId);
      this.authorityRegistry.clearSession(authority, targetId, sessionId);
    }
    const attached = await connection.send("Target.attachToTarget", {
      targetId,
      flatten: true,
    });
    this.assertAuthority(authority);
    if (typeof attached?.sessionId !== "string" || !attached.sessionId) {
      throw new BrowserControllerError(
        "browser_attach_failed",
        `browser target ${JSON.stringify(targetId)} did not return a session`,
      );
    }
    sessionId = attached.sessionId;
    this.sessionsByTarget.set(targetId, sessionId);
    this.targetsBySession.set(sessionId, targetId);
    const frameSessions = this.frameSessions;
    if (!this.authorityRegistry.setSession(authority, targetId, sessionId)) {
      await connection.send("Target.detachFromTarget", { sessionId }).catch(() => {});
      throw new BrowserControllerError(
        "browser_cdp_disconnected",
        "browser CDP authority was replaced while attaching a target",
      );
    }
    try {
      await Promise.all([
        connection.send("Page.enable", {}, sessionId),
        connection.send("Page.setLifecycleEventsEnabled", { enabled: true }, sessionId),
        connection.send("Runtime.enable", {}, sessionId),
        connection.send("Network.enable", {}, sessionId),
        connection.send("Inspector.enable", {}, sessionId),
        frameSessions.start(connection, sessionId),
      ]);
      this.assertAuthority(authority);
    } catch (error) {
      if (this.authorityRegistry.isCurrent(authority, { ready: false })) {
        if (this.sessionsByTarget.get(targetId) === sessionId) this.sessionsByTarget.delete(targetId);
        this.targetsBySession.delete(sessionId);
        this.authorityRegistry.clearSession(authority, targetId, sessionId);
        await frameSessions.removeTarget(targetId);
      }
      await connection.send("Target.detachFromTarget", { sessionId }).catch(() => {});
      throw error;
    }
    return sessionId;
  }

  async ensureWriterTargetSession(connection, targetId, authority = this.requireAuthority(connection)) {
    this.assertAuthority(authority);
    let sessionId = this.sessionsByTarget.get(targetId);
    if (sessionId && this.authorityRegistry.isSessionCurrent(authority, targetId, sessionId)) return sessionId;
    if (sessionId) {
      this.sessionsByTarget.delete(targetId);
      this.targetsBySession.delete(sessionId);
      this.authorityRegistry.clearSession(authority, targetId, sessionId);
    }
    const attached = await connection.send("Target.attachToTarget", {
      targetId,
      flatten: true,
    });
    this.assertAuthority(authority);
    if (typeof attached?.sessionId !== "string" || !attached.sessionId) {
      throw new BrowserControllerError(
        "browser_attach_failed",
        `browser writer target ${JSON.stringify(targetId)} did not return a session`,
      );
    }
    sessionId = attached.sessionId;
    this.sessionsByTarget.set(targetId, sessionId);
    this.targetsBySession.set(sessionId, targetId);
    if (!this.authorityRegistry.setSession(authority, targetId, sessionId)) {
      await connection.send("Target.detachFromTarget", { sessionId }).catch(() => {});
      throw new BrowserControllerError(
        "browser_cdp_disconnected",
        "browser CDP authority was replaced while attaching a writer target",
      );
    }
    try {
      await Promise.all([
        connection.send("Network.enable", {}, sessionId),
        connection.send("Debugger.enable", {}, sessionId),
      ]);
    } catch (error) {
      if (this.sessionsByTarget.get(targetId) === sessionId) this.sessionsByTarget.delete(targetId);
      this.targetsBySession.delete(sessionId);
      this.authorityRegistry.clearSession(authority, targetId, sessionId);
      await connection.send("Target.detachFromTarget", { sessionId }).catch(() => {});
      throw error;
    }
    return sessionId;
  }

  recordConnectionEvent(
    message,
    authority = this.authorityRegistry.currentFor(this.connection, { ready: true }),
  ) {
    if (!authority || !this.authorityRegistry.isCurrent(authority, { ready: true })) return;
    const connection = authority.connection;
    if (message?.method === "Target.targetInfoChanged" && message.params?.targetInfo) {
      const observed = this.observeTargetAuthority(authority, message.params.targetInfo);
      if (observed?.stale) return;
    }
    if (message?.method === "Network.requestWillBeSent" && typeof message.sessionId === "string"
        && typeof message.params?.requestId === "string") {
      const requests = this.networkRequestsBySession.get(message.sessionId) ?? new Set();
      requests.add(message.params.requestId);
      this.networkRequestsBySession.set(message.sessionId, requests);
    }
    if (["Network.loadingFinished", "Network.loadingFailed"].includes(message?.method)
        && typeof message.sessionId === "string" && typeof message.params?.requestId === "string") {
      const requests = this.networkRequestsBySession.get(message.sessionId);
      requests?.delete(message.params.requestId);
      if (requests?.size === 0) this.networkRequestsBySession.delete(message.sessionId);
    }
    if (this.frameSessions.observe(message, connection)) return;
    const dialogTargetId = this.targetsBySession.get(message?.sessionId) ?? message?.params?.targetId;
    this.dialogDefaults.observe(message, dialogTargetId, this.documentIdsByTarget.get(dialogTargetId));
    if (message?.method === "Target.detachedFromTarget") {
      const sessionId = message.params?.sessionId ?? message.sessionId;
      const targetId = this.targetsBySession.get(sessionId) ?? message.params?.targetId;
      if (typeof sessionId === "string") {
        this.targetsBySession.delete(sessionId);
        this.networkRequestsBySession.delete(sessionId);
      }
      if (typeof targetId === "string") {
        this.sessionsByTarget.delete(targetId);
        this.dialogDefaults.delete(targetId);
        this.targetsByFrame.removeTarget(targetId);
        void this.frameSessions.removeTarget(targetId);
        this.authorityRegistry.clearSession(authority, targetId, sessionId);
      }
    }
    this.targetsByFrame.record(message, this.targetsBySession.get(message?.sessionId));
    if (message?.method === "Page.frameNavigated" && !message.params?.frame?.parentId) {
      const targetId = this.targetsBySession.get(message.sessionId);
      const documentId = message.params?.frame?.loaderId;
      if (targetId && typeof documentId === "string" && documentId) {
        this.documentIdsByTarget.set(targetId, documentId);
        this.authorityRegistry.setDocument(authority, targetId, documentId);
      }
    }
    if (message?.method === "Browser.downloadWillBegin") {
      const targetId = this.targetsByFrame.get(message.params?.frameId);
      const guid = message.params?.guid;
      if (typeof guid === "string" && guid) {
        this.targetsByDownload.set(guid, targetId ?? null);
        this.scheduleDownloadDiskCheck(authority);
      }
    }
    if (
      message?.method === "Browser.downloadProgress" &&
      message.params?.state === "inProgress"
    ) {
      this.scheduleDownloadDiskCheck(authority);
    }
    this.eventJournal.recordCdp(message, this.eventContext(authority));
    if (
      message?.method === "Browser.downloadProgress" &&
      message.params?.state !== "inProgress"
    ) {
      const guid = message.params?.guid;
      if (typeof guid === "string") {
        this.targetsByDownload.delete(guid);
        this.downloadCancellationReasons.delete(guid);
      }
    }
  }

  scheduleDownloadDiskCheck(authority = this.authorityRegistry.currentFor(this.connection, { ready: true })) {
    if (!authority || !this.authorityRegistry.isCurrent(authority, { ready: true })) return;
    if (this.targetsByDownload.size === 0) return;
    if (this.downloadDiskCheckPending) {
      this.downloadDiskCheckRequested = true;
      return;
    }
    this.downloadDiskCheckPending = true;
    const connection = authority.connection;
    const browserGeneration = authority.browserGeneration;
    void assertBrowserDownloadHeadroom({
      downloadDirectory: this.downloadDirectory,
      minimumFreeBytes: this.minimumDownloadFreeBytes,
      fileSystem: this.fileSystem,
    }).catch(async (error) => {
      if (
        ![
          "browser_download_low_disk",
          "browser_download_unavailable",
          "browser_download_unconfigured",
        ].includes(error?.code) ||
        connection !== this.connection ||
        browserGeneration !== this.browserGeneration ||
        !this.authorityRegistry.isCurrent(authority, { ready: true })
      ) return;
      const active = [...this.targetsByDownload.keys()]
        .filter((guid) => !this.downloadCancellationReasons.has(guid));
      await Promise.all(active.map(async (guid) => {
        this.downloadCancellationReasons.set(guid, "disk_pressure");
        try {
          await connection.send("Browser.cancelDownload", { guid });
        } catch {
          this.downloadCancellationReasons.delete(guid);
        }
      }));
    }).finally(() => {
      if (
        connection !== this.connection ||
        browserGeneration !== this.browserGeneration ||
        !this.authorityRegistry.isCurrent(authority, { ready: true })
      ) return;
      this.downloadDiskCheckPending = false;
      if (this.downloadDiskCheckRequested) {
        this.downloadDiskCheckRequested = false;
        this.scheduleDownloadDiskCheck();
      }
    });
  }

  eventContext(authority = this.authorityRegistry.current) {
    return {
      browserGeneration: authority?.browserGeneration ?? this.browserGeneration,
      targetIdForSession: (sessionId) => this.targetsBySession.get(sessionId) ?? null,
      targetIdForFrame: (frameId) => this.targetsByFrame.get(frameId) ?? null,
      targetIdForDownload: (guid) => this.targetsByDownload.get(guid) ?? null,
      downloadCancellationReason: (guid) => this.downloadCancellationReasons.get(guid) ?? null,
      documentIdForTarget: (targetId) => this.documentIdsByTarget.get(targetId) ?? null,
    };
  }
}

export class CdpConnection {
  constructor(
    socket,
    requestTimeoutMs = DEFAULT_REQUEST_TIMEOUT_MS,
    maximumFrameBytes = MAX_CDP_FRAME_BYTES,
  ) {
    this.socket = socket;
    this.requestTimeoutMs = requestTimeoutMs;
    this.maximumFrameBytes = maximumFrameBytes;
    this.nextRequestId = 1;
    this.pending = new Map();
    this.eventWaiters = new Map();
    this.eventListeners = new Set();
    this.disconnectEventSent = false;
    this.closed = false;
    socket.addEventListener("message", (event) => this.receive(event.data));
    socket.addEventListener("close", () => this.failPending("browser_cdp_disconnected"));
    socket.addEventListener("error", () => this.failPending("browser_cdp_socket_error"));
  }

  isOpen() {
    return !this.closed && this.socket.readyState === 1;
  }

  send(method, params = {}, sessionId) {
    if (!this.isOpen()) {
      return Promise.reject(
        new BrowserControllerError("browser_cdp_disconnected", "browser CDP connection is closed"),
      );
    }
    const id = this.nextRequestId++;
    const payload = { id, method, params };
    if (sessionId) {
      payload.sessionId = sessionId;
    }
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        reject(
          new BrowserControllerError(
            "browser_cdp_timeout",
            `browser CDP method ${method} timed out after ${this.requestTimeoutMs}ms`,
          ),
        );
      }, this.requestTimeoutMs);
      this.pending.set(id, { method, resolve, reject, timeout });
      try {
        this.socket.send(JSON.stringify(payload));
      } catch (error) {
        clearTimeout(timeout);
        this.pending.delete(id);
        reject(normalizeControllerError(error));
      }
    });
  }

  waitForEvent(method, timeoutMs, sessionId) {
    let waiter;
    const promise = new Promise((resolve) => {
      const timeout = setTimeout(() => {
        this.removeEventWaiter(method, waiter);
        resolve(null);
      }, timeoutMs);
      waiter = { sessionId, resolve, timeout };
      const waiters = this.eventWaiters.get(method) ?? new Set();
      waiters.add(waiter);
      this.eventWaiters.set(method, waiters);
    });
    return {
      promise,
      cancel: () => {
        if (!waiter) return;
        clearTimeout(waiter.timeout);
        this.removeEventWaiter(method, waiter);
        waiter.resolve(null);
      },
    };
  }

  subscribe(listener) {
    this.eventListeners.add(listener);
    return () => this.eventListeners.delete(listener);
  }

  async close() {
    if (this.closed) {
      return;
    }
    this.closed = true;
    this.failPending("browser_cdp_shutdown");
    if (this.socket.readyState === 0 || this.socket.readyState === 1) {
      this.socket.close();
    }
  }

  receive(rawMessage) {
    if (rawFrameByteLength(rawMessage) > this.maximumFrameBytes) {
      this.failPending("browser_cdp_frame_too_large");
      if (this.socket.readyState === 0 || this.socket.readyState === 1) {
        this.socket.close();
      }
      return;
    }
    let message;
    try {
      message = JSON.parse(String(rawMessage));
    } catch {
      this.failPending("browser_cdp_invalid_json");
      return;
    }
    if (typeof message?.method === "string") {
      this.dispatchEvent(message);
    }
    if (!Number.isSafeInteger(message?.id)) {
      return;
    }
    const pending = this.pending.get(message.id);
    if (!pending) {
      return;
    }
    this.pending.delete(message.id);
    clearTimeout(pending.timeout);
    if (message.error) {
      pending.reject(
        new BrowserControllerError(
          "browser_cdp_command_failed",
          `${pending.method}: ${String(message.error.message ?? "CDP command failed")}`,
        ),
      );
      return;
    }
    pending.resolve(message.result ?? {});
  }

  failPending(code) {
    this.closed = true;
    if (!this.disconnectEventSent) {
      this.disconnectEventSent = true;
      this.notifyListeners({ method: "Chariox.browserDisconnected", params: { code } });
    }
    for (const pending of this.pending.values()) {
      clearTimeout(pending.timeout);
      pending.reject(new BrowserControllerError(code, "browser CDP connection closed"));
    }
    this.pending.clear();
    for (const [method, waiters] of this.eventWaiters) {
      for (const waiter of waiters) {
        clearTimeout(waiter.timeout);
        waiter.resolve(null);
      }
      this.eventWaiters.delete(method);
    }
  }

  dispatchEvent(message) {
    this.notifyListeners(message);
    const waiters = this.eventWaiters.get(message.method);
    if (!waiters) return;
    for (const waiter of [...waiters]) {
      if (waiter.sessionId && waiter.sessionId !== message.sessionId) continue;
      clearTimeout(waiter.timeout);
      this.removeEventWaiter(message.method, waiter);
      waiter.resolve(message);
    }
  }

  notifyListeners(message) {
    for (const listener of this.eventListeners) {
      try {
        listener(message);
      } catch {
        // A consumer cannot interrupt CDP response handling.
      }
    }
  }

  removeEventWaiter(method, waiter) {
    const waiters = this.eventWaiters.get(method);
    if (!waiters) return;
    waiters.delete(waiter);
    if (waiters.size === 0) this.eventWaiters.delete(method);
  }
}

function rawFrameByteLength(value) {
  if (typeof value === "string") return Buffer.byteLength(value, "utf8");
  if (value instanceof ArrayBuffer) return value.byteLength;
  if (ArrayBuffer.isView(value)) return value.byteLength;
  return Buffer.byteLength(String(value), "utf8");
}

export function assertPrivateDebuggerUrl(rawUrl, debuggerEndpoint = DEFAULT_DEBUGGER_ENDPOINT) {
  const url = new URL(rawUrl);
  const endpoint = new URL(debuggerEndpoint);
  if (
    url.protocol !== "ws:" ||
    !isLoopbackHost(url.hostname) ||
    normalizedPort(url) !== normalizedPort(endpoint)
  ) {
    throw new BrowserControllerError(
      "browser_debugger_endpoint_unsafe",
      "browser debugger WebSocket must remain on the configured loopback port",
    );
  }
  return url.toString();
}

function canonicalViewport(viewport) {
  const keys = [
    "css_width",
    "css_height",
    "device_scale_factor",
    "desktop_pixel_width",
    "desktop_pixel_height",
  ];
  const canonical = {};
  for (const key of keys) {
    const value = viewport?.[key];
    if (!Number.isSafeInteger(value) || value <= 0) {
      throw new BrowserControllerError(
        "browser_viewport_invalid",
        `browser viewport ${key} must be a positive integer`,
      );
    }
    canonical[key] = value;
  }
  return canonical;
}

function requiredIdentity(value, field) {
  if (typeof value !== "string" || !value) {
    throw new BrowserControllerError(
      "browser_snapshot_invalid",
      `browser snapshot ${field} must be a non-empty string`,
    );
  }
  return value;
}

function utf8ByteLengthAtMost(value, limit) {
  let length = 0;
  for (const character of value) {
    const codePoint = character.codePointAt(0);
    length += codePoint <= 0x7f
      ? 1
      : codePoint <= 0x7ff
        ? 2
        : codePoint <= 0xffff
          ? 3
          : 4;
    if (length > limit) return false;
  }
  return true;
}

function deviceMetricsFor(viewport) {
  return {
    width: viewport.css_width,
    height: viewport.css_height,
    deviceScaleFactor: viewport.device_scale_factor,
    mobile: false,
    screenWidth: Math.floor(viewport.desktop_pixel_width / viewport.device_scale_factor),
    screenHeight: Math.floor(viewport.desktop_pixel_height / viewport.device_scale_factor),
  };
}

async function connectToBrowser({
  debuggerEndpoint,
  requestTimeoutMs,
  fetchImpl,
  webSocketFactory,
}) {
  let response;
  try {
    response = await fetchImpl(new URL("/json/version", debuggerEndpoint), {
      signal: AbortSignal.timeout(requestTimeoutMs),
    });
  } catch (error) {
    throw new BrowserControllerError(
      "browser_debugger_unavailable",
      `browser debugger discovery failed: ${String(error?.message ?? error)}`,
    );
  }
  if (!response.ok) {
    throw new BrowserControllerError(
      "browser_debugger_unavailable",
      `browser debugger discovery returned HTTP ${response.status}`,
    );
  }
  const discovery = await response.json();
  if (typeof discovery?.webSocketDebuggerUrl !== "string") {
    throw new BrowserControllerError(
      "browser_debugger_invalid",
      "browser debugger discovery omitted its WebSocket URL",
    );
  }
  const debuggerUrl = assertPrivateDebuggerUrl(
    discovery.webSocketDebuggerUrl,
    debuggerEndpoint,
  );
  const socket = webSocketFactory(debuggerUrl);
  await waitForSocketOpen(socket, requestTimeoutMs);
  return new CdpConnection(socket, requestTimeoutMs);
}

function waitForSocketOpen(socket, timeoutMs) {
  if (socket.readyState === 1) {
    return Promise.resolve();
  }
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(
        new BrowserControllerError(
          "browser_cdp_timeout",
          `browser CDP socket did not open within ${timeoutMs}ms`,
        ),
      );
    }, timeoutMs);
    const opened = () => {
      cleanup();
      resolve();
    };
    const failed = () => {
      cleanup();
      reject(
        new BrowserControllerError("browser_cdp_socket_error", "browser CDP socket failed"),
      );
    };
    const cleanup = () => {
      clearTimeout(timeout);
      socket.removeEventListener("open", opened);
      socket.removeEventListener("error", failed);
    };
    socket.addEventListener("open", opened, { once: true });
    socket.addEventListener("error", failed, { once: true });
  });
}

function isLoopbackHost(hostname) {
  const normalized = hostname.replace(/^\[|\]$/g, "").toLowerCase();
  return normalized === "127.0.0.1" || normalized === "localhost" || normalized === "::1";
}

function normalizedPort(url) {
  if (url.port) {
    return url.port;
  }
  return url.protocol === "https:" || url.protocol === "wss:" ? "443" : "80";
}

function normalizeControllerError(error) {
  if (error instanceof BrowserControllerError) {
    return error;
  }
  if (error instanceof BrowserSnapshotError) {
    return new BrowserControllerError(error.code, error.message);
  }
  if (error instanceof BrowserActionError) {
    return new BrowserControllerError(error.code, error.message);
  }
  if (error instanceof BrowserFileTransferError) {
    return new BrowserControllerError(error.code, error.message);
  }
  if (error instanceof BrowserPermissionError) {
    return new BrowserControllerError(error.code, error.message);
  }
  if (error instanceof BrowserEventError) {
    return new BrowserControllerError(error.code, error.message);
  }
  if (error instanceof BrowserCompatibilityError) {
    return new BrowserControllerError(error.code, error.message);
  }
  if (error instanceof BrowserHistoryError) {
    return new BrowserControllerError(error.code, error.message);
  }
  return new BrowserControllerError(
    "browser_controller_internal",
    String(error?.message ?? error),
  );
}
