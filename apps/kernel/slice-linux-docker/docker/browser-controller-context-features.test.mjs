import assert from "node:assert/strict";
import test from "node:test";

import {
  BrowserCdpClient,
  CdpConnection,
} from "./browser-controller-cdp.mjs";
import {
  BrowserActionError,
  actionabilityFunction,
  performBrowserAction,
} from "./browser-controller-actions.mjs";
import { BrowserDialogDefaults } from "./browser-controller-dialogs.mjs";
import { BrowserEventJournal } from "./browser-controller-events.mjs";
import {
  cancelBrowserDownload,
  uploadBrowserFiles,
} from "./browser-controller-files.mjs";
import {
  BrowserFrameTargets,
  withBrowserActionFrame,
  withBrowserFrames,
} from "./browser-controller-frames.mjs";
import { setBrowserPermission } from "./browser-controller-permissions.mjs";

const viewport = {
  css_width: 1280,
  css_height: 720,
  device_scale_factor: 1,
  desktop_pixel_width: 1280,
  desktop_pixel_height: 720,
};

test("M2 proves shadow traversal, nested-frame ownership, and cross-frame staleness", async () => {
  const ownerWindow = {
    getComputedStyle: () => ({ display: "block", visibility: "visible", opacity: "1" }),
  };
  ownerWindow.top = ownerWindow;
  const shadowRoot = {};
  const shadowChild = { getRootNode: () => shadowRoot };
  const shadowTarget = {
    isConnected: true,
    scrollIntoView: () => {},
    getBoundingClientRect: () => ({ left: 10, top: 20, width: 100, height: 30 }),
    matches: () => false,
    closest: () => null,
    getAttribute: () => null,
    contains: () => false,
    isContentEditable: false,
  };
  shadowRoot.host = shadowTarget;
  shadowTarget.shadowRoot = { elementFromPoint: () => shadowChild };
  shadowTarget.ownerDocument = {
    defaultView: ownerWindow,
    elementFromPoint: () => shadowTarget,
  };
  assert.equal(actionabilityFunction.call(shadowTarget).state, "ready");

  const topWindow = {
    getComputedStyle: () => ({ paddingLeft: "4px", paddingTop: "5px" }),
  };
  topWindow.top = topWindow;
  const frameElement = {
    clientLeft: 2,
    clientTop: 3,
    getBoundingClientRect: () => ({ left: 100, top: 50 }),
    ownerDocument: { defaultView: topWindow },
  };
  const childWindow = {
    top: topWindow,
    frameElement,
    getComputedStyle: () => ({ display: "block", visibility: "visible", opacity: "1" }),
  };
  const frameTarget = {
    isConnected: true,
    scrollIntoView: () => {},
    getBoundingClientRect: () => ({ left: 10, top: 20, width: 20, height: 10 }),
    matches: () => false,
    closest: () => null,
    getAttribute: () => null,
    contains: () => false,
    isContentEditable: false,
    ownerDocument: { defaultView: childWindow, elementFromPoint: () => frameTarget },
  };
  const coordinates = actionabilityFunction.call(frameTarget);
  assert.deepEqual([coordinates.x, coordinates.y], [126, 83]);

  const rootTree = {
    frame: { id: "root-frame", loaderId: "root-document" },
    childFrames: [{
      frame: { id: "child-frame", parentId: "root-frame", loaderId: "child-document" },
      childFrames: [{
        frame: { id: "grandchild-frame", parentId: "child-frame", loaderId: "grandchild-document" },
      }],
    }],
  };
  const childTree = rootTree.childFrames[0];
  const changedRootTree = {
    ...rootTree,
    frame: { ...rootTree.frame, loaderId: "root-replaced" },
  };
  const frameConnection = new FrameConnection(rootTree, childTree);
  const registry = new BrowserFrameTargets();
  registry.merge("tab-a", [rootTree]);
  assert.equal(registry.get("grandchild-frame"), "tab-a");
  registry.remove("child-frame");
  assert.equal(registry.get("grandchild-frame"), undefined);

  const observed = await withBrowserFrames(
    frameConnection,
    "root-session",
    "tab-a",
    "root-document",
    async (frames) => frames.map((entry) => ({
      sessionId: entry.sessionId,
      frameId: entry.frame.id,
      prefix: entry.prefix,
      nestedFrameId: entry.tree.childFrames?.[0]?.frame?.id,
    })),
  );
  assert.deepEqual(observed, [{
    sessionId: "root-session",
    frameId: "root-frame",
    prefix: "",
    nestedFrameId: "child-frame",
  }, {
    sessionId: "child-session",
    frameId: "child-frame",
    prefix: "frame:child-frame:child-document:",
    nestedFrameId: "grandchild-frame",
  }]);
  assert.deepEqual(frameConnection.calls.filter(({ method }) => method === "Target.detachFromTarget"), [
    { method: "Target.detachFromTarget", params: { sessionId: "child-session" }, sessionId: undefined },
  ]);

  frameConnection.rootTreeAfter = changedRootTree;
  frameConnection.rootReads = 0;
  await assert.rejects(
    withBrowserActionFrame({
      connection: frameConnection,
      sessionId: "root-session",
      targetId: "tab-a",
      documentId: "root-document",
      nodeRef: "frame:child-frame:child-document:backend:7",
    }, async (options) => {
      await options.assertContext();
      return { action_kind: "click" };
    }),
    { code: "stale_document_reference" },
  );
  assert.equal(
    frameConnection.calls.filter(({ method }) => method === "Target.detachFromTarget").length,
    2,
    "frame action cleanup must release the isolated child session after a stale parent check",
  );
});

test("M2 keeps dialog answers document-bound, exactly once, and bounded on CDP timeout", async () => {
  const connection = new ControllerConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const reconciled = await browser.reconcile(viewport);

  connection.emit({
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: { type: "prompt", defaultPrompt: "dialog-default-canary", message: "private-canary" },
  });
  const accepted = await browser.handleDialog({
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
  });
  assert.deepEqual(accepted, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
  });
  assert.deepEqual(lastCall(connection, "Page.handleJavaScriptDialog").params, {
    accept: true,
    promptText: "dialog-default-canary",
  });
  assert.equal(connection.calls.filter(({ method }) => method === "Page.handleJavaScriptDialog").length, 1);

  connection.emit({
    method: "Page.javascriptDialogClosed",
    sessionId: "session-a",
    params: { result: true },
  });
  connection.emit({
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: { type: "alert", message: "private-alert-canary" },
  });
  await browser.handleDialog({
    target_id: "target-a",
    document_id: "loader-a",
    action: "dismiss",
  });
  assert.deepEqual(lastCall(connection, "Page.handleJavaScriptDialog").params, { accept: false });

  connection.emit({
    method: "Page.javascriptDialogClosed",
    sessionId: "session-a",
    params: { result: false },
  });
  connection.emit({
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: { type: "prompt", defaultPrompt: "not-used", message: "private-prompt-canary" },
  });
  await browser.handleDialog({
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
    prompt_text: "explicit-answer",
  });
  assert.deepEqual(lastCall(connection, "Page.handleJavaScriptDialog").params, {
    accept: true,
    promptText: "explicit-answer",
  });
  assert.equal(
    connection.calls.filter(({ method }) => method === "Page.handleJavaScriptDialog").length,
    3,
    "each dialog request emits one authoritative CDP answer",
  );
  const trace = browser.pollEvents({
    browser_generation: reconciled.browser_generation,
    cursor: reconciled.event_cursor,
    limit: 20,
  });
  assert.equal(JSON.stringify(trace).includes("private-canary"), false);

  const socket = new ScriptedSocket();
  const cdp = new CdpConnection(socket, 10);
  const pending = cdp.send("Page.handleJavaScriptDialog", { accept: true }, "session-a");
  const command = socket.sent.at(-1);
  await assert.rejects(pending, { code: "browser_cdp_timeout" });
  socket.message({ id: command.id, result: {} });
  assert.equal(
    socket.sent.filter(({ method }) => method === "Page.handleJavaScriptDialog").length,
    1,
    "a timed-out dialog answer is not replayed after a late acknowledgement",
  );
  await cdp.close();
});

test("M2 attributes popup actions to stable target/document identities and bounds popup timeout", async () => {
  const connection = new ControllerConnection({
    targets: [
      { targetId: "target-a", type: "page", url: "https://a.test/", title: "Parent" },
      {
        targetId: "popup-b",
        type: "page",
        openerId: "target-a",
        url: "https://popup.test/authorize?state=private-canary",
        title: "Popup",
      },
    ],
  });
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const reconciled = await browser.reconcile(viewport);
  const parent = reconciled.tabs.find((tab) => tab.target_id === "target-a");
  const popup = reconciled.tabs.find((tab) => tab.target_id === "popup-b");
  assert.ok(parent);
  assert.ok(popup);
  assert.equal(popup.document_id, "loader-popup");
  assert.equal("opener_id" in popup, false, "the current tab contract has no invented opener field");

  const parentAction = await browser.performAction({
    target_id: parent.target_id,
    document_id: parent.document_id,
    node_ref: "backend:1",
    action: { kind: "click" },
  });
  const popupAction = await browser.performAction({
    target_id: popup.target_id,
    document_id: popup.document_id,
    node_ref: "backend:2",
    action: { kind: "click" },
  });
  assert.equal(parentAction.target_id, "target-a");
  assert.equal(parentAction.document_id, "loader-a");
  assert.equal(popupAction.target_id, "popup-b");
  assert.equal(popupAction.document_id, "loader-popup");

  connection.emit({
    method: "Target.targetCreated",
    params: {
      targetInfo: {
        targetId: "popup-b",
        type: "page",
        openerId: "target-a",
        url: "https://popup.test/authorize?state=private-canary",
      },
    },
  });
  const created = browser.pollEvents({
    browser_generation: reconciled.browser_generation,
    cursor: reconciled.event_cursor,
    limit: 20,
  }).events.at(-1);
  assert.deepEqual(created, {
    event_id: 2,
    browser_generation: 1,
    kind: "target_created",
    target_id: "popup-b",
    document_id: null,
    data: { url: "https://popup.test/authorize" },
  });

  connection.actionabilityBySession.set("session-popup-b", { state: "disabled" });
  let now = 0;
  await assert.rejects(
    performBrowserAction({
      connection,
      sessionId: "session-popup-b",
      targetId: "popup-b",
      documentId: "loader-popup",
      nodeRef: "backend:2",
      action: { kind: "click" },
      timeoutMs: 100,
      now: () => now,
      sleep: async (milliseconds) => { now += milliseconds; },
    }),
    (error) => error instanceof BrowserActionError
      && error.code === "browser_action_timeout"
      && error.reason === "disabled",
  );
});

test("M2 bounds download progress, cancellation ownership, and safe filenames", async () => {
  const connection = new ControllerConnection();
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    downloadDirectory: "/safe/downloads",
    fileSystem: {
      statfs: async () => ({ bavail: 1024 * 1024 * 1024, bsize: 1 }),
    },
  });
  const reconciled = await browser.reconcile(viewport);
  connection.emit({
    method: "Browser.downloadWillBegin",
    params: {
      frameId: "frame-a",
      guid: "download-m2",
      url: "https://a.test/file?token=private-canary#fragment",
      suggestedFilename: "../../safe/report.txt",
    },
  });
  const cancellation = await browser.cancelDownload({
    browser_generation: reconciled.browser_generation,
    guid: "download-m2",
  });
  assert.deepEqual(cancellation, {
    browser_generation: 1,
    guid: "download-m2",
    cancellation_requested: true,
  });
  connection.emit({
    method: "Browser.downloadProgress",
    params: {
      guid: "download-m2",
      state: "inProgress",
      receivedBytes: -10,
      totalBytes: Number.MAX_SAFE_INTEGER + 100,
    },
  });
  connection.emit({
    method: "Browser.downloadProgress",
    params: {
      guid: "download-m2",
      state: "canceled",
      receivedBytes: 12,
      totalBytes: 10,
    },
  });
  const events = browser.pollEvents({
    browser_generation: reconciled.browser_generation,
    cursor: reconciled.event_cursor,
    limit: 20,
  }).events;
  const started = events.find((event) => event.kind === "download_started");
  const progress = events.find((event) => event.kind === "download_progress" && event.data.state === "inProgress");
  const canceled = events.find((event) => event.kind === "download_progress" && event.data.state === "canceled");
  assert.equal(started.target_id, "target-a");
  assert.equal(started.data.suggested_filename, "report.txt");
  assert.equal(started.data.url, "https://a.test/file");
  assert.equal(progress.target_id, "target-a");
  assert.equal(progress.data.received_bytes, 0);
  assert.equal(progress.data.total_bytes, Number.MAX_SAFE_INTEGER);
  assert.equal(canceled.data.received_bytes, 12);
  assert.equal(connection.calls.filter(({ method }) => method === "Browser.cancelDownload").length, 1);
  await assert.rejects(
    browser.cancelDownload({ browser_generation: 1, guid: "download-m2" }),
    { code: "browser_download_not_active" },
  );
  assert.equal(JSON.stringify(events).includes("private-canary"), false);
});

test("M2 validates local and transferred uploads while rejecting remote, missing, and denied files", async () => {
  const connection = new UploadConnection();
  const entries = new Map([
    ["/uploads", { type: "directory", realpath: "/uploads" }],
    ["/uploads/local.txt", { type: "file", realpath: "/uploads/local.txt", size: 5 }],
    ["/uploads/transferred.bin", { type: "file", realpath: "/uploads/transferred.bin", size: 7 }],
    ["/uploads/link.txt", { type: "file", realpath: "/outside/private-canary", size: 1 }],
    ["/outside/private-canary", { type: "file", realpath: "/outside/private-canary", size: 1 }],
    ["/remote/report.txt", { type: "file", realpath: "/remote/report.txt", size: 2 }],
  ]);
  const fileSystem = {
    realpath: async (value) => {
      const entry = entries.get(value);
      if (!entry) throw Object.assign(new Error("missing"), { code: "ENOENT" });
      return entry.realpath;
    },
    stat: async (value) => {
      const entry = entries.get(value);
      if (!entry) throw Object.assign(new Error("missing"), { code: "ENOENT" });
      return {
        size: entry.size ?? 0,
        isDirectory: () => entry.type === "directory",
        isFile: () => entry.type === "file",
      };
    },
  };
  const upload = async (filePath) => uploadBrowserFiles({
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    nodeRef: "backend:103",
    filePaths: [filePath],
    uploadRoots: ["/uploads"],
    fileSystem,
  });

  const local = await upload("/uploads/local.txt");
  const transferred = await upload("/uploads/transferred.bin");
  assert.deepEqual(local, {
    target_id: "target-a",
    document_id: "loader-a",
    file_count: 1,
    total_bytes: 5,
  });
  assert.equal(transferred.total_bytes, 7);
  assert.equal(JSON.stringify({ local, transferred }).includes("/uploads"), false);
  await assert.rejects(upload("/remote/report.txt"), { code: "browser_upload_denied" });
  await assert.rejects(upload("/uploads/link.txt"), { code: "browser_upload_denied" });
  await assert.rejects(upload("/uploads/missing.txt"), { code: "browser_upload_invalid" });
  assert.equal(
    connection.calls.filter(({ method }) => method === "DOM.setFileInputFiles").length,
    2,
    "only validated local/transferred files reach the renderer",
  );
});

test("M2 resolves permission allow/deny once and fails closed on bounded timeout", async () => {
  const connection = new PermissionConnection();
  for (const setting of ["granted", "denied"]) {
    const result = await setBrowserPermission({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      targetUrl: "https://a.test/settings?token=private-canary",
      permission: "clipboard-sanitized-write",
      setting,
    });
    assert.deepEqual(result, {
      target_id: "target-a",
      document_id: "loader-a",
      permission: "clipboard-sanitized-write",
      setting,
    });
  }
  const decisions = connection.calls.filter(({ method }) => method === "Browser.setPermission");
  assert.deepEqual(decisions.map(({ params }) => params.setting), ["granted", "denied"]);
  assert.deepEqual(decisions[0].params.permission, {
    name: "clipboard-write",
    allowWithoutSanitization: false,
  });
  assert.equal(JSON.stringify(decisions).includes("private-canary"), false);

  const socket = new ScriptedSocket((command, current) => {
    if (command.method === "Page.getFrameTree") {
      current.message({ id: command.id, result: { frameTree: { frame: { loaderId: "loader-a" } } } });
    }
  });
  const cdp = new CdpConnection(socket, 10);
  await assert.rejects(
    setBrowserPermission({
      connection: cdp,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      targetUrl: "https://a.test/",
      permission: "geolocation",
      setting: "prompt",
    }),
    { code: "browser_cdp_timeout" },
  );
  assert.equal(socket.sent.filter(({ method }) => method === "Browser.setPermission").length, 1);
  socket.message({ id: socket.sent.at(-1).id, result: {} });
  assert.equal(socket.sent.filter(({ method }) => method === "Browser.setPermission").length, 1);
  await cdp.close();
});

test("M2 rejects stale tab/frame/document/generation references and preserves journal recovery boundaries", async () => {
  const firstConnection = new ControllerConnection();
  const secondConnection = new ControllerConnection({
    targets: [{ targetId: "target-a", type: "page", url: "https://a.test/reconnected", title: "A" }],
  });
  const connections = [firstConnection, secondConnection];
  const browser = new BrowserCdpClient({ connectionFactory: async () => connections.shift() });
  const first = await browser.reconcile(viewport);

  await assert.rejects(
    browser.manageTab({ target_id: "target-a", document_id: "loader-old", action: "activate" }),
    { code: "stale_document_reference" },
  );
  await assert.rejects(
    browser.manageTab({ target_id: "missing-tab", document_id: "loader-a", action: "activate" }),
    { code: "browser_target_not_found" },
  );
  await assert.rejects(
    browser.setPermission({
      target_id: "target-a",
      document_id: "loader-old",
      permission: "geolocation",
      setting: "granted",
    }),
    { code: "stale_document_reference" },
  );

  firstConnection.emit({
    method: "Browser.downloadWillBegin",
    params: { frameId: "frame-a", guid: "generation-bound", url: "https://a.test/file", suggestedFilename: "file" },
  });
  await assert.rejects(
    browser.cancelDownload({ browser_generation: first.browser_generation + 1, guid: "generation-bound" }),
    { code: "stale_browser_generation" },
  );
  assert.equal(browser.pollEvents({ browser_generation: first.browser_generation + 1, cursor: 0, limit: 10 }).replay_gap, true);

  await browser.close();
  const second = await browser.reconcile(viewport);
  assert.equal(second.browser_generation, 2);
  assert.equal(browser.pollEvents({ browser_generation: first.browser_generation, cursor: 0, limit: 10 }).replay_gap, true);
  assert.equal(second.tabs[0].document_id, "loader-a");
});

test("M2 journal diagnostics are bounded, secret-safe, and explicit about replay gaps", () => {
  const journal = new BrowserEventJournal({ capacity: 2 });
  const context = {
    browserGeneration: 1,
    targetIdForSession: () => "target-a",
    documentIdForTarget: () => "loader-a",
    targetIdForFrame: () => "target-a",
    targetIdForDownload: () => "target-a",
  };
  journal.recordCdp({
    method: "Network.requestWillBeSent",
    sessionId: "session-a",
    params: {
      requestId: "request-a",
      request: { method: "GET", url: "https://user:pass@example.test/path?token=private-canary#fragment" },
      type: "Document",
    },
  }, context);
  journal.recordCdp({
    method: "Network.loadingFailed",
    sessionId: "session-a",
    params: { requestId: "request-a", errorText: "private-canary stack", canceled: false, type: "Document" },
  }, context);
  journal.recordCdp({
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: { type: "prompt", message: "private-canary", defaultPrompt: "private-canary" },
  }, context);
  journal.recordCdp({
    method: "Browser.downloadWillBegin",
    params: { frameId: "frame-a", guid: "safe-guid", url: "https://example.test/download?token=private-canary", suggestedFilename: "../../safe.txt" },
  }, context);

  const replayGap = journal.poll({ browserGeneration: 1, cursor: 0, limit: 20 });
  assert.equal(replayGap.replay_gap, true, "bounded journals must require resnapshot after eviction");
  const current = journal.poll({ browserGeneration: 1, cursor: replayGap.next_cursor - 2, limit: 20 });
  assert.equal(current.replay_gap, false);
  assert.equal(JSON.stringify(current).includes("private-canary"), false);
  assert.equal(current.events.find((event) => event.kind === "download_started").data.suggested_filename, "safe.txt");
  assert.equal(current.events.find((event) => event.kind === "dialog_opened").data.has_default_prompt, true);
  assert.equal(
    journal.poll({ browserGeneration: 2, cursor: current.next_cursor, limit: 20 }).replay_gap,
    true,
  );

  const defaults = new BrowserDialogDefaults();
  defaults.observe({
    method: "Page.javascriptDialogOpening",
    params: { type: "prompt", defaultPrompt: "private-canary" },
  }, "target-a", "loader-a");
  assert.equal(defaults.get("target-a", "loader-a"), "private-canary");
  defaults.observe({
    method: "Page.javascriptDialogClosed",
    params: {},
  }, "target-a", "loader-a");
  assert.equal(defaults.get("target-a", "loader-a"), undefined);
});

function lastCall(connection, method) {
  return connection.calls.filter((call) => call.method === method).at(-1);
}

class ControllerConnection {
  constructor({
    targets = [
      { targetId: "target-a", type: "page", url: "https://a.test/", title: "A" },
      { targetId: "target-b", type: "page", url: "https://b.test/", title: "B" },
    ],
  } = {}) {
    this.calls = [];
    this.listeners = new Set();
    this.open = true;
    this.targets = targets;
    this.closedTargetIds = new Set();
    this.actionabilityBySession = new Map();
  }

  isOpen() { return this.open; }

  async close() { this.open = false; }

  subscribe(listener) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  emit(message) {
    for (const listener of this.listeners) listener(message);
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    if (method === "Target.getTargets") {
      return {
        targetInfos: this.targets.filter((target) => !this.closedTargetIds.has(target.targetId)),
      };
    }
    if (method === "Target.attachToTarget") {
      const sessionId = params.targetId === "target-a"
        ? "session-a"
        : params.targetId === "target-b"
          ? "session-b"
          : `session-${params.targetId}`;
      return { sessionId };
    }
    if (method === "Target.closeTarget") {
      this.closedTargetIds.add(params.targetId);
      return { success: true };
    }
    if (method === "Page.getFrameTree") {
      const targetId = sessionId?.replace(/^session-/, "") ?? "target-a";
      const frameId = targetId === "popup-b" ? "frame-popup" : `frame-${targetId.replace("target-", "")}`;
      const loaderId = targetId === "popup-b" ? "loader-popup" : `loader-${targetId.replace("target-", "")}`;
      return { frameTree: { frame: { id: frameId, loaderId } } };
    }
    if (method === "Runtime.evaluate") {
      return { result: { value: sessionId !== "session-popup-b" } };
    }
    if (method === "DOM.resolveNode") return { object: { objectId: `object-${sessionId}` } };
    if (method === "Runtime.callFunctionOn") {
      if (params.functionDeclaration?.includes("scrollIntoView")) {
        return { result: { value: this.actionabilityBySession.get(sessionId) ?? {
          state: "ready",
          x: 50,
          y: 50,
          width: 100,
          height: 30,
        } } };
      }
      return { result: { value: { ok: true } } };
    }
    if (method === "Browser.setPermission") return {};
    if (method === "Page.handleJavaScriptDialog") return {};
    if (method === "Target.activateTarget") return {};
    if (method === "Browser.cancelDownload") return {};
    if (method === "Runtime.releaseObject") return {};
    if (method.startsWith("Input.")) return {};
    return {};
  }
}

class FrameConnection {
  constructor(rootTree, childTree) {
    this.rootTree = rootTree;
    this.childTree = childTree;
    this.rootTreeAfter = null;
    this.rootReads = 0;
    this.calls = [];
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    if (method === "Page.getFrameTree") {
      if (sessionId === "child-session") return { frameTree: this.childTree };
      const frameTree = this.rootReads++ === 0 || !this.rootTreeAfter
        ? this.rootTree
        : this.rootTreeAfter;
      return { frameTree };
    }
    if (method === "Target.getTargets") {
      return { targetInfos: [{ targetId: "child-target", type: "iframe" }] };
    }
    if (method === "Target.attachToTarget") return { sessionId: "child-session" };
    if (method === "Target.detachFromTarget") return {};
    return {};
  }
}

class UploadConnection {
  constructor() {
    this.calls = [];
    this.loaderId = "loader-a";
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    if (method === "Page.getFrameTree") return { frameTree: { frame: { loaderId: this.loaderId } } };
    if (method === "DOM.resolveNode") return { object: { objectId: "file-object" } };
    if (method === "Runtime.callFunctionOn") return { result: { value: "file" } };
    return {};
  }
}

class PermissionConnection {
  constructor() {
    this.calls = [];
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    if (method === "Page.getFrameTree") return { frameTree: { frame: { loaderId: "loader-a" } } };
    return {};
  }
}

class ScriptedSocket {
  constructor(onSend = () => {}) {
    this.readyState = 1;
    this.sent = [];
    this.listeners = new Map();
    this.onSend = onSend;
  }

  addEventListener(kind, listener) {
    const listeners = this.listeners.get(kind) ?? new Set();
    listeners.add(listener);
    this.listeners.set(kind, listeners);
  }

  send(payload) {
    const command = JSON.parse(payload);
    this.sent.push(command);
    this.onSend(command, this);
  }

  close() {
    this.readyState = 3;
    for (const listener of this.listeners.get("close") ?? []) listener({});
  }

  message(payload) {
    for (const listener of this.listeners.get("message") ?? []) {
      listener({ data: JSON.stringify(payload) });
    }
  }
}
