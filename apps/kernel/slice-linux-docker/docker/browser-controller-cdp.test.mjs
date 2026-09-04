import assert from "node:assert/strict";
import test from "node:test";

import {
  assertPrivateDebuggerUrl,
  BrowserCdpClient,
  CdpConnection,
} from "./browser-controller-cdp.mjs";
import { handleBrowserControllerRequest } from "./browser-controller.mjs";

const viewport = {
  css_width: 1280,
  css_height: 720,
  device_scale_factor: 2,
  desktop_pixel_width: 2560,
  desktop_pixel_height: 1440,
};

test("persistent browser connection returns page identities, focus, and applied viewport", async () => {
  const connection = new FakeConnection();
  let connectionCount = 0;
  const browser = new BrowserCdpClient({
    connectionFactory: async () => {
      connectionCount += 1;
      return connection;
    },
  });

  const first = await browser.reconcile(viewport);
  const second = await browser.reconcile(viewport);

  assert.equal(connectionCount, 1);
  assert.equal(first.browser_generation, 1);
  assert.equal(first.event_cursor, 1);
  assert.deepEqual(first, second);
  assert.deepEqual(first.tabs, [
    {
      target_id: "target-a",
      document_id: "loader-a",
      url: "https://a.test/",
      title: "A",
    },
    {
      target_id: "target-b",
      document_id: "loader-b",
      url: "https://b.test/",
      title: "B",
    },
  ]);
  assert.equal(first.focused_target_id, "target-b");
  assert.deepEqual(first.viewport, viewport);
  assert.equal(
    connection.calls.filter((call) => call.method === "Target.attachToTarget").length,
    2,
    "persistent target sessions should be reused",
  );
  assert.equal(
    connection.calls.filter((call) => call.method === "Page.enable").length,
    2,
    "each persistent target session should subscribe to Page lifecycle events once",
  );
  for (const method of [
    "Page.setLifecycleEventsEnabled",
    "Runtime.enable",
    "Network.enable",
    "Inspector.enable",
  ]) {
    assert.equal(
      connection.calls.filter((call) => call.method === method).length,
      2,
      `${method} should run once for each persistent target session`,
    );
  }
  assert.deepEqual(
    connection.calls.find(
      (call) => call.method === "Emulation.setDeviceMetricsOverride" && call.sessionId === "session-a",
    ).params,
    {
      width: 1280,
      height: 720,
      deviceScaleFactor: 2,
      mobile: false,
      screenWidth: 1280,
      screenHeight: 720,
    },
  );
  assert.equal(
    connection.calls.find(
      (call) => call.method === "Runtime.evaluate" && call.sessionId === "session-a",
    ).params.expression,
    "document.visibilityState === 'visible'",
    "active-tab focus must not disappear merely because the browser window loses OS focus",
  );
});

test("viewport validation happens before opening a browser connection", async () => {
  let connected = false;
  const browser = new BrowserCdpClient({
    connectionFactory: async () => {
      connected = true;
      return new FakeConnection();
    },
  });

  await assert.rejects(
    browser.reconcile({ ...viewport, css_width: 0 }),
    (error) => error.code === "browser_viewport_invalid",
  );
  assert.equal(connected, false);
});

test("failed event subscription closes the connection before a clean reconnect", async () => {
  const failed = new FakeConnection();
  failed.failDiscovery = true;
  const recovered = new FakeConnection();
  const connections = [failed, recovered];
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connections.shift(),
  });

  await assert.rejects(browser.reconcile(viewport), /discovery failed/);
  assert.equal(failed.open, false);
  const result = await browser.reconcile(viewport);

  assert.equal(result.browser_generation, 2);
  assert.equal(result.event_cursor, 1);
});

test("a detached target session is discarded before the next reconcile", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);

  connection.emit({
    method: "Target.detachedFromTarget",
    sessionId: "session-a",
    params: { sessionId: "session-a", targetId: "target-a" },
  });
  await browser.reconcile(viewport);

  assert.equal(
    connection.calls.filter(
      (call) => call.method === "Target.attachToTarget" && call.params.targetId === "target-a",
    ).length,
    2,
  );
});

test("tab lifecycle operations stay document-bound and use browser target commands", async () => {
  const connection = new DelayedTargetCloseConnection(1);
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);

  const activated = await browser.manageTab({
    target_id: "target-a",
    document_id: "loader-a",
    action: "activate",
  });
  const closed = await browser.manageTab({
    target_id: "target-a",
    document_id: "loader-a",
    action: "close",
  });

  assert.deepEqual(activated, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    action: "activate",
  });
  assert.deepEqual(closed, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    action: "close",
  });
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Target.activateTarget")?.params,
    { targetId: "target-a" },
  );
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Target.closeTarget")?.params,
    { targetId: "target-a" },
  );
  await assert.rejects(
    browser.manageTab({ target_id: "target-b", document_id: "loader-old", action: "activate" }),
    (error) => error.code === "stale_document_reference",
  );
  await assert.rejects(
    browser.manageTab({ target_id: "target-b", document_id: "loader-b", action: "detach" }),
    (error) => error.code === "browser_tab_action_invalid",
  );
});

test("tab close settles target removal before reporting completion", async () => {
  const connection = new DelayedTargetCloseConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);

  const closed = await browser.manageTab({
    target_id: "target-b",
    document_id: "loader-b",
    action: "close",
  });

  assert.equal(closed.action, "close");
  assert.equal(connection.closurePolls, 2);
  const reconciled = await browser.reconcile(viewport);
  assert.deepEqual(reconciled.tabs.map((tab) => tab.target_id), ["target-a"]);
  assert.equal(reconciled.focused_target_id, "target-a");
});

test("tab close fails within the request timeout when Chrome retains the target", async () => {
  const connection = new DelayedTargetCloseConnection(Number.POSITIVE_INFINITY);
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    requestTimeoutMs: 1,
  });
  await browser.reconcile(viewport);

  await assert.rejects(
    browser.manageTab({
      target_id: "target-b",
      document_id: "loader-b",
      action: "close",
    }),
    (error) => error.code === "browser_tab_close_failed" && /within 1ms/.test(error.message),
  );
});

test("legacy navigation and waits reuse the reconciled target session", async () => {
  const connection = new CompatibilityConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);

  const navigation = await browser.navigate({
    target_id: "target-a",
    document_id: "loader-a",
    url: "https://example.test/settings",
  });
  const selector = await browser.wait({
    target_id: "target-a",
    document_id: "loader-next",
    kind: "selector",
    selector: "#settings",
    timeout_ms: 500,
  });
  const idle = await browser.wait({
    target_id: "target-a",
    document_id: "loader-next",
    kind: "idle",
    timeout_ms: 500,
  });

  assert.equal(navigation.document_id, "loader-next");
  assert.equal(selector.kind, "selector");
  assert.equal(idle.kind, "idle");
  assert.equal(
    connection.calls.filter(
      (call) => call.method === "Target.attachToTarget" && call.params.targetId === "target-a",
    ).length,
    1,
    "compatibility calls must retain the controller-owned target session",
  );
});

test("structured snapshots bind compact accessibility and DOM nodes to one document", async () => {
  const connection = new SnapshotConnection();
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
  });
  await browser.reconcile(viewport);

  const first = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });
  const second = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });

  assert.equal(first.browser_generation, 1);
  assert.equal(first.target_id, "target-a");
  assert.equal(first.document_id, "loader-a");
  assert.equal(first.snapshot_revision, 1);
  assert.equal(second.snapshot_revision, 2);
  assert.deepEqual(first.accessibility_nodes, [
    {
      node_ref: "backend:103",
      parent_ref: null,
      child_refs: ["backend:104"],
      role: "button",
      name: "Save",
      description: "Save this form",
      value: "",
      ignored: false,
      disabled: false,
      focused: true,
    },
    {
      node_ref: "backend:104",
      parent_ref: "backend:103",
      child_refs: [],
      role: "StaticText",
      name: "Save",
      description: "",
      value: "",
      ignored: false,
      disabled: false,
      focused: false,
    },
  ]);
  assert.deepEqual(
    first.dom_nodes.find((node) => node.node_ref === "backend:103"),
    {
      node_ref: "backend:103",
      parent_ref: "backend:102",
      document_index: 0,
      node_type: 1,
      node_name: "BUTTON",
      text: "",
      attributes: { id: "save", type: "submit" },
      bounds: { x: 10, y: 20, width: 100, height: 30 },
    },
  );
  assert.equal(
    first.dom_nodes.find((node) => node.node_ref === "backend:200").document_index,
    1,
    "frame nodes must retain the document that owns them",
  );
  assert.deepEqual(
    first.dom_nodes.find((node) => node.node_ref === "backend:105").attributes,
    { type: "password", value: "[redacted]" },
    "password values must never leave the controller snapshot",
  );
  assert.deepEqual(first.dom_documents, [
    { document_index: 0, url: "https://a.test/", owner_node_ref: null },
    { document_index: 1, url: "https://frame.test/", owner_node_ref: "backend:103" },
  ]);
  assert.deepEqual(first.shadow_roots, [
    { node_ref: "backend:102", shadow_root_type: "open" },
  ]);

  connection.loaderId = "loader-a2";
  await assert.rejects(
    browser.snapshot({ target_id: "target-a", document_id: "loader-a" }),
    (error) => error.code === "stale_document_reference",
  );
  connection.loaderId = "loader-a";
  const afterRejectedCapture = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });
  assert.equal(
    afterRejectedCapture.snapshot_revision,
    3,
    "a rejected capture must not consume a snapshot revision",
  );
});

test("structured snapshot strings are bounded by UTF-8 bytes", async () => {
  const connection = new SnapshotConnection();
  connection.accessibilityName = "😀".repeat(2_000);
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
  });
  await browser.reconcile(viewport);

  const snapshot = await browser.snapshot({
    target_id: "target-a",
    document_id: "loader-a",
  });

  assert.equal(
    new TextEncoder().encode(snapshot.accessibility_nodes[0].name).length,
    2_048,
  );
});

test("dialog handling is document-bound and validates prompt input", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
  });
  await browser.reconcile(viewport);
  const frameTreeCallsBeforeDialog = connection.calls.filter(
    (call) => call.method === "Page.getFrameTree",
  ).length;

  const accepted = await browser.handleDialog({
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
    prompt_text: "answer",
  });

  assert.deepEqual(accepted, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
  });
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Page.handleJavaScriptDialog").params,
    { accept: true, promptText: "answer" },
  );
  await browser.handleDialog({
    target_id: "target-a",
    document_id: "loader-a",
    action: "accept",
    prompt_text: "",
  });
  assert.deepEqual(
    connection.calls.filter((call) => call.method === "Page.handleJavaScriptDialog").at(-1).params,
    { accept: true, promptText: "" },
  );
  await browser.handleDialog({
    target_id: "target-a",
    document_id: "loader-a",
    action: "dismiss",
    prompt_text: null,
  });
  assert.deepEqual(
    connection.calls.filter((call) => call.method === "Page.handleJavaScriptDialog").at(-1).params,
    { accept: false },
  );
  assert.equal(
    connection.calls.filter((call) => call.method === "Page.getFrameTree").length,
    frameTreeCallsBeforeDialog,
    "dialog handling must use the last controller-observed document because page commands block while a dialog is open",
  );
  await assert.rejects(
    browser.handleDialog({
      target_id: "target-a",
      document_id: "loader-b",
      action: "dismiss",
    }),
    (error) => error.code === "stale_document_reference",
  );
  await assert.rejects(
    browser.handleDialog({
      target_id: "target-a",
      document_id: "loader-a",
      action: "ignore",
    }),
    (error) => error.code === "browser_dialog_invalid",
  );
});

test("download and upload requests stay target-bound and return no file paths", async () => {
  const connection = new FakeConnection();
  const entries = new Map([
    ["/safe/downloads", { type: "directory", size: 0 }],
    ["/safe/uploads", { type: "directory", size: 0 }],
    ["/safe/uploads/report.txt", { type: "file", size: 12 }],
  ]);
  const fileSystem = {
    mkdir: async () => {},
    realpath: async (candidate) => {
      if (!entries.has(candidate)) throw Object.assign(new Error("missing"), { code: "ENOENT" });
      return candidate;
    },
    stat: async (candidate) => {
      const entry = entries.get(candidate);
      if (!entry) throw Object.assign(new Error("missing"), { code: "ENOENT" });
      return {
        size: entry.size,
        isDirectory: () => entry.type === "directory",
        isFile: () => entry.type === "file",
      };
    },
  };
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    downloadDirectory: "/safe/downloads",
    uploadRoots: ["/safe/uploads"],
    fileSystem,
  });
  await browser.reconcile(viewport);

  const downloads = await browser.configureDownloads({
    target_id: "target-a",
    document_id: "loader-a",
  });
  const upload = await browser.uploadFiles({
    target_id: "target-a",
    document_id: "loader-a",
    node_ref: "backend:103",
    file_paths: ["/safe/uploads/report.txt"],
  });

  assert.deepEqual(downloads, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    enabled: true,
  });
  assert.deepEqual(upload, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    file_count: 1,
    total_bytes: 12,
  });
  assert.equal(JSON.stringify({ downloads, upload }).includes("/safe"), false);
});

test("prompt defaults remain document-bound, private, and cleared when the dialog lifecycle ends", async () => {
  let connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const target = { target_id: "target-a", document_id: "loader-a", action: "accept" };
  await browser.reconcile(viewport);
  const open = () => connection.emit({
    method: "Page.javascriptDialogOpening", sessionId: "session-a",
    params: { type: "prompt", defaultPrompt: "private default", message: "private message" },
  });
  const lastAnswer = () => connection.calls.filter((call) => call.method === "Page.handleJavaScriptDialog").at(-1).params;
  open();
  const result = await browser.handleDialog(target);
  assert.deepEqual(lastAnswer(), { accept: true, promptText: "private default" });
  assert.equal(JSON.stringify(result).includes("private"), false);
  const trace = browser.pollEvents({ cursor: 0, browser_generation: 1 });
  assert.equal(JSON.stringify(trace).includes("private"), false);
  await assert.rejects(browser.handleDialog({ ...target, document_id: "old-document" }), (error) => error.code === "stale_document_reference");
  await browser.handleDialog({ ...target, prompt_text: "" });
  assert.deepEqual(lastAnswer(), { accept: true, promptText: "" });
  await browser.handleDialog({ target_id: "target-b", document_id: "loader-b", action: "accept" });
  assert.deepEqual(lastAnswer(), { accept: true });

  for (const event of [
    { method: "Page.javascriptDialogClosed", sessionId: "session-a", params: { result: true } },
    { method: "Page.frameNavigated", sessionId: "session-a", params: { frame: { id: "frame-a", loaderId: "loader-a" } } },
    { method: "Target.targetCrashed", params: { targetId: "target-a" } },
    { method: "Target.targetDestroyed", params: { targetId: "target-a" } },
    { method: "Inspector.targetCrashed", sessionId: "session-a", params: {} },
    { method: "Target.detachedFromTarget", params: { sessionId: "session-a", targetId: "target-a" } },
  ]) {
    open();
    connection.emit(event);
    await browser.handleDialog(target);
    assert.deepEqual(lastAnswer(), { accept: true }, event.method);
  }
  open();
  await browser.close();
  connection = new FakeConnection();
  await browser.reconcile(viewport);
  await browser.handleDialog(target);
  assert.deepEqual(lastAnswer(), { accept: true });
  await browser.close();
});

test("permission decisions derive the current target origin", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);

  const result = await browser.setPermission({
    target_id: "target-a",
    document_id: "loader-a",
    permission: "geolocation",
    setting: "prompt",
  });

  assert.deepEqual(result, {
    browser_generation: 1,
    target_id: "target-a",
    document_id: "loader-a",
    permission: "geolocation",
    setting: "prompt",
  });
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Browser.setPermission").params,
    {
      permission: { name: "geolocation" },
      setting: "prompt",
      origin: "https://a.test",
    },
  );
});

test("controller event polling uses the reconciliation cursor and persistent session mapping", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const reconciled = await browser.reconcile(viewport);
  connection.emit({
    method: "Runtime.consoleAPICalled",
    sessionId: "session-a",
    params: { type: "warning", args: [{ value: "must-not-leave-controller" }] },
  });

  const replay = browser.pollEvents({
    browser_generation: reconciled.browser_generation,
    cursor: reconciled.event_cursor,
    limit: 10,
  });
  assert.equal(replay.replay_gap, false);
  assert.equal(replay.events.length, 1);
  assert.equal(replay.events[0].target_id, "target-a");
  assert.equal(replay.events[0].document_id, "loader-a");
  assert.equal(JSON.stringify(replay).includes("must-not-leave-controller"), false);

  connection.emit({
    method: "Browser.downloadWillBegin",
    params: { frameId: "frame-a", guid: "download-a", url: "https://a.test/file?secret=1", suggestedFilename: "report.txt" },
  });
  connection.emit({
    method: "Browser.downloadProgress",
    params: { guid: "download-a", state: "completed", receivedBytes: 12, totalBytes: 12 },
  });
  const downloads = browser.pollEvents({
    browser_generation: reconciled.browser_generation,
    cursor: replay.next_cursor,
    limit: 10,
  });
  assert.deepEqual(downloads.events.map((event) => event.target_id), ["target-a", "target-a"]);
  assert.equal(JSON.stringify(downloads).includes("secret"), false);
});

test("debugger discovery cannot redirect the controller away from loopback", () => {
  assert.equal(
    assertPrivateDebuggerUrl(
      "ws://localhost:9222/devtools/browser/one",
      "http://127.0.0.1:9222",
    ),
    "ws://localhost:9222/devtools/browser/one",
  );
  assert.throws(
    () =>
      assertPrivateDebuggerUrl(
        "ws://example.test:9222/devtools/browser/one",
        "http://127.0.0.1:9222",
      ),
    (error) => error.code === "browser_debugger_endpoint_unsafe",
  );
  assert.throws(
    () =>
      assertPrivateDebuggerUrl(
        "ws://127.0.0.1:9333/devtools/browser/one",
        "http://127.0.0.1:9222",
      ),
    (error) => error.code === "browser_debugger_endpoint_unsafe",
  );
});

test("CDP responses are correlated and command failures stay bounded", async () => {
  const socket = new FakeSocket();
  const connection = new CdpConnection(socket, 20);
  const response = connection.send("Target.getTargets");
  const request = JSON.parse(socket.sent[0]);
  socket.message({ id: request.id, result: { targetInfos: [] } });
  assert.deepEqual(await response, { targetInfos: [] });

  const dialog = connection.waitForEvent("Page.javascriptDialogOpening", 20);
  const observed = [];
  connection.subscribe((event) => observed.push(event.method));
  socket.message({
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: { type: "prompt", message: "Name?" },
  });
  assert.deepEqual(await dialog.promise, {
    method: "Page.javascriptDialogOpening",
    sessionId: "session-a",
    params: { type: "prompt", message: "Name?" },
  });
  assert.deepEqual(observed, ["Page.javascriptDialogOpening"]);

  const timedOut = connection.send("Page.getFrameTree", {}, "session-a");
  await assert.rejects(timedOut, (error) => error.code === "browser_cdp_timeout");
  await connection.close();
});

test("timed-out or disconnected dialog replies terminate and do not leak defaults into a new connection", async (t) => {
  let socket = new DialogFaultSocket();
  const browser = new BrowserCdpClient({ connectionFactory: async () => new CdpConnection(socket, 100) });
  t.after(() => browser.close());
  let id = 0;
  const request = (method, params) => handleBrowserControllerRequest({ id: ++id, method, params }, { browser });
  const target = { target_id: "target-a", document_id: "loader-a", action: "accept" };
  assert.equal((await request("browser.reconcile", { viewport })).ok, true);
  socket.message({ method: "Page.javascriptDialogOpening", sessionId: "session-a", params: { type: "prompt", defaultPrompt: "old private default" } });
  socket.holdDialog = true;
  const timedOut = await request("browser.dialog", target);
  assert.equal(timedOut.ok, false);
  assert.equal(timedOut.error.code, "browser_cdp_timeout");
  // A late acknowledgement belongs only to the expired request.
  socket.message({ id: socket.heldDialog.id, result: {} });
  socket.holdDialog = false;
  assert.equal((await request("browser.dialog", target)).ok, true);
  assert.equal(socket.lastDialog.params.promptText, "old private default");

  socket.holdDialog = true;
  socket.dialogSent = Promise.withResolvers();
  const disconnectedRequest = request("browser.dialog", target);
  await socket.dialogSent.promise;
  socket.close();
  const disconnected = await disconnectedRequest;
  assert.equal(disconnected.ok, false);
  assert.equal(disconnected.error.code, "browser_cdp_disconnected");
  socket = new DialogFaultSocket();
  const reconnected = await request("browser.reconcile", { viewport });
  assert.equal(reconnected.ok, true, JSON.stringify(reconnected.error));
  assert.equal(reconnected.result.browser_generation, 2);
  assert.equal((await request("browser.dialog", target)).ok, true);
  assert.deepEqual(socket.lastDialog.params, { accept: true }, "a new connection must not reuse the previous dialog default");
  const trace = await request("browser.events.poll", { cursor: 0, browser_generation: 2 });
  assert.equal(JSON.stringify(trace).includes("old private default"), false);
});

class FakeConnection {
  constructor() {
    this.calls = [];
    this.open = true;
    this.listeners = new Set();
  }

  isOpen() {
    return this.open;
  }

  async close() {
    this.open = false;
  }

  subscribe(listener) {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  emit(message) {
    for (const listener of this.listeners) listener(message);
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    if (method === "Target.setDiscoverTargets" && this.failDiscovery) {
      throw new Error("discovery failed");
    }
    if (method === "Target.getTargets") {
      return {
        targetInfos: [
          { targetId: "target-a", type: "page", url: "https://a.test/", title: "A" },
          { targetId: "worker-a", type: "service_worker", url: "https://a.test/sw.js" },
          { targetId: "target-b", type: "page", url: "https://b.test/", title: "B" },
        ],
      };
    }
    if (method === "Target.attachToTarget") {
      return { sessionId: params.targetId === "target-a" ? "session-a" : "session-b" };
    }
    if (method === "Target.closeTarget") {
      return { success: true };
    }
    if (method === "Page.getFrameTree") {
      return {
        frameTree: {
          frame: {
            id: sessionId === "session-a" ? "frame-a" : "frame-b",
            loaderId: sessionId === "session-a" ? "loader-a" : "loader-b",
          },
        },
      };
    }
    if (method === "Runtime.evaluate") {
      return { result: { value: sessionId === "session-b" } };
    }
    if (method === "DOM.resolveNode") return { object: { objectId: "file-object" } };
    if (method === "Runtime.callFunctionOn") return { result: { value: "file" } };
    return {};
  }
}

class DelayedTargetCloseConnection extends FakeConnection {
  constructor(closurePollsBeforeRemoval = 2) {
    super();
    this.closingTargetId = null;
    this.closurePolls = 0;
    this.closurePollsBeforeRemoval = closurePollsBeforeRemoval;
  }

  async send(method, params = {}, sessionId) {
    if (method === "Target.closeTarget") {
      this.calls.push({ method, params, sessionId });
      this.closingTargetId = params.targetId;
      return { success: true };
    }
    if (method === "Target.getTargets" && this.closingTargetId) {
      this.calls.push({ method, params, sessionId });
      this.closurePolls += 1;
      const targetInfos = [
        { targetId: "target-a", type: "page", url: "https://a.test/", title: "A" },
        { targetId: "target-b", type: "page", url: "https://b.test/", title: "B" },
      ].filter((target) => (
        target.targetId !== this.closingTargetId
        || this.closurePolls < this.closurePollsBeforeRemoval
      ));
      return { targetInfos };
    }
    if (method === "Runtime.evaluate" && sessionId === "session-a" && this.closurePolls >= 2) {
      this.calls.push({ method, params, sessionId });
      return { result: { value: true } };
    }
    return super.send(method, params, sessionId);
  }
}

class SnapshotConnection extends FakeConnection {
  constructor() {
    super();
    this.loaderId = "loader-a";
    this.accessibilityName = "Save";
  }

  async send(method, params = {}, sessionId) {
    if (method === "Page.getFrameTree") {
      this.calls.push({ method, params, sessionId });
      return { frameTree: { frame: { loaderId: this.loaderId } } };
    }
    if (method === "Accessibility.getFullAXTree") {
      this.calls.push({ method, params, sessionId });
      return {
        nodes: [
          {
            nodeId: "ax-button",
            backendDOMNodeId: 103,
            ignored: false,
            role: { value: "button" },
            name: { value: this.accessibilityName },
            description: { value: "Save this form" },
            childIds: ["ax-text"],
            properties: [
              { name: "focused", value: { value: true } },
              { name: "disabled", value: { value: false } },
            ],
          },
          {
            nodeId: "ax-text",
            backendDOMNodeId: 104,
            parentId: "ax-button",
            ignored: false,
            role: { value: "StaticText" },
            name: { value: "Save" },
          },
        ],
      };
    }
    if (method === "DOMSnapshot.captureSnapshot") {
      this.calls.push({ method, params, sessionId });
      return {
        strings: [
          "#document",
          "HTML",
          "BODY",
          "BUTTON",
          "#text",
          "Save",
          "id",
          "save",
          "type",
          "submit",
          "INPUT",
          "password",
          "value",
          "top-secret",
          "https://a.test/",
          "https://frame.test/",
          "open",
        ],
        documents: [
          {
            nodes: {
              parentIndex: [-1, 0, 1, 2, 3, 2],
              nodeType: [9, 1, 1, 1, 3, 1],
              nodeName: [0, 1, 2, 3, 4, 10],
              nodeValue: [0, 0, 0, 0, 5, 0],
              backendNodeId: [100, 101, 102, 103, 104, 105],
              attributes: [[], [], [], [6, 7, 8, 9], [], [8, 11, 12, 13]],
              contentDocumentIndex: { index: [3], value: [1] },
              shadowRootType: { index: [2], value: [16] },
            },
            documentURL: 14,
            layout: {
              nodeIndex: [3],
              bounds: [[10, 20, 100, 30]],
            },
          },
          {
            nodes: {
              parentIndex: [-1],
              nodeType: [9],
              nodeName: [0],
              nodeValue: [0],
              backendNodeId: [200],
              attributes: [[]],
            },
            documentURL: 15,
            layout: { nodeIndex: [], bounds: [] },
          },
        ],
      };
    }
    return super.send(method, params, sessionId);
  }
}

class CompatibilityConnection extends FakeConnection {
  constructor() {
    super();
    this.loaderId = "loader-a";
  }

  async send(method, params = {}, sessionId) {
    if (method === "Page.getFrameTree" && sessionId === "session-a") {
      this.calls.push({ method, params, sessionId });
      return { frameTree: { frame: { id: "frame-a", loaderId: this.loaderId } } };
    }
    if (method === "Page.navigate") {
      this.calls.push({ method, params, sessionId });
      this.loaderId = "loader-next";
      return { loaderId: this.loaderId };
    }
    if (
      method === "Runtime.evaluate" &&
      (params.expression.includes("querySelector") || params.expression.includes("readyState"))
    ) {
      this.calls.push({ method, params, sessionId });
      return { result: { value: true } };
    }
    return super.send(method, params, sessionId);
  }
}

class FakeSocket extends EventTarget {
  constructor() {
    super();
    this.readyState = 1;
    this.sent = [];
  }

  send(payload) {
    this.sent.push(payload);
  }

  close() {
    this.readyState = 3;
    this.dispatchEvent(new Event("close"));
  }

  message(payload) {
    this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(payload) }));
  }
}

class DialogFaultSocket extends FakeSocket {
  constructor() {
    super();
    this.remote = new FakeConnection();
    this.holdDialog = false;
    this.dialogSent = Promise.withResolvers();
  }

  send(payload) {
    super.send(payload);
    const request = JSON.parse(payload);
    if (request.method === "Page.handleJavaScriptDialog") {
      this.lastDialog = request;
      this.dialogSent.resolve();
      if (this.holdDialog) {
        this.heldDialog = request;
        return;
      }
    }
    void this.remote.send(request.method, request.params, request.sessionId)
      .then((result) => this.message({ id: request.id, result }));
  }
}
