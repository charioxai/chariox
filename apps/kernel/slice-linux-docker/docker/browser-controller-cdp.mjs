import {pageObservationFence,nativeStatePage} from './browser-controller-frames.mjs';
import {FOCUS_WORLD,ensureFocusWorld,recordFocusWorld} from './browser-focus-world.mjs';
import { BrowserResponseBodies, responseBufferLimits } from './browser-controller-response-bodies.mjs';
import { BrowserMemory } from './browser-memory.mjs';
import { BrowserRendererRecovery } from './browser-renderer-recovery.mjs';
import { recordBrowserFill, finishBrowserFill } from './browser-protection-regions.mjs';
import { createHash } from "node:crypto";
import { artifactBytes, BrowserArtifactError, BrowserPassiveCapture, readCompletedDownload, withUploadArtifacts } from "./browser-controller-artifacts.mjs";
import { captureProtectedBrowserImage } from "./browser-controller-image.mjs";
import { observeBrowserNote } from "./browser-controller-notes.mjs";
import { redactObservation } from "./browser-controller-snapshot.mjs";
import { BrowserInputCapture } from "./browser-controller-input.mjs";
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
import { applyBrowserBar } from "./browser-controller-bar.mjs";
import { acquireBrowserCookieWriterFence } from "./browser-controller-cookie-fence.mjs";
import {retireNativeStates} from './kernel-browser-native-worker.mjs';

const DEFAULT_DEBUGGER_ENDPOINT = "http://127.0.0.1:9222";
const DEFAULT_REQUEST_TIMEOUT_MS = 5_000;
const PERSISTENT_COOKIE_WRITER_TARGET_TYPES = new Set(["worker", "shared_worker"]);
// Focus is read in a controller-owned isolated world: a page can redefine
// document.visibilityState in its own world, but not in this one.

export class BrowserControllerError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserControllerError";
    this.code = code;
  }
}

// Private provenance survives adapter normalization without trusting a caller's
// code, message, name or public BrowserControllerError constructor.
const normalizedStaleReferences = new WeakSet();
export function isTrustedStaleReferenceError(error) {
  return normalizedStaleReferences.has(error) || (
    (error instanceof BrowserActionError || error instanceof BrowserCompatibilityError
      || error instanceof BrowserSnapshotError)
    && ["stale_document_reference", "stale_element_reference"].includes(error.code)
  );
}
function normalizeNativeError(error) {
  const normalized = new BrowserControllerError(error.code, error.message);
  if (isTrustedStaleReferenceError(error)) normalizedStaleReferences.add(normalized);
  return normalized;
}

/** A CDP failure because the target or its session no longer exists. */
function targetGone(error) {
  return /Session with given id not found|No target with given id|Target closed/.test(String(error?.message ?? error));
}

export class BrowserCdpClient {
  constructor({
    debuggerEndpoint = DEFAULT_DEBUGGER_ENDPOINT,
    requestTimeoutMs = DEFAULT_REQUEST_TIMEOUT_MS,
    fetchImpl = globalThis.fetch,
    webSocketFactory = (url) => new WebSocket(url),
    connectionFactory,
    downloadDirectory,
    minimumDownloadFreeBytes = DEFAULT_MINIMUM_DOWNLOAD_FREE_BYTES,
    uploadRoots = [],
    fileSystem,
    stageUploads,
    eventJournal = new BrowserEventJournal(),
    applyCanonicalDisplay,
    focusWorldSource,
    memory = new BrowserMemory(),
  } = {}) {
    this.memory = memory;
    this.rendererRecovery = new BrowserRendererRecovery(this);
    this.applyCanonicalDisplay = applyCanonicalDisplay;
    this.focusWorldSource = focusWorldSource;
    this.debuggerEndpoint = new URL(debuggerEndpoint);
    this.requestTimeoutMs = requestTimeoutMs;
    this.fetchImpl = fetchImpl;
    this.webSocketFactory = webSocketFactory;
    this.connectionFactory = connectionFactory;
    this.downloadDirectory = downloadDirectory;
    this.minimumDownloadFreeBytes = minimumDownloadFreeBytes;
    this.uploadRoots = uploadRoots;
    this.fileSystem = fileSystem;
    this.stageUploads = stageUploads;
    this.eventJournal = eventJournal;
    this.connection = null;
    this.connectionEpoch = 0;
    this.connectionAttempt = null;
    this.pageCloseQueue = Promise.resolve();
    this.unsubscribeFromConnection = null;
    this.browserGeneration = 0;
    this.sessionsByTarget = new Map();
    this.targetSessionAttempts = new Map();
    this.targetsBySession = new Map();
    this.targetsByFrame = new BrowserFrameTargets();
    this.frameSessions = new BrowserFrameSessions(this.targetsByFrame, (id) => this.targetsBySession.get(id));
    this.targetsByDownload = new Map();
    this.downloads = new Map();
    this.passiveCapture = new BrowserPassiveCapture();
    this.downloadCancellationReasons = new Map();
    this.downloadDiskCheckPending = false;
    this.downloadDiskCheckRequested = false;
    this.documentIdsByTarget = new Map();
    this.commitWaiters = new Map();
    this.pageFramesByTarget = new Map();
    // Records that loading start retired until a same-document navigation proves the document unchanged.
    this.suspendedPageFrames = new Map();
    this.focusWorldsByTarget = new Map();
    this.snapshotStateByTarget = new Map();
    this.protectedValues = new Set();
    this.fillTargets = new Map();
    this.fillRevision = 0;
    this.dialogDefaults = new BrowserDialogDefaults();
    this.networkRequestsBySession = new Map();
    this.responseBodies = new BrowserResponseBodies();
    this.cookieWriterFence = null;
    this.cookieWriterFenceInUse = false;
    this.inputCapture = new BrowserInputCapture();
    this.appliedViewport = null;
    this.viewportByTarget = new Map();
    this.contentInsetsByWindow = new Map();
    this.layoutQueue = new Map();
    this.layoutByTarget = new Map();
    this.canonical = null;
  }

  // MP-08/MP-10: discovery is observational once the canonical viewport, and
  // the Room browser bar and App panel layout sent with it, are applied.
  // Resolves when this target's main frame commits a document other than `documentId` (at once when it already has): a
  // long poll in the old document's page need not wait for its own timer to notice a navigation.
  whenDocumentMoves(targetId, documentId) {
    let wake = () => {}, waiters;
    const promise = new Promise(resolve => {
      wake = resolve;
      if (this.documentIdsByTarget.get(targetId) !== documentId) return resolve();
      waiters = this.commitWaiters.get(targetId) ?? new Set(); this.commitWaiters.set(targetId, waiters); waiters.add(resolve);
    });
    return { promise, cancel: () => { waiters?.delete(wake); if (waiters?.size === 0 && this.commitWaiters.get(targetId) === waiters) this.commitWaiters.delete(targetId); } };
  }

  canReconcileConcurrently(rawViewport, { browserBarVisible, appPanelCssWidth } = {}) {
    try {
      return this.connection?.isOpen() === true
        && this.appliedViewport === appliedReconcileKey(canonicalViewport(rawViewport), browserBarVisible, appPanelCssWidth);
    } catch { return false; }
  }

  async reconcile(rawViewport, { browserBarVisible, appPanelCssWidth, visibleTargetId } = {}) {
    const viewport = canonicalViewport(rawViewport);
    this.canonical = viewport;
    // A changed or failed viewport is never an observational preflight.
    if (this.appliedViewport !== appliedReconcileKey(viewport, browserBarVisible, appPanelCssWidth)) {
      this.appliedViewport = null;
    }
    try {
      await this.applyCanonicalDisplay?.(viewport);
    } catch (error) {
      this.appliedViewport = null;
      throw error;
    }
    // A kernel before the automatic App panel sends no width: App pages then
    // get the whole viewport and no panel is drawn beside them.
    this.appViewport = Number.isSafeInteger(appPanelCssWidth) && appPanelCssWidth > 0
      && appPanelCssWidth < viewport.css_width
      ? { viewport, panelCssWidth: appPanelCssWidth }
      : null;
    const connection = await this.ensureConnection();
    try {
      const { targetInfos = [] } = await connection.send("Target.getTargets");
      const pages = targetInfos.filter(
        (target) => target?.type === "page" && typeof target.targetId === "string",
      );
      const writerTargets = targetInfos.filter(
        (target) => PERSISTENT_COOKIE_WRITER_TARGET_TYPES.has(target?.type)
          && typeof target.targetId === "string",
      );
      const persistentTargetIds = new Set(
        [...pages, ...writerTargets].map((target) => target.targetId),
      );
      for (const targetId of this.sessionsByTarget.keys()) {
        if (!persistentTargetIds.has(targetId)) await this.forgetTarget(targetId);
      }
      // A Tab can close while it is inspected (App Tabs are swept when the
      // browser connection is re-established): it drops out of this
      // reconcile; a live Tab whose session went stale is attached again.
      const appTargets = new Map([...(this.appTabs?.apps?.values() ?? [])].map((app) => [app.targetId, app]));
      const metricsFor = (target) => (appTargets.has(target.targetId) && this.appMetrics(appTargets.get(target.targetId).page))
        || this.layoutMetrics(target.targetId, viewport);
      // Window mode changes the available content area. Measure only after
      // Chromium has applied it, and retire page metrics when that mode changes.
      if (typeof browserBarVisible === "boolean") {
        const before = JSON.stringify([...this.browserBarApplied]);
        await applyBrowserBar(connection, pages, appTargets, browserBarVisible, this.browserBarApplied);
        if (before !== JSON.stringify([...this.browserBarApplied])) this.viewportByTarget.clear();
      }
      const inspected = (await Promise.all(pages.map(async (target) => {
        try {
          return await this.inspectPage(connection, target, metricsFor(target), {visibleTargetId});
        } catch (error) {
          if (this.rendererRecovery.crashed.has(target.targetId)) {
            return this.inspectPage(connection, target, metricsFor(target), {visibleTargetId});
          }
          if (!targetGone(error)) throw error;
          await this.forgetTarget(target.targetId);
          const { targetInfos: now = [] } = await connection.send("Target.getTargets");
          if (!now.some((candidate) => candidate.targetId === target.targetId)) return null;
          try {
            return await this.inspectPage(connection, target, metricsFor(target), {visibleTargetId});
          } catch (again) {
            if (this.rendererRecovery.crashed.has(target.targetId)) {
              return this.inspectPage(connection, target, metricsFor(target), {visibleTargetId});
            }
            // Gone twice: it is closing. The next reconcile sees it if not.
            if (!targetGone(again)) throw again;
            await this.forgetTarget(target.targetId);
            return null;
          }
        }
      }))).filter(Boolean);
      await Promise.all(
        writerTargets.map((target) => this.reconcileWriterTarget(connection, target.targetId)),
      );
      // A home kernel before protocol 379 sends no flag; a 379 worker behind it
      // defaults it to hidden (fullscreen), the new default.
      this.appliedViewport = appliedReconcileKey(viewport, browserBarVisible, appPanelCssWidth);
      const focused = inspected.find((tab) => tab.focused)?.target_id ?? null;
      return {
        browser_generation: this.browserGeneration,
        tabs: inspected.map(({ focused: _focused, ...tab }) => tab),
        focused_target_id: focused,
        viewport,
        event_cursor: this.eventJournal.cursor(),
      };
    } catch (error) {
      if (!connection.isOpen()) this.invalidateConnection(connection);
      throw normalizeControllerError(error);
    }
  }

  async close() {
    this.memory.close();
    const connection = this.invalidateConnection(undefined, { preserveFrameSessions: true });
    await this.frameSessions.close();
    if (connection) await connection.close();
  }

  async withCookieWritersQuiesced(operation) {
    if (typeof operation !== "function" || this.cookieWriterFenceInUse) {
      throw new BrowserControllerError(
        "browser_cookie_writer_fence_busy",
        "browser cookie writer fence is already in use",
      );
    }
    const connection = await this.ensureConnection();
    if (!this.cookieWriterFence) {
      const { targetInfos = [] } = await connection.send("Target.getTargets");
      const pageTargets = targetInfos.filter(
        (target) => target?.type === "page" && typeof target.targetId === "string",
      );
      const pageSessions = await Promise.all(
        pageTargets.map((target) => this.ensureTargetSession(connection, target.targetId)),
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
        waitForNetworkIdle: (sessions) => this.waitForNetworkIdle(sessions),
        waitForWriterTargetsGone: (targetIds) => this.waitForWriterTargetsGone(connection, targetIds),
      });
    }
    const fence = this.cookieWriterFence;
    this.cookieWriterFenceInUse = true;
    let retained = false;
    try {
      return await operation({ retain: () => { retained = true; } });
    } finally {
      this.cookieWriterFenceInUse = false;
      if (!retained && this.cookieWriterFence === fence) {
        this.cookieWriterFence = null;
        await fence.release();
      }
    }
  }

  async waitForNetworkIdle(sessionIds) {
    const deadline = Date.now() + this.requestTimeoutMs;
    while (sessionIds.some((sessionId) => (this.networkRequestsBySession.get(sessionId)?.size ?? 0) > 0)) {
      if (Date.now() >= deadline) {
        throw new BrowserControllerError(
          "browser_cookie_writer_fence_timeout",
          "browser network did not quiesce before cookie import",
        );
      }
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }

  async waitForWriterTargetsGone(connection, targetIds) {
    const pending = new Set(targetIds);
    const deadline = Date.now() + this.requestTimeoutMs;
    while (true) {
      const { targetInfos = [] } = await connection.send("Target.getTargets");
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

  invalidateConnection(expectedConnection, {
    preserveFrameSessions = false,
    excludeConnectionAttempt = null,
  } = {}) {
    if (expectedConnection !== undefined && this.connection !== expectedConnection) return null;
    this.connectionEpoch += 1;
    const connection = this.connection;
    const error = new BrowserControllerError("browser_cdp_disconnected", "browser connection was invalidated");
    const attempt = this.connectionAttempt;
    if (attempt !== excludeConnectionAttempt) {
      this.connectionAttempt = null;
      if (attempt) {
        attempt.invalidated = true;
        attempt.rejectInvalidated(error);
      }
    }
    for (const pending of this.targetSessionAttempts.values()) {
      pending.invalidated = true;
      pending.rejectInvalidated(error);
    }
    this.targetSessionAttempts.clear();
    this.connection = null;
    this.rendererRecovery.close();
    this.appliedViewport = null;
    this.viewportByTarget.clear(); this.contentInsetsByWindow.clear();
    this.focusWorldsByTarget.clear();
    this.unsubscribeFromConnection?.();
    this.unsubscribeFromConnection = null;
    this.sessionsByTarget.clear();
    this.targetsBySession.clear();
    this.targetsByFrame.clear();
    if (!preserveFrameSessions) this.frameSessions.clear();
    this.targetsByDownload.clear();
    this.downloads.clear();
    this.passiveCapture.clear();
    this.downloadCancellationReasons.clear();
    this.downloadDiskCheckPending = false;
    this.downloadDiskCheckRequested = false;
    this.documentIdsByTarget.clear(); this.pageFramesByTarget.clear(); this.suspendedPageFrames.clear();
    this.fillTargets.clear();
    this.snapshotStateByTarget.clear();
    this.dialogDefaults.clear();
    this.networkRequestsBySession.clear();
    this.responseBodies.clear();
    this.cookieWriterFence = null;
    this.cookieWriterFenceInUse = false;
    // A relaunched Chromium numbers its windows from 1 again.
    this.browserBarApplied = new Map();
    return connection;
  }

  async ensureConnection() {
    if (this.connectionAttempt) {
      return this.connectionAttempt.promise;
    }
    if (this.connection?.isOpen()) {
      return this.connection;
    }
    const staleConnection = this.invalidateConnection();
    const attempt = { epoch: this.connectionEpoch, invalidated: false };
    let rejectInvalidated;
    const invalidated = new Promise((_, reject) => {
      rejectInvalidated = reject;
    });
    attempt.rejectInvalidated = rejectInvalidated;
    this.connectionAttempt = attempt;
    const initialization = this.initializeConnection(attempt, staleConnection);
    attempt.promise = Promise.race([initialization, invalidated]).finally(() => {
      if (this.connectionAttempt === attempt) this.connectionAttempt = null;
    });
    return attempt.promise;
  }

  async initializeConnection(attempt, staleConnection) {
    let connection;
    let unsubscribe;
    try {
      if (staleConnection) {
        await staleConnection.close().catch(() => {});
      }
      this.assertConnectionAttemptCurrent(attempt);
      connection = this.connectionFactory
        ? await this.connectionFactory()
        : await connectToBrowser({
            debuggerEndpoint: this.debuggerEndpoint,
            requestTimeoutMs: this.requestTimeoutMs,
            fetchImpl: this.fetchImpl,
            webSocketFactory: this.webSocketFactory,
          });
      this.assertConnectionAttemptCurrent(attempt);
      if (!connection?.isOpen()) {
        throw new BrowserControllerError(
          "browser_cdp_disconnected",
          "browser connection closed during initialization",
        );
      }
      this.connection = connection;
      this.browserGeneration += 1;
      if (typeof connection.subscribe === "function") {
        unsubscribe = connection.subscribe((message) => {
          if (this.connection === connection
              && this.connectionEpoch === attempt.epoch) {
            this.recordConnectionEvent(message);
          }
        });
      }
      this.unsubscribeFromConnection = unsubscribe ?? null;
      this.assertConnectionAttemptCurrent(attempt, connection);
      await connection.send("Target.setDiscoverTargets", { discover: true });
      this.assertConnectionAttemptCurrent(attempt, connection);
      this.memory.watch(connection);
      this.eventJournal.recordCdp(
        { method: "Chariox.browserConnected", params: {} },
        this.eventContext(),
      );
      if (this.onConnected) await this.onConnected(connection);
      this.assertConnectionAttemptCurrent(attempt, connection);
      return connection;
    } catch (error) {
      if (this.unsubscribeFromConnection === unsubscribe) {
        unsubscribe?.();
        this.unsubscribeFromConnection = null;
      }
      if (connection && this.connection === connection && this.connectionEpoch === attempt.epoch) {
        this.invalidateConnection(connection, { excludeConnectionAttempt: attempt });
      }
      if (connection && this.connection !== connection) {
        await connection.close().catch(() => {});
      }
      throw error;
    }
  }

  assertConnectionAttemptCurrent(attempt, connection) {
    if (!this.isConnectionAttemptCurrent(attempt, connection)) {
      throw new BrowserControllerError(
        "browser_cdp_disconnected",
        "browser connection initialization was invalidated",
      );
    }
  }

  isConnectionAttemptCurrent(attempt, connection) {
    return this.connectionAttempt === attempt
      && !attempt.invalidated
      && this.connectionEpoch === attempt.epoch
      && (connection === undefined || this.connection === connection)
      && (connection === undefined || connection.isOpen());
  }

  async configurePageTargetSession(connection, sessionId) {
    const results = await Promise.allSettled([
      ...(this.focusWorldSource ? [connection.send("Page.addScriptToEvaluateOnNewDocument", {
        source: this.focusWorldSource, worldName: FOCUS_WORLD, runImmediately: true,
      }, sessionId)] : []),
      connection.send("Page.enable", {}, sessionId),
      connection.send("Page.setLifecycleEventsEnabled", { enabled: true }, sessionId),
      connection.send("Runtime.enable", {}, sessionId),
      connection.send("Network.enable", responseBufferLimits, sessionId),
      connection.send("Inspector.enable", {}, sessionId),
      this.frameSessions.start(connection, sessionId),
    ]);
    const failed = results.find((result) => result.status === "rejected");
    if (failed) throw failed.reason;
  }

  async configureWriterTargetSession(connection, sessionId) {
    const results = await Promise.allSettled([
      connection.send("Network.enable", responseBufferLimits, sessionId),
      connection.send("Debugger.enable", {}, sessionId),
    ]);
    const failed = results.find((result) => result.status === "rejected");
    if (failed) throw failed.reason;
  }

  async initializeTargetSession(attempt, configure) {
    const { connection, targetId } = attempt;
    let sessionId;
    try {
      const attached = await connection.send("Target.attachToTarget", {
        targetId,
        flatten: true,
      });
      if (typeof attached?.sessionId !== "string" || !attached.sessionId) {
        throw new BrowserControllerError(
          "browser_attach_failed",
          `browser target ${JSON.stringify(targetId)} did not return a session`,
        );
      }
      sessionId = attached.sessionId;
      attempt.sessionId = sessionId;
      this.assertTargetSessionAttemptCurrent(attempt);
      this.sessionsByTarget.set(targetId, sessionId);
      this.targetsBySession.set(sessionId, targetId);
      attempt.published = true;
      await configure(connection, sessionId, targetId);
      this.assertTargetSessionAttemptCurrent(attempt);
      return sessionId;
    } catch (error) {
      const ownsSession = this.targetSessionAttempts.get(targetId) === attempt
        && this.connection === connection && sessionId
        && this.sessionsByTarget.get(targetId) === sessionId
        && this.targetsBySession.get(sessionId) === targetId;
      if (ownsSession) {
        this.sessionsByTarget.delete(targetId);
        this.targetsBySession.delete(sessionId);
        await this.frameSessions.removeTarget(targetId);
      }
      if (sessionId) {
        await connection.send("Target.detachFromTarget", { sessionId }).catch(() => {});
      }
      throw error;
    }
  }

  assertTargetSessionAttemptCurrent(attempt) {
    if (!this.isTargetSessionAttemptCurrent(attempt)) {
      throw new BrowserControllerError(
        "browser_cdp_disconnected",
        `browser target ${JSON.stringify(attempt.targetId)} session attach was invalidated`,
      );
    }
  }

  isTargetSessionAttemptCurrent(attempt) {
    return this.targetSessionAttempts.get(attempt.targetId) === attempt
      && !attempt.invalidated
      && !attempt.detachedSessionIds.has(attempt.sessionId)
      && this.connectionEpoch === attempt.epoch
      && this.connection === attempt.connection
      && attempt.connection.isOpen()
      && (!attempt.published
        || this.sessionsByTarget.get(attempt.targetId) === attempt.sessionId);
  }

  beginTargetSessionAttempt(connection, targetId, configure) {
    const attempt = {
      connection,
      epoch: this.connectionEpoch,
      targetId,
      detachedSessionIds: new Set(),
      published: false,
      invalidated: false,
    };
    let rejectInvalidated;
    const invalidated = new Promise((_, reject) => {
      rejectInvalidated = reject;
    });
    attempt.rejectInvalidated = rejectInvalidated;
    this.targetSessionAttempts.set(targetId, attempt);
    const initialization = this.initializeTargetSession(attempt, configure);
    attempt.promise = Promise.race([initialization, invalidated]).finally(() => {
      if (this.targetSessionAttempts.get(targetId) === attempt) {
        this.targetSessionAttempts.delete(targetId);
      }
    });
    return attempt.promise;
  }

  ensureTargetSessionFor(connection, targetId, configure) {
    if (connection !== this.connection || !connection.isOpen()) {
      return Promise.reject(new BrowserControllerError(
        "browser_cdp_disconnected",
        `browser target ${JSON.stringify(targetId)} cannot attach on a stale connection`,
      ));
    }
    const pending = this.targetSessionAttempts.get(targetId);
    if (pending && pending.connection === connection && pending.epoch === this.connectionEpoch
        && !pending.invalidated) {
      return pending.promise;
    }
    const sessionId = this.sessionsByTarget.get(targetId);
    if (sessionId) {
      return Promise.resolve(sessionId);
    }
    return this.beginTargetSessionAttempt(connection, targetId, configure);
  }

  async forgetTarget(targetId) {
    this.rendererRecovery.forget(targetId);
    this.invalidateTargetSession(targetId);
    const sessionId = this.sessionsByTarget.get(targetId);
    this.targetsBySession.delete(sessionId);
    this.networkRequestsBySession.delete(sessionId);
    this.responseBodies.drop(sessionId);
    this.sessionsByTarget.delete(targetId);
    this.viewportByTarget.delete(targetId); this.layoutByTarget.delete(targetId); this.layoutQueue.delete(targetId);
    this.documentIdsByTarget.delete(targetId); this.pageFramesByTarget.delete(targetId); this.suspendedPageFrames.delete(targetId);
    this.focusWorldsByTarget.delete(targetId);
    this.snapshotStateByTarget.delete(targetId);
    this.dialogDefaults.delete(targetId);
    this.targetsByFrame.removeTarget(targetId);
    await this.frameSessions.removeTarget(targetId);
  }

  // App pages are narrower than the canonical viewport: the trusted
  // conversation panel takes the right of the desktop. Null without a panel.
  appMetrics(page) {
    if (!this.appViewport) return null;
    const { viewport, panelCssWidth } = this.appViewport;
    // The kernel sizes each App page beside its panel; before it does, the
    // default panel takes the right of the page.
    const size = validPage(page, viewport) ?? { width: viewport.css_width - panelCssWidth, height: viewport.css_height };
    return { ...deviceMetricsFor(viewport, !!this.applyCanonicalDisplay), ...size };
  }


  // The person's display sets the layout size of the Tab they view (a phone is narrower than the canonical desktop).
  layoutViewport(targetId, viewport) {
    const layout = this.viewportByTarget.get(targetId)?.metrics ?? this.layoutByTarget.get(targetId);
    return layout ? { ...viewport, css_width: layout.width, css_height: layout.height } : viewport;
  }

  layoutMetrics(targetId, viewport) {
    return { ...deviceMetricsFor(viewport, !!this.applyCanonicalDisplay), ...(this.layoutByTarget.get(targetId) ?? {}) };
  }

  // Null restores the canonical layout. App pages keep their panel-sized layout.
  // Applications run one at a time per Tab and read the newest layout, so a restore that overtakes a slower override still wins.
  setLayout(targetId, layout) {
    if (layout) this.layoutByTarget.set(targetId, { width: layout.width, height: layout.height });
    else this.layoutByTarget.delete(targetId);
    return this.queueLayout(targetId, () => this.applyLayout(targetId));
  }

  queueLayout(targetId, operation) {
    const run = (this.layoutQueue.get(targetId) ?? Promise.resolve()).catch(() => {}).then(operation);
    this.layoutQueue.set(targetId, run);
    return run;
  }

  // MP-08/MP-10/MP-11: clearing native emulation also retires its configured
  // key. A new viewer restores its viewport after this release, in order.
  clearViewport(targetId) {
    return this.queueLayout(targetId, async () => {
      const connection = this.connection;
      if (!connection) return;
      const sessionId = await this.ensureTargetSession(connection, targetId);
      this.viewportByTarget.delete(targetId);
      await connection.send('Emulation.clearDeviceMetricsOverride', {}, sessionId);
    });
  }

  async applyLayout(targetId) {
    const connection = this.connection;
    if (!this.canonical || !connection) return;
    if ([...(this.appTabs?.apps?.values() ?? [])].some((app) => app.targetId === targetId)) return;
    const sessionId = await this.ensureTargetSession(connection, targetId);
    const metrics = this.layoutMetrics(targetId, this.canonical);
    const key = JSON.stringify(metrics);
    if (this.viewportByTarget.get(targetId)?.key === key) return;
    const applied = await this.setPageMetrics(connection, sessionId, metrics);
    this.viewportByTarget.set(targetId, { key, metrics: applied });
  }

  async setPageMetrics(connection, sessionId, metrics) {
    if (this.applyCanonicalDisplay) {
      let { windowId, bounds } = await connection.send("Browser.getWindowForTarget", {}, sessionId);
      const desktop = this.canonical;
      const deadline = performance.now() + 2000;
      const maximized = () => ["maximized", "fullscreen"].includes(bounds?.windowState);
      while (desktop && maximized() && (bounds.width !== desktop.desktop_pixel_width || bounds.height !== desktop.desktop_pixel_height)) {
        if (performance.now() >= deadline) throw new BrowserControllerError("viewport_apply_failed", "Chromium window resize is still pending; retry the browser viewport");
        await new Promise(resolve => setTimeout(resolve, 25));
        ({ windowId, bounds } = await connection.send("Browser.getWindowForTarget", {}, sessionId));
      }
      if (![bounds?.left, bounds?.top, bounds?.width, bounds?.height].every(Number.isFinite)) {
        throw new BrowserControllerError("viewport_apply_failed", "Chromium window bounds are unavailable; restore its window and retry");
      }
      let insets = this.contentInsetsByWindow.get(windowId);
      if (!insets || insets.state !== bounds.windowState) {
        // The first native measurement owns the chrome insets. Emulated page
        // sizes must never become the next native measurement after a resize.
        await connection.send("Emulation.setDeviceMetricsOverride", {
          width: 0, height: 0, deviceScaleFactor: 0, scale: 1, mobile: false, dontSetVisibleSize: true,
        }, sessionId);
        await connection.send("Emulation.clearDeviceMetricsOverride", {}, sessionId);
        // innerWidth/innerHeight are CSS pixels and follow the user's page zoom;
        // window bounds are not. Convert to window units, rounding down by the
        // half CSS pixel the page may have rounded up so content never overshoots.
        const { cssVisualViewport } = await connection.send("Page.getLayoutMetrics", {}, sessionId);
        const zoom = cssVisualViewport?.zoom;
        if (!Number.isFinite(zoom) || zoom <= 0) throw new BrowserControllerError("viewport_apply_failed", "Chromium page zoom is unavailable; retry the browser viewport");
        const slack = zoom === 1 ? 0 : Math.ceil(zoom);
        const windowUnits = (cssPixels) => zoom === 1 ? cssPixels : Math.floor((cssPixels - 0.5) * zoom);
        const tree = await connection.send("Page.getFrameTree", {}, sessionId);
        const { executionContextId } = await connection.send("Page.createIsolatedWorld", {
          frameId: tree.frameTree.frame.id, worldName: "chariox-content-viewport",
        }, sessionId);
        let content;
        for (;;) {
          const { result } = await connection.send("Runtime.evaluate", {
            expression: "({ width: innerWidth, height: innerHeight })", contextId: executionContextId, returnByValue: true,
          }, sessionId);
          const measured = result?.value;
          content = { width: windowUnits(measured?.width), height: windowUnits(measured?.height) };
          if (!maximized() || Math.abs(content.width - bounds.width) <= slack) break;
          if (performance.now() >= deadline) throw new BrowserControllerError("viewport_apply_failed", "Chromium content resize is still pending; retry the browser viewport");
          await new Promise(resolve => setTimeout(resolve, 25));
          ({ bounds } = await connection.send("Browser.getWindowForTarget", {}, sessionId));
        }
        if (![content.width, content.height].every(n => Number.isFinite(n) && n > 0)
            || content.width > bounds.width + slack || content.height > bounds.height + slack) {
          throw new BrowserControllerError("viewport_apply_failed", "Chromium content area is unavailable; retry the browser viewport");
        }
        // A settled maximized window has no side chrome; the rounding is not an inset.
        if (maximized()) content.width = bounds.width;
        content.width = Math.min(content.width, bounds.width);
        content.height = Math.min(content.height, bounds.height);
        insets = { state: bounds.windowState, width: bounds.width - content.width, height: bounds.height - content.height };
        this.contentInsetsByWindow.set(windowId, insets);
      }
      const content = { width: bounds.width - insets.width, height: bounds.height - insets.height };
      const visibleWidth = desktop ? Math.min(content.width,
        desktop.desktop_pixel_width - Math.max(0, bounds.left) - (bounds.width - content.width)) : content.width;
      const visibleHeight = desktop ? Math.min(content.height,
        desktop.desktop_pixel_height - Math.max(0, bounds.top) - (bounds.height - content.height)) : content.height;
      if (![visibleWidth, visibleHeight].every(n => Number.isFinite(n) && n >= metrics.deviceScaleFactor)) {
        throw new BrowserControllerError("viewport_apply_failed", "Chromium content area is outside the desktop; restore its window and retry");
      }
      metrics = { ...metrics,
        width: Math.min(metrics.width, Math.floor(visibleWidth / metrics.deviceScaleFactor)),
        height: Math.min(metrics.height, Math.floor(visibleHeight / metrics.deviceScaleFactor)),
      };
    }
    await connection.send("Emulation.setDeviceMetricsOverride", metrics, sessionId);
    // Headed emulation leaves the visible area at CSS size even when the
    // image scale is larger. Keep it large enough for the physical raster.
    if (this.applyCanonicalDisplay) {
      await connection.send("Emulation.setVisibleSize", {
        width: Math.round(metrics.width * metrics.deviceScaleFactor),
        height: Math.round(metrics.height * metrics.deviceScaleFactor),
      }, sessionId);
    }
    return metrics;
  }

  async inspectPage(connection, target, metrics, {visibleTargetId} = {}) {
    if (this.rendererRecovery.crashed.has(target.targetId)) {
      // Browser-owned target metadata survives renderer death. Keep its last
      // document binding so inventory and close never wait on the dead page.
      return { target_id: target.targetId,
        ...(typeof target.openerId === "string" ? { opener_target_id: target.openerId } : {}),
        document_id: this.documentIdsByTarget.get(target.targetId),
        url: typeof target.url === "string" ? target.url : "",
        title: typeof target.title === "string" ? target.title : "", focused: false };
    }
    const sessionId = await this.ensureTargetSession(connection, target.targetId);
    // Metrics differ per target (App pages lay out beside their panel).
    // Reapply only when this target's metrics changed.
    const metricsKey = JSON.stringify(metrics);
    const configured = this.viewportByTarget.get(target.targetId)?.key === metricsKey;
    if (!configured) {
      const applied = await this.setPageMetrics(connection, sessionId, metrics);
      this.viewportByTarget.set(target.targetId, { key: metricsKey, metrics: applied });
    }
    const metadata = nativeStatePage(this, connection, sessionId, target, visibleTargetId, configured);
    if (metadata) return metadata;
    const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
    const frame = frameTree?.frameTree?.frame;
    const documentId = frame?.loaderId;
    if (typeof documentId !== "string" || !documentId) {
      throw new BrowserControllerError(
        "browser_document_identity_missing",
        `browser target ${JSON.stringify(target.targetId)} has no top-level loader identity`,
      );
    }
    const observation = pageObservationFence(this, connection, sessionId, target.targetId, frame);
    // MP-08/MP-10/MP-11: overlap independent observations, retaining both
    // fences and draining their work before a failed observation returns.
    const [focus, frames] = await Promise.allSettled([
      this.readFocus(connection, sessionId, target.targetId, frame),
      registerBrowserFrameTargets(connection, sessionId, target.targetId, documentId, this.targetsByFrame),
    ]);
    if (focus.status === "rejected") throw focus.reason;
    if (frames.status === "rejected") throw frames.reason;
    observation.assertCurrent();
    this.documentIdsByTarget.set(target.targetId, documentId);
    if (!observation.owner) this.pageFramesByTarget.set(target.targetId, { connection, sessionId, frame });
    return {
      target_id: target.targetId,
      ...(typeof target.openerId === "string" ? { opener_target_id: target.openerId } : {}),
      document_id: documentId,
      url: typeof target.url === "string" ? target.url : "",
      title: typeof target.title === "string" ? target.title : "",
      focused: focus.value === true,
    };
  }

  // One isolated world per document: polls reuse it, a new document gets a new one.
  // Input capture emulates focus while it runs: report the physical visibility
  // it captured before enabling emulation instead.
  ensureFocusWorld(connection, sessionId, targetId, frame, options) {
    return ensureFocusWorld(this, connection, sessionId, targetId, frame, options);
  }
  async observeNote(request) { return observeBrowserNote(this, request); }

  async readFocus(connection, sessionId, targetId, frame) {
    const captured = this.inputCapture.visibilityBySession.get(sessionId);
    if (captured) return captured.visible;
    const world = await this.ensureFocusWorld(connection, sessionId, targetId, frame);
    let focus;
    try {
      focus = await connection.send(
        "Runtime.evaluate",
        {
          expression: "document.visibilityState === 'visible'",
          contextId: world.contextId,
          returnByValue: true,
          awaitPromise: false,
        },
        sessionId,
      );
    } catch (error) {
      this.focusWorldsByTarget.delete(targetId);
      // MP-08/MP-10: a navigation (e.g. a native viewer click) destroyed the
      // world between the frame read and this poll; callers retry stale reads.
      if (/Cannot find context with specified id|Execution context was destroyed/.test(error?.message ?? "")) {
        throw new BrowserSnapshotError("stale_document_reference", "browser document changed during focus read");
      }
      throw error;
    }
    // This read may have begun just before input enabled emulation.
    return this.inputCapture.visibilityBySession.get(sessionId)?.visible ?? focus?.result?.value === true;
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
    const { targetInfos = [] } = await connection.send("Target.getTargets");
    const target = targetInfos.find(
      (candidate) => candidate?.type === "page" && candidate.targetId === targetId,
    );
    if (!target) {
      throw new BrowserControllerError(
        "browser_target_not_found",
        `browser target ${JSON.stringify(targetId)} is not available`,
      );
    }
    if (this.documentIdsByTarget.get(targetId) !== documentId) {
      throw new BrowserControllerError(
        "stale_document_reference",
        `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
      );
    }
    assertNotCancelled(signal);
    if (action === "activate") {
      const sessionId = await this.ensureTargetSession(connection, targetId);
      assertNotCancelled(signal);
      await connection.send("Page.bringToFront", {}, sessionId);
    } else {
      const result = await this.closePageTarget(connection, targetId, { signal });
      if (result?.success !== true) {
        throw new BrowserControllerError(
          "browser_tab_close_failed",
          `browser target ${JSON.stringify(targetId)} did not close`,
        );
      }
      await this.waitForTargetClosure(connection, targetId);
    }
    return {
      browser_generation: this.browserGeneration,
      target_id: targetId,
      document_id: documentId,
      action,
    };
  }

  async closePageTarget(connection, targetId, { signal } = {}) {
    const previous = this.pageCloseQueue;
    let release;
    this.pageCloseQueue = new Promise((resolve) => { release = resolve; });
    await previous;
    try {
      assertNotCancelled(signal);
      const { targetInfos = [] } = await connection.send("Target.getTargets");
      const pages = targetInfos.filter((target) => target?.type === "page");
      assertNotCancelled(signal);
      if (pages.some((target) => target.targetId === targetId) && pages.length === 1) {
        // Headed Chromium exits with its last window. Create the replacement
        // before closing; if creation fails, leave the original page open.
        await connection.send("Target.createTarget", { url: "about:blank" });
      }
      assertNotCancelled(signal);
      return await connection.send("Target.closeTarget", { targetId });
    } finally {
      release();
    }
  }

  async waitForTargetClosure(connection, targetId) {
    const deadline = Date.now() + this.requestTimeoutMs;
    while (true) {
      const { targetInfos = [] } = await connection.send("Target.getTargets");
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
    await this.memory.requireHeadroom();
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    if (this.rendererRecovery.crashed.has(targetId)) {
      throw new BrowserControllerError('browser_renderer_crashed', 'Browser tab crashed; recovery waits for available memory. Close the tab to stop recovery.');
    }
    const documentId = requiredIdentity(
      rawRequest?.document_id,
      "document_id",
    );
    const connection = await this.ensureConnection();
    const { targetInfos = [] } = await connection.send("Target.getTargets");
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
    const sessionId = await this.ensureTargetSession(connection, targetId);
    const previous = this.snapshotStateByTarget.get(targetId);
    const snapshotRevision =
      previous?.documentId === documentId ? previous.revision + 1 : 1;
    this.snapshotStateByTarget.set(targetId, {
      documentId,
      revision: snapshotRevision,
    });
    try {
      return await captureBrowserFrames({
        connection,
        sessionId,
        targetId,
        documentId,
        browserGeneration: this.browserGeneration,
        snapshotRevision,
        protectedValues: this.protectedValues,
      });
    } catch (error) {
      if (this.snapshotStateByTarget.get(targetId)?.revision === snapshotRevision) {
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
    // Register before focus/input handlers run, including a failed or partial fill.
    if (rawRequest?.action?.kind === "fill" && rawRequest.action.expected_document_url) {
      this.protectedValues.add(rawRequest.action.text);
    }
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const connection = await this.ensureConnection();
    const { targetInfos = [] } = await connection.send("Target.getTargets");
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
    const sessionId = await this.ensureTargetSession(connection, targetId);
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
        withInput: operation => this.inputCapture.run(connection, sessionId, operation),
      }, async options => {
        if (rawRequest?.action?.kind === 'fill' && rawRequest.action.expected_document_url) {
          const target = await recordBrowserFill(connection, {...options, browserGeneration:this.browserGeneration, trackingDocumentId:documentId, trackingNodeRef:rawRequest.node_ref}, rawRequest.action.text, ++this.fillRevision);
          this.fillTargets.set(`${targetId}:${rawRequest.node_ref}`, target);
          // MP-11: a new protected target ends every display lease before the value is typed.
          retireNativeStates();
          // MP-11: bind the completed value before releasing in-flight coverage.
          // A failed action still fingerprints whatever it inserted; an
          // unprovable completion keeps the fill pending (covered).
          let result;
          try { result = await performBrowserAction(options); }
          catch (error) { await finishBrowserFill(connection, target).catch(() => {}); throw error; }
          await finishBrowserFill(connection, target);
          return result;
        }
        return performBrowserAction(options);
      });
      return {
        browser_generation: this.browserGeneration,
        ...result,
      };
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async navigate(rawRequest, { signal } = {}) {
    assertNotCancelled(signal);
    await this.memory.requireHeadroom();
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const { connection, sessionId } = await this.resolvePageTarget(targetId);
    try {
      const result = await navigateBrowser({
        connection,
        sessionId,
        targetId,
        documentId,
        url: rawRequest?.url,
        signal,
      });
      this.documentIdsByTarget.set(targetId, result.document_id);
      this.snapshotStateByTarget.delete(targetId);
      return {
        browser_generation: this.browserGeneration,
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
    const { connection, sessionId } = await this.resolvePageTarget(targetId);
    try {
      const result = await navigateBrowserHistory({
        connection,
        sessionId,
        targetId,
        documentId,
        action: rawRequest?.action,
        signal,
      });
      this.documentIdsByTarget.set(targetId, result.document_id);
      this.snapshotStateByTarget.delete(targetId);
      return {
        browser_generation: this.browserGeneration,
        ...result,
      };
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async wait(rawRequest) {
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const { connection, sessionId } = await this.resolvePageTarget(targetId);
    try {
      return {
        browser_generation: this.browserGeneration,
        ...(await waitForBrowserState({
          connection,
          sessionId,
          targetId,
          documentId,
          kind: rawRequest?.kind,
          selector: rawRequest?.selector,
          timeoutMs: rawRequest?.timeout_ms,
        })),
      };
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
    const { targetInfos = [] } = await connection.send("Target.getTargets");
    const target = targetInfos.find(
      (candidate) => candidate?.type === "page" && candidate.targetId === targetId,
    );
    if (!target) {
      throw new BrowserControllerError(
        "browser_target_not_found",
        `browser target ${JSON.stringify(targetId)} is not available`,
      );
    }
    const sessionId = await this.ensureTargetSession(connection, targetId);
    if (this.documentIdsByTarget.get(targetId) !== documentId) {
      throw new BrowserControllerError(
        "stale_document_reference",
        `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
      );
    }
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
    if (rawRequest.dialog_id != null && !this.dialogDefaults.claim(targetId, rawRequest.dialog_id, documentId)) {
      throw new BrowserControllerError("browser_dialog_invalid", "browser dialog is no longer pending; read the current browser state");
    }
    try { await connection.send(
      "Page.handleJavaScriptDialog",
      {
        accept: action === "accept",
        ...(action === "accept" && answer != null ? { promptText: answer } : {}),
      },
      sessionId,
    ); } finally { this.dialogDefaults.release(rawRequest.dialog_id); }
    // Do not retire a replacement dialog that opened while CDP acknowledged this answer.
    if (rawRequest.dialog_id != null && this.dialogDefaults.current(targetId, rawRequest.dialog_id, documentId)) {
      this.dialogDefaults.delete(targetId);
    }
    return {
      browser_generation: this.browserGeneration,
      target_id: targetId,
      document_id: documentId,
      action,
    };
  }

  async configureDownloads(rawRequest, { signal } = {}) {
    assertNotCancelled(signal);
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const { connection, sessionId } = await this.resolvePageTarget(targetId);
    try {
      return {
        browser_generation: this.browserGeneration,
        ...await configureBrowserDownloads({
          connection,
          sessionId,
          targetId,
          documentId,
          downloadDirectory: this.downloadDirectory,
          minimumFreeBytes: this.minimumDownloadFreeBytes,
          fileSystem: this.fileSystem,
          signal,
        }),
      };
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async cancelDownload(rawRequest) {
    const connection = await this.ensureConnection();
    try {
      return await cancelBrowserDownload({
        connection,
        browserGeneration: this.browserGeneration,
        requestedBrowserGeneration: rawRequest?.browser_generation,
        guid: rawRequest?.guid,
        targetsByDownload: this.targetsByDownload,
      });
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  async uploadFiles(rawRequest, { signal } = {}) {
    assertNotCancelled(signal);
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const { connection, sessionId } = await this.resolvePageTarget(targetId);
    try {
      const upload = async (filePaths, uploadRoots) => ({
        browser_generation: this.browserGeneration,
        ...await withBrowserActionFrame({
          connection,
          sessionId,
          targetId,
          documentId,
          nodeRef: rawRequest?.node_ref,
          filePaths,
          uploadRoots,
          fileSystem: this.fileSystem,
          stageUploads: this.stageUploads,
          signal,
        }, uploadBrowserFiles),
      });
      return rawRequest?.artifact_files
        ? await withUploadArtifacts(rawRequest.artifact_files, signal, upload)
        : await upload(rawRequest?.file_paths, this.uploadRoots);
    } catch (error) {
      throw normalizeControllerError(error);
    }
  }

  // MP-08/MP-10/MP-11: document/viewport fences around same-tab bytes.
  async captureArtifact(request) {
    await this.memory.requireHeadroom();
    try {
      const { connection, sessionId } = await this.resolvePageTarget(requiredIdentity(request?.target_id, "target_id"));
      const targetId = request.target_id;
      const documentId = requiredIdentity(request.document_id, "document_id");
      const generation = this.browserGeneration;
      if (request.browser_generation !== generation) throw new BrowserControllerError("stale_browser_generation", "Browser artifact needs the observed browser generation");
      const canonical = canonicalViewport(request.viewport);
      const viewport = this.layoutViewport(targetId, canonical);
      const metricsKey = JSON.stringify(this.layoutMetrics(targetId, canonical));
      const assertBound = async () => {
        const tree = await connection.send("Page.getFrameTree", {}, sessionId);
        if (connection !== this.connection || generation !== this.browserGeneration || tree?.frameTree?.frame?.loaderId !== documentId)
          throw new BrowserControllerError("stale_document_reference", "Browser artifact document changed");
        if (this.viewportByTarget.get(targetId)?.key !== metricsKey)
          throw new BrowserControllerError("browser_viewport_invalid", "Browser artifact needs the canonical observed viewport");
      };
      await assertBound();
      const observeGeometry = async () => {
        const { cssVisualViewport: visual } = await connection.send("Page.getLayoutMetrics", {}, sessionId);
        const fields = ["pageX", "pageY", "clientWidth", "clientHeight", "scale"];
        if (!visual || fields.some(key => !Number.isFinite(visual[key])) || visual.scale !== 1)
          throw new BrowserControllerError("browser_viewport_invalid", "Browser image requires observed unzoomed visual viewport geometry");
        return Object.fromEntries(fields.map(key => [key, visual[key]]));
      };
      const geometry = await observeGeometry();
      let artifact;
      let redaction = "metadata_only";
      if (request.kind === "identity") {
        artifact = artifactBytes(Buffer.from(JSON.stringify(geometry)), "browser-identity.json", "application/json");
      } else if (request.kind === "image") {
        const captured = await captureProtectedBrowserImage({ connection, sessionId, targetId, documentId, viewport, protectedValues: this.protectedValues, fillTargets:[...this.fillTargets.values()] });
        artifact = artifactBytes(captured.bytes, "browser-tab.png", "image/png");
        redaction = captured.redaction;
      } else if (request.kind === "network") {
        const captured = this.passiveCapture.capture(targetId, documentId);
        const redacted = redactObservation(JSON.parse(Buffer.from(captured.data_base64, "base64")), this.protectedValues);
        artifact = artifactBytes(Buffer.from(JSON.stringify(redacted)), captured.display_name, captured.mime_type);
      } else if (request.kind === "download") {
        if (this.protectedValues.size) throw new BrowserControllerError("browser_observation_redacted", "protected Room download bytes are withheld");
        artifact = await readCompletedDownload({ directory: this.downloadDirectory, downloads: this.downloads,
          guid: request.guid, targetId, documentId, browserGeneration: generation });
      } else throw new BrowserControllerError("browser_artifact_invalid", "unknown Browser artifact kind");
      await assertBound();
      if (JSON.stringify(geometry) !== JSON.stringify(await observeGeometry()))
        throw new BrowserControllerError("browser_viewport_invalid", "Browser viewport moved during capture");
      return { ...artifact, geometry, kind: request.kind, guid: request.guid ?? null, target_id: targetId,
        document_id: documentId, browser_generation: generation, viewport: request.viewport, redaction,
        browser_id: `browser-${createHash("sha256").update(connection.browserInstanceId).digest("hex")}` };
    } catch (error) {
      if (error instanceof BrowserControllerError || error instanceof BrowserArtifactError || error instanceof BrowserSnapshotError) throw normalizeControllerError(error);
      throw new BrowserControllerError("browser_artifact_unavailable", "Browser artifact bytes are unavailable");
    }
  }

  async setPermission(rawRequest, { signal } = {}) {
    assertNotCancelled(signal);
    const targetId = requiredIdentity(rawRequest?.target_id, "target_id");
    const documentId = requiredIdentity(rawRequest?.document_id, "document_id");
    const { connection, sessionId, target } = await this.resolvePageTarget(targetId);
    try {
      return {
        browser_generation: this.browserGeneration,
        ...await setBrowserPermission({
          connection,
          sessionId,
          targetId,
          documentId,
          targetUrl: target.url,
          permission: rawRequest?.permission,
          setting: rawRequest?.setting,
          signal,
        }),
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

  // MP-11: region protection inspects isolated frames through these sessions.
  frameSession(frameId, connection) { return this.frameSessions.sessionFor(frameId, connection); }

  // MP-08/MP-10/MP-11: navigation events are ordered with CDP replies. Reuse
  // only this live connection/session/loader; a cold or retired binding queries.
  currentPageFrame(targetId) {
    const entry = this.pageFramesByTarget.get(targetId);
    return entry && entry.connection === this.connection && entry.connection?.isOpen?.() === true
      && entry.sessionId === this.sessionsByTarget.get(targetId)
      && entry.frame.loaderId === this.documentIdsByTarget.get(targetId) ? entry : null;
  }
  // MP-08/MP-10/MP-11 (P1): the committed document of this target on the live connection/session. Unlike
  // currentPageFrame it stays while a navigation is under way: the page shows that document until the next commit.
  committedPageDocument(targetId) {
    const sessionId = this.sessionsByTarget.get(targetId), documentId = this.documentIdsByTarget.get(targetId);
    return this.connection?.isOpen?.() === true && sessionId && documentId ? { connection: this.connection, sessionId, document_id: documentId } : null;
  }
  async resolvePageTarget(targetId) {
    if (this.rendererRecovery.crashed.has(targetId)) {
      throw new BrowserControllerError('browser_renderer_crashed', 'Browser tab crashed; recovery waits for available memory. Close the tab to stop recovery.');
    }
    const connection = await this.ensureConnection();
    const { targetInfos = [] } = await connection.send("Target.getTargets");
    const target = targetInfos.find(
      (candidate) => candidate?.type === "page" && candidate.targetId === targetId,
    );
    if (!target) {
      throw new BrowserControllerError(
        "browser_target_not_found",
        `browser target ${JSON.stringify(targetId)} is not available`,
      );
    }
    return {
      connection,
      target,
      sessionId: await this.ensureTargetSession(connection, targetId),
    };
  }

  async ensureTargetSession(connection, targetId) {
    return this.ensureTargetSessionFor(connection, targetId, this.configurePageTargetSession.bind(this));
  }

  // MP-08/MP-10/MP-11: short-lived workers can exit after Target.getTargets.
  // This observation never substitutes for the cookie import's writer fence.
  async reconcileWriterTarget(connection, targetId) {
    for (let attempt = 0; attempt < 2; attempt += 1) {
      try { return await this.ensureWriterTargetSession(connection, targetId); }
      catch (error) {
        if (!targetGone(error)) throw error;
        await this.forgetTarget(targetId);
        const { targetInfos = [] } = await connection.send("Target.getTargets");
        if (!targetInfos.some(target => target.targetId === targetId)) return null;
        if (attempt === 1) throw error;
      }
    }
  }

  async ensureWriterTargetSession(connection, targetId) {
    return this.ensureTargetSessionFor(connection, targetId, this.configureWriterTargetSession.bind(this));
  }

  invalidateTargetSession(targetId, sessionId) {
    const pending = this.targetSessionAttempts.get(targetId);
    // Until attach replies, this detach may belong to a retired attempt. Match
    // it against the returned session before rejecting the current callers.
    if (pending && sessionId !== undefined && pending.sessionId === undefined) {
      pending.detachedSessionIds.add(sessionId);
      return;
    }
    if (pending && (sessionId === undefined || pending.sessionId === sessionId)) {
      pending.invalidated = true;
      this.targetSessionAttempts.delete(targetId);
      pending.rejectInvalidated(new BrowserControllerError(
        "browser_cdp_disconnected", "Target closed or detached during session initialization",
      ));
    }
  }

  recordConnectionEvent(message) {
    this.rendererRecovery.observe(message);
    this.responseBodies.observe(message);
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
    const observedTarget = this.targetsBySession.get(message?.sessionId);
    this.passiveCapture.record(redactObservation(message, this.protectedValues), {
      targetId: observedTarget, documentId: this.documentIdsByTarget.get(observedTarget),
    });
    if (this.frameSessions.observe(message, this.connection)) return;
    const dialogTargetId = this.targetsBySession.get(message?.sessionId) ?? message?.params?.targetId;
    this.dialogDefaults.observe(redactObservation(message, this.protectedValues), dialogTargetId, this.documentIdsByTarget.get(dialogTargetId));
    if (message?.method === "Target.detachedFromTarget") {
      const sessionId = message.params?.sessionId ?? message.sessionId;
      const targetId = this.targetsBySession.get(sessionId) ?? message.params?.targetId;
      if (typeof sessionId === "string") {
        this.targetsBySession.delete(sessionId);
        this.networkRequestsBySession.delete(sessionId);
        this.responseBodies.drop(sessionId);
      }
      if (typeof targetId === "string") this.invalidateTargetSession(targetId, sessionId);
      // A late detach for an older session can arrive after this target has
      // already been attached again. Retire target-wide state only when the
      // detached session is still the mapped owner (or no owner remains).
      if (typeof targetId === "string" &&
          (!this.sessionsByTarget.has(targetId) || this.sessionsByTarget.get(targetId) === sessionId)) {
        this.appliedViewport = null;
        this.sessionsByTarget.delete(targetId);
        this.viewportByTarget.delete(targetId);
        this.focusWorldsByTarget.delete(targetId);
        this.dialogDefaults.delete(targetId);
        this.targetsByFrame.removeTarget(targetId);
        void this.frameSessions.removeTarget(targetId);
      }
    }
    this.targetsByFrame.record(message, this.targetsBySession.get(message?.sessionId));
    if (this.visitedLinks && typeof message?.method === "string" && message.method.startsWith("Page.")) {
      const targetId = this.targetsBySession.get(message.sessionId), shown = (this.pageFramesByTarget.get(targetId) ?? this.suspendedPageFrames.get(targetId))?.frame;
      if (targetId) this.visitedLinks.observe(targetId, message, shown?.url);
    }
    // Navigation start, not loading start: fragment and pushState navigations also start loading but keep the document.
    if (message?.method === "Page.frameStartedNavigating") {
      const targetId = this.targetsBySession.get(message.sessionId), entry = this.pageFramesByTarget.get(targetId);
      if (entry?.frame.id === message.params?.frameId) { this.pageFramesByTarget.delete(targetId); this.suspendedPageFrames.set(targetId, entry); }
    }
    if (message?.method === "Page.navigatedWithinDocument") {
      // A history traversal inside the document starts navigating but never commits one: the record it retired is still current.
      const targetId = this.targetsBySession.get(message.sessionId), entry = this.suspendedPageFrames.get(targetId);
      if (entry?.frame.id === message.params?.frameId) { this.suspendedPageFrames.delete(targetId); this.pageFramesByTarget.set(targetId, entry); }
    }
    if (message?.method === "Page.frameNavigated" && !message.params?.frame?.parentId) {
      const targetId = this.targetsBySession.get(message.sessionId);
      const documentId = message.params?.frame?.loaderId;
      if (targetId && typeof documentId === "string" && documentId) {
        this.documentIdsByTarget.set(targetId, documentId);
        this.suspendedPageFrames.delete(targetId);
        const waiters = this.commitWaiters.get(targetId);
        if (waiters) { this.commitWaiters.delete(targetId); for (const wake of waiters) wake(); }
        const navigatedAt = performance.timeOrigin + performance.now();
        this.pageFramesByTarget.set(targetId, { connection: this.connection, sessionId: message.sessionId, frame: message.params.frame, navigatedAt });
        this.pageTiming?.("page_commit", navigatedAt, navigatedAt);
        for(const [key,target] of this.fillTargets)if(target.target_id===targetId&&target.document_id!==documentId)this.fillTargets.delete(key);
      }
    }
    if (message?.method === "Page.loadEventFired") {
      const committed = this.pageFramesByTarget.get(this.targetsBySession.get(message.sessionId))?.navigatedAt;
      if (committed) this.pageTiming?.("page_load", committed, performance.timeOrigin + performance.now());
    }
    recordFocusWorld(this,message);
    if (message?.method === "Browser.downloadWillBegin") {
      const targetId = this.targetsByFrame.get(message.params?.frameId);
      const guid = message.params?.guid;
      if (typeof guid === "string" && guid) {
        this.targetsByDownload.set(guid, targetId ?? null);
        if (this.downloads.size >= 200) this.downloads.delete(this.downloads.keys().next().value);
        this.downloads.set(guid, { targetId, documentId: this.documentIdsByTarget.get(targetId),
          browserGeneration: this.browserGeneration, filename: message.params?.suggestedFilename, state: "inProgress" });
        this.scheduleDownloadDiskCheck();
      }
    }
    if (
      message?.method === "Browser.downloadProgress" &&
      message.params?.state === "inProgress"
    ) {
      this.scheduleDownloadDiskCheck();
    }
    this.eventJournal.recordCdp(redactObservation(message, this.protectedValues), this.eventContext());
    if (
      message?.method === "Browser.downloadProgress" &&
      message.params?.state !== "inProgress"
    ) {
      const guid = message.params?.guid;
      if (typeof guid === "string") {
        const observed = this.downloads.get(guid);
        if (observed) {
          observed.state = message.params?.state;
          if (Number.isSafeInteger(message.params?.receivedBytes) && message.params.receivedBytes >= 0)
            observed.expectedBytes = message.params.receivedBytes;
        }
        this.targetsByDownload.delete(guid);
        this.downloadCancellationReasons.delete(guid);
      }
    }
  }

  scheduleDownloadDiskCheck() {
    if (this.targetsByDownload.size === 0) return;
    if (this.downloadDiskCheckPending) {
      this.downloadDiskCheckRequested = true;
      return;
    }
    this.downloadDiskCheckPending = true;
    const connection = this.connection;
    const browserGeneration = this.browserGeneration;
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
        browserGeneration !== this.browserGeneration
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
        browserGeneration !== this.browserGeneration
      ) return;
      this.downloadDiskCheckPending = false;
      if (this.downloadDiskCheckRequested) {
        this.downloadDiskCheckRequested = false;
        this.scheduleDownloadDiskCheck();
      }
    });
  }

  eventContext() {
    return {
      browserGeneration: this.browserGeneration,
      targetIdForSession: (sessionId) => this.targetsBySession.get(sessionId) ?? null,
      targetIdForFrame: (frameId) => this.targetsByFrame.get(frameId) ?? null,
      targetIdForDownload: (guid) => this.targetsByDownload.get(guid) ?? null,
      downloadCancellationReason: (guid) => this.downloadCancellationReasons.get(guid) ?? null,
      documentIdForTarget: (targetId) => this.documentIdsByTarget.get(targetId) ?? null,
    };
  }
}

export class CdpConnection {
  constructor(socket, requestTimeoutMs = DEFAULT_REQUEST_TIMEOUT_MS) {
    this.socket = socket;
    this.requestTimeoutMs = requestTimeoutMs;
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

  send(method, params = {}, sessionId, { timeoutMs = this.requestTimeoutMs } = {}) {
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
    // A timeout fires from a timer, with no caller on its stack: keep the caller's frames to name who waited.
    const caller = {};
    Error.captureStackTrace(caller);
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pending.delete(id);
        const error = new BrowserControllerError(
          "browser_cdp_timeout",
          `browser CDP method ${method} timed out after ${timeoutMs}ms`,
        );
        error.stack += caller.stack.replace(/^[^\n]*/, "");
        reject(error);
      }, timeoutMs);
      this.pending.set(id, { method, sessionId, resolve, reject, timeout });
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
    let message;
    try {
      message = JSON.parse(String(rawMessage));
    } catch {
      this.failPending("browser_cdp_invalid_json");
      return;
    }
    if (typeof message?.method === "string") {
      this.dispatchEvent(message);
      if (message.method === "Target.detachedFromTarget") this.failSession(message.params?.sessionId);
      if (message.method === "Inspector.targetCrashed") this.failSession(message.sessionId);
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

  // Chromium never answers a command whose session detached (its target
  // closed): fail those at once instead of at their timeout.
  failSession(sessionId) {
    if (typeof sessionId !== "string") return;
    for (const [id, pending] of this.pending) {
      if (pending.sessionId !== sessionId) continue;
      this.pending.delete(id);
      clearTimeout(pending.timeout);
      pending.reject(new BrowserControllerError("browser_cdp_command_failed", `${pending.method}: Target closed`));
    }
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

function validPage(page, viewport) {
  const fits = (value, max) => Number.isSafeInteger(value) && value > 0 && value <= max;
  return fits(page?.width, viewport.css_width) && fits(page?.height, viewport.css_height)
    ? { width: page.width, height: page.height }
    : null;
}

function appliedReconcileKey(viewport, browserBarVisible, appPanelCssWidth) {
  return JSON.stringify({ viewport, browserBarVisible, appPanelCssWidth });
}

function deviceMetricsFor(viewport, physicalDisplay = false) {
  return {
    width: viewport.css_width,
    height: viewport.css_height,
    deviceScaleFactor: viewport.device_scale_factor,
    // Room X desktops stay at host scale 1. DPR changes raster density, but
    // Chromium otherwise paints the emulated view at CSS size on that desktop.
    ...(physicalDisplay ? { scale: viewport.device_scale_factor } : {}),
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
  const connection = new CdpConnection(socket, requestTimeoutMs);
  // This is Chromium's physical browser endpoint, not our reconnect counter.
  // Upload staging must survive websocket and controller-process replacement.
  connection.browserInstanceId = debuggerUrl;
  return connection;
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
  if (error instanceof BrowserArtifactError) return new BrowserControllerError(error.code, error.message);
  if (error instanceof BrowserSnapshotError) {
    return normalizeNativeError(error);
  }
  if (error instanceof BrowserActionError) {
    return normalizeNativeError(error);
  }
  if (error instanceof BrowserFileTransferError) {
    return normalizeNativeError(error);
  }
  if (error instanceof BrowserPermissionError) {
    return normalizeNativeError(error);
  }
  if (error instanceof BrowserEventError) {
    return normalizeNativeError(error);
  }
  if (error instanceof BrowserCompatibilityError) {
    return normalizeNativeError(error);
  }
  if (error instanceof BrowserHistoryError) {
    return normalizeNativeError(error);
  }
  return new BrowserControllerError(
    "browser_controller_internal",
    String(error?.message ?? error),
  );
}
