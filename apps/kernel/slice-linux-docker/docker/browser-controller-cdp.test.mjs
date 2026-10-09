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

test("concurrent reads share connection initialization", async () => {
  const ready = Promise.withResolvers();
  const connection = new FakeConnection();
  let opens = 0;
  const browser = new BrowserCdpClient({ connectionFactory: async () => {
    opens += 1;
    await ready.promise;
    return connection;
  } });
  const first = browser.ensureConnection();
  const second = browser.ensureConnection();
  ready.resolve();
  const connections = await Promise.all([first, second]);
  assert.equal(opens, 1);
  assert.equal(connections[0], connections[1]);
});

test("concurrent reads share fully initialized target sessions", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.ensureConnection();
  const [first, second] = await Promise.all([
    browser.ensureTargetSession(connection, "target-a"),
    browser.ensureTargetSession(connection, "target-a"),
  ]);
  assert.equal(first, second);
  assert.equal(connection.calls.filter(call => call.method === "Target.attachToTarget").length, 1);
  assert.equal(connection.calls.filter(call => call.method === "Page.enable").length, 1);
});

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
  assert.equal(connection.calls.filter(call => call.method === "Emulation.setDeviceMetricsOverride").length, 2,
    "MP-08/MP-10 unchanged reconciliation must not resize either tab again");
  assert.deepEqual(first.viewport, viewport);
  assert.equal(
    connection.calls.filter((call) => call.method === "Target.attachToTarget").length,
    3,
    "persistent page and cookie-writing worker sessions should be reused",
  );
  assert.equal(
    connection.calls.filter((call) => call.method === "Page.enable").length,
    2,
    "each persistent target session should subscribe to Page lifecycle events once",
  );
  for (const method of [
    "Page.setLifecycleEventsEnabled",
    "Runtime.enable",
    "Inspector.enable",
  ]) {
    assert.equal(
      connection.calls.filter((call) => call.method === method).length,
      2,
      `${method} should run once for each persistent target session`,
    );
  }
  assert.equal(
    connection.calls.filter((call) => call.method === "Network.enable").length,
    3,
    "network requests must be tracked for pages and cookie-writing workers",
  );
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

test("focus is read in a controller-owned isolated world a page cannot redefine", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  await browser.reconcile(viewport);
  const worlds = connection.calls.filter((call) => call.method === "Page.createIsolatedWorld");
  assert.deepEqual(
    worlds.map((call) => [call.sessionId, call.params.frameId, call.params.worldName]),
    [["session-a", "frame-a", "chariox-controller-focus"], ["session-b", "frame-b", "chariox-controller-focus"]],
    "one world per document, reused by later polls",
  );
  const focusReads = connection.calls.filter(
    (call) => call.method === "Runtime.evaluate" && call.params.expression === "document.visibilityState === 'visible'",
  );
  assert.equal(focusReads.length, 4);
  assert.ok(focusReads.every((call) => typeof call.params.contextId === "number"), "never evaluated in the page's own world");
});

test("a Tab that closes while it is inspected drops out instead of failing the reconcile", async () => {
  const connection = new FakeConnection();
  connection.vanishOnInspect = { "session-a": "target-a" };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const reconciled = await browser.reconcile(viewport);
  assert.deepEqual(reconciled.tabs.map((tab) => tab.target_id), ["target-b"]);
  assert.equal(reconciled.focused_target_id, "target-b");
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

test("cookie writer fence drains an existing worker response before import", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  connection.emit({
    method: "Network.requestWillBeSent",
    sessionId: "worker-session-a",
    params: { requestId: "worker-cookie-response" },
  });
  let imported = false;
  const guardedImport = browser.withCookieWritersQuiesced(async () => { imported = true; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(imported, false, "a worker response remains a cookie writer until it settles");
  connection.emit({
    method: "Network.loadingFinished",
    sessionId: "worker-session-a",
    params: { requestId: "worker-cookie-response" },
  });
  await guardedImport;
  assert.equal(imported, true);
});

test("cookie writer fence closes a worker first discovered after reconcile through its session", async () => {
  const connection = new FakeConnection();
  connection.includeWorker = false;
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  connection.includeWorker = true;

  await browser.withCookieWritersQuiesced(async () => {});

  assert.equal(connection.calls.some(({ method, params }) =>
    method === "Target.attachToTarget" && params.targetId === "worker-a"), true);
  assert.equal(connection.calls.some(({ method, params, sessionId }) =>
    method === "Runtime.evaluate" && params.expression === "self.close()"
      && sessionId === "worker-session-a"), true);
  assert.equal(connection.calls.some(({ method, params }) =>
    method === "Target.closeTarget" && params.targetId === "worker-a"), false);
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

test("App pages get a narrower viewport; the panel takes the rest", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  browser.appTabs = { apps: new Map([["session-a", { targetId: "target-a" }]]) };
  const widths = () => new Map(connection.calls
    .filter((call) => call.method === "Emulation.setDeviceMetricsOverride")
    .map((call) => [call.sessionId, call.params.width]));
  await browser.reconcile(viewport, { appPanelCssWidth: 380 });
  assert.equal(widths().get("session-a"), 900);
  // A page the kernel sized (bottom panel) keeps its width and loses height.
  browser.appTabs.apps.get("session-a").page = { width: 1280, height: 460 };
  await browser.reconcile(viewport, { appPanelCssWidth: 380 });
  const last = connection.calls.filter((call) => call.method === "Emulation.setDeviceMetricsOverride" && call.sessionId === "session-a").at(-1).params;
  assert.deepEqual([last.width, last.height], [1280, 460]);
  // A size larger than the viewport is ignored.
  browser.appTabs.apps.get("session-a").page = { width: 5000, height: 460 };
  await browser.reconcile(viewport, { appPanelCssWidth: 380 });
  assert.equal(widths().get("session-a"), 900);
  browser.appTabs.apps.get("session-a").page = null;
  assert.ok([...widths()].every(([session, width]) => session === "session-a" || width === 1280));
  assert.equal(browser.appMetrics().width, 900);
  // An older kernel sends no width, and a width that leaves no page is refused.
  for (const appPanelCssWidth of [undefined, 1280]) {
    await browser.reconcile(viewport, { appPanelCssWidth });
    assert.equal(widths().get("session-a"), 1280);
    assert.equal(browser.appMetrics(), null);
  }
});

test("a reconnected browser applies the bar to windows whose ids an earlier browser used", async () => {
  // Both browsers put target-a in window 1; the second starts it clipped.
  const windowed = (state) => {
    const connection = new FakeConnection();
    connection.includeWorker = false;
    connection.windows = { 1: state };
    const send = connection.send.bind(connection);
    connection.send = async (method, params = {}, sessionId) => {
      if (method === "Browser.getWindowForTarget") return { windowId: params.targetId === "target-a" ? 1 : 2 };
      if (method === "Browser.getWindowBounds") return { bounds: { windowState: connection.windows[params.windowId] ?? "fullscreen" } };
      if (method === "Browser.setWindowBounds") connection.windows[params.windowId] = params.bounds.windowState;
      return send(method, params, sessionId);
    };
    return connection;
  };
  const first = windowed("normal");
  const second = windowed("normal");
  const connections = [first, second];
  const browser = new BrowserCdpClient({ connectionFactory: async () => connections.shift() });
  await browser.reconcile(viewport, { browserBarVisible: false });
  assert.equal(first.windows[1], "fullscreen");
  first.open = false; // Chromium was relaunched.
  await browser.reconcile(viewport, { browserBarVisible: false });
  assert.equal(second.windows[1], "fullscreen");
});

test("a failed cookie-writer fence release is never reused", async () => {
  const connection = new ReleaseFaultConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  const initialFenceArmCount = connection.calls.filter(({ method, params }) =>
    method === "Target.setAutoAttach" && params.autoAttach === true).length;

  await assert.rejects(
    browser.withCookieWritersQuiesced(async () => {}),
    { code: "browser_cookie_writer_fence_release_failed", recoveryRequired: true },
  );
  connection.failRelease = false;
  await browser.withCookieWritersQuiesced(async () => {});

  assert.equal(connection.calls.filter(({ method, params }) =>
    method === "Target.setAutoAttach" && params.autoAttach === true).length,
  initialFenceArmCount + 2);
});

test("a failed cookie-writer fence acquisition is never reused", async () => {
  const connection = new AcquisitionFaultConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  const initialFenceArmCount = connection.calls.filter(({ method, params }) =>
    method === "Target.setAutoAttach" && params.autoAttach === true).length;

  await assert.rejects(
    browser.withCookieWritersQuiesced(async () => {}),
    { code: "browser_cookie_writer_fence_failed", recoveryRequired: true },
  );
  connection.failAcquisition = false;
  await browser.withCookieWritersQuiesced(async () => {});

  assert.equal(connection.calls.filter(({ method, params }) =>
    method === "Target.setAutoAttach" && params.autoAttach === true).length,
  initialFenceArmCount + 2);
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
    connection.calls.find((call) => call.method === "Page.bringToFront"),
    { method: "Page.bringToFront", params: {}, sessionId: "session-a" },
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

test("tab activation brings the selected page to front before focus is reconciled", async () => {
  const connection = new PageActivationConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const before = await browser.reconcile(viewport);
  assert.equal(before.focused_target_id, "target-b");

  await browser.manageTab({
    target_id: "target-a",
    document_id: "loader-a",
    action: "activate",
  });

  const after = await browser.reconcile(viewport);
  assert.equal(after.focused_target_id, "target-a");
  assert.equal(
    connection.calls.some(
      (call) => call.method === "Page.bringToFront" && call.sessionId === "session-a",
    ),
    true,
    "activation must use the selected page session so a subsequent focus read sees it",
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
      states: [],
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
      states: [],
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
    statfs: async () => ({ bavail: 1024 * 1024 * 1024, bsize: 1 }),
  };
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    downloadDirectory: "/safe/downloads",
    uploadRoots: ["/safe/uploads"],
    fileSystem,
    stageUploads: async ({ files }) => ({ files, markExposed() {}, async discard() {} }),
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

test("controller cancels every active download when slice storage drops below its reserve", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    downloadDirectory: "/safe/downloads",
    minimumDownloadFreeBytes: 256 * 1024 * 1024,
    fileSystem: {
      statfs: async () => ({ bavail: 128 * 1024 * 1024, bsize: 1 }),
    },
  });
  const reconciled = await browser.reconcile(viewport);

  for (const [frameId, guid] of [["frame-a", "download-a"], ["frame-b", "download-b"]]) {
    connection.emit({
      method: "Browser.downloadWillBegin",
      params: { frameId, guid, url: "https://example.test/file", suggestedFilename: "file.bin" },
    });
  }
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    connection.calls
      .filter((call) => call.method === "Browser.cancelDownload")
      .map((call) => call.params.guid)
      .sort(),
    ["download-a", "download-b"],
  );
  connection.emit({
    method: "Browser.downloadProgress",
    params: { guid: "download-a", state: "canceled", receivedBytes: 1, totalBytes: 10 },
  });
  const events = browser.pollEvents({
    browser_generation: reconciled.browser_generation,
    cursor: reconciled.event_cursor,
    limit: 10,
  }).events;
  const canceled = events.find((event) => event.kind === "download_progress");
  assert.equal(canceled.data.cancellation_reason, "disk_pressure");
});

test("controller guards downloads before their frame is mapped to a tab", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    downloadDirectory: "/safe/downloads",
    minimumDownloadFreeBytes: 256 * 1024 * 1024,
    fileSystem: {
      statfs: async () => ({ bavail: 128 * 1024 * 1024, bsize: 1 }),
    },
  });
  const reconciled = await browser.reconcile(viewport);

  connection.emit({
    method: "Browser.downloadWillBegin",
    params: {
      frameId: "frame-not-yet-observed",
      guid: "download-before-frame-map",
      url: "https://example.test/immediate-download",
      suggestedFilename: "immediate.bin",
    },
  });
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    connection.calls
      .filter((call) => call.method === "Browser.cancelDownload")
      .map((call) => call.params.guid),
    ["download-before-frame-map"],
  );
  const started = browser.pollEvents({
    browser_generation: reconciled.browser_generation,
    cursor: reconciled.event_cursor,
    limit: 10,
  }).events.find((event) => event.kind === "download_started");
  assert.equal(started.target_id, null, "optional attribution must not gate disk safety");
});

test("a download arriving during disk-pressure cancellation receives a follow-up check", async () => {
  const connection = new FakeConnection();
  const releaseFirstCancellation = Promise.withResolvers();
  const send = connection.send.bind(connection);
  connection.send = async (method, params = {}, sessionId) => {
    const result = await send(method, params, sessionId);
    if (method === "Browser.cancelDownload" && params.guid === "download-a") {
      await releaseFirstCancellation.promise;
    }
    return result;
  };
  const browser = new BrowserCdpClient({
    connectionFactory: async () => connection,
    downloadDirectory: "/safe/downloads",
    minimumDownloadFreeBytes: 256 * 1024 * 1024,
    fileSystem: {
      statfs: async () => ({ bavail: 128 * 1024 * 1024, bsize: 1 }),
    },
  });
  await browser.reconcile(viewport);

  connection.emit({
    method: "Browser.downloadWillBegin",
    params: {
      frameId: "frame-a",
      guid: "download-a",
      url: "https://example.test/a",
      suggestedFilename: "a.bin",
    },
  });
  while (!connection.calls.some(
    (call) => call.method === "Browser.cancelDownload" && call.params.guid === "download-a",
  )) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  connection.emit({
    method: "Browser.downloadWillBegin",
    params: {
      frameId: "frame-b",
      guid: "download-b",
      url: "https://example.test/b",
      suggestedFilename: "b.bin",
    },
  });
  releaseFirstCancellation.resolve();
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    connection.calls
      .filter((call) => call.method === "Browser.cancelDownload")
      .map((call) => call.params.guid),
    ["download-a", "download-b"],
  );
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
  const request = (method, params) => handleBrowserControllerRequest(
    { id: ++id, method, params },
    {
      browser,
      resourceInventory: async () => ({
        browser_ids: ["browser-pid-fixture"],
        profile_ids: ["profile-sha256-fixture"],
      }),
    },
  );
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

test("any command's new controller connection sweeps leftover App Tabs", async () => {
  const connection = new FakeConnection();
  connection.extraTargets = [{ targetId: "app-left", type: "page", url: "https://a1.app.chariox.internal/", title: "App" }];
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const response = await handleBrowserControllerRequest({ id: 1, method: "browser.reconcile", params: { viewport } }, {
    browser, resourceInventory: async () => ({ browser_ids: ["browser-pid-fixture"], profile_ids: ["profile-sha256-fixture"] }),
  });
  assert.equal(response.ok, true, JSON.stringify(response.error));
  assert.ok(connection.closedTargetIds.has("app-left"), "the non-App command's connection ran the App sweep");
  assert.ok(!connection.closedTargetIds.has("target-a"));
  // The sweep finishes before the command lists Tabs: a restored App window
  // is never inspected while it closes (a slice restart's first reconcile).
  assert.deepEqual(response.result.tabs.map((tab) => tab.target_id), ["target-a", "target-b"]);
  assert.ok(!connection.calls.some((call) => call.method === "Target.attachToTarget" && call.params.targetId === "app-left"));
});

test("a command on a session that detaches fails at once, not at its timeout", async () => {
  const socket = new FakeSocket();
  const connection = new CdpConnection(socket, 60_000);
  const closing = connection.send("Page.enable", {}, "session-a");
  const other = connection.send("Page.enable", {}, "session-b");
  socket.message({ method: "Target.detachedFromTarget", params: { sessionId: "session-a", targetId: "target-a" } });
  await assert.rejects(closing, (error) => error.code === "browser_cdp_command_failed" && /Page\.enable: Target closed/.test(error.message));
  socket.message({ id: JSON.parse(socket.sent[1]).id, result: {} });
  assert.deepEqual(await other, {});
  await connection.close();
});

test("a Tab that closes while its session starts drops out of the reconcile at once", async () => {
  // Chromium does not answer the commands of a session whose target closed.
  const socket = new ClosingTabSocket("target-a", "session-a");
  const browser = new BrowserCdpClient({ connectionFactory: async () => new CdpConnection(socket, 60_000) });
  const reconciled = await browser.reconcile(viewport);
  assert.deepEqual(reconciled.tabs.map((tab) => tab.target_id), ["target-b"]);
  await browser.close();
});

test("a Tab still listed after its session went away twice is left for the next reconcile", async () => {
  const connection = new StillClosingConnection("session-a");
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  const reconciled = await browser.reconcile(viewport);
  assert.deepEqual(reconciled.tabs.map((tab) => tab.target_id), ["target-b"]);
  connection.closingSessionId = null;
  assert.deepEqual((await browser.reconcile(viewport)).tabs.map((tab) => tab.target_id), ["target-a", "target-b"]);
});

class FakeConnection {
  constructor() {
    this.calls = [];
    this.open = true;
    this.listeners = new Set();
    this.includeWorker = true;
    this.closedTargetIds = new Set();
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
    if (method === "Emulation.setDeviceMetricsOverride" && this.vanishOnInspect?.[sessionId]) {
      const targetId = this.vanishOnInspect[sessionId];
      delete this.vanishOnInspect[sessionId];
      this.closedTargetIds.add(targetId);
      throw new Error(`Emulation.setDeviceMetricsOverride: Session with given id not found.`);
    }
    if (method === "Target.setDiscoverTargets" && this.failDiscovery) {
      throw new Error("discovery failed");
    }
    if (method === "Target.getTargets") {
      return {
        targetInfos: [
          { targetId: "target-a", type: "page", url: "https://a.test/", title: "A" },
          ...(this.includeWorker
            ? [{ targetId: "worker-a", type: "worker", url: "https://a.test/worker.js" }]
            : []),
          { targetId: "target-b", type: "page", url: "https://b.test/", title: "B" },
          ...(this.extraTargets ?? []),
        ].filter(({ targetId }) => !this.closedTargetIds.has(targetId)),
      };
    }
    if (method === "Target.attachToTarget") {
      const sessions = {
        "target-a": "session-a",
        "target-b": "session-b",
        "worker-a": "worker-session-a",
        ...Object.fromEntries((this.extraTargets ?? []).map(({ targetId }) => [targetId, `session-${targetId}`])),
      };
      return { sessionId: sessions[params.targetId] };
    }
    if (method === "Target.closeTarget") {
      this.closedTargetIds.add(params.targetId);
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
    if (method === "Page.createIsolatedWorld") {
      this.isolatedWorlds = (this.isolatedWorlds ?? 0) + 1;
      return { executionContextId: 100 + this.isolatedWorlds };
    }
    if (method === "Runtime.evaluate") {
      if (sessionId === "worker-session-a" && params.expression === "self.close()") {
        this.closedTargetIds.add("worker-a");
      }
      return { result: { value: sessionId === "session-b" } };
    }
    if (method === "DOM.resolveNode") return { object: { objectId: "file-object" } };
    if (method === "Runtime.callFunctionOn") return { result: { value: "file" } };
    return {};
  }
}

class StillClosingConnection extends FakeConnection {
  constructor(closingSessionId) {
    super();
    this.closingSessionId = closingSessionId;
  }

  async send(method, params = {}, sessionId) {
    if (method === "Emulation.setDeviceMetricsOverride" && sessionId === this.closingSessionId) {
      this.calls.push({ method, params, sessionId });
      throw new Error(`${method}: Session with given id not found.`);
    }
    return super.send(method, params, sessionId);
  }
}

class PageActivationConnection extends FakeConnection {
  constructor() {
    super();
    this.focusedTargetId = "target-b";
  }

  async send(method, params = {}, sessionId) {
    if (method === "Page.bringToFront") {
      this.focusedTargetId = sessionId === "session-a" ? "target-a" : "target-b";
      return super.send(method, params, sessionId);
    }
    if (method === "Runtime.evaluate" && params.expression === "document.visibilityState === 'visible'") {
      this.calls.push({ method, params, sessionId });
      return {
        result: {
          value: this.focusedTargetId === (sessionId === "session-a" ? "target-a" : "target-b"),
        },
      };
    }
    return super.send(method, params, sessionId);
  }
}

class ReleaseFaultConnection extends FakeConnection {
  constructor() {
    super();
    this.failRelease = true;
  }

  async send(method, params = {}, sessionId) {
    if (this.failRelease && method === "Page.setWebLifecycleState" && params.state === "active") {
      this.calls.push({ method, params, sessionId });
      throw new Error("fixture release failure");
    }
    return super.send(method, params, sessionId);
  }
}

class AcquisitionFaultConnection extends FakeConnection {
  constructor() {
    super();
    this.failAcquisition = true;
  }

  async send(method, params = {}, sessionId) {
    if (this.failAcquisition && method === "Page.stopLoading") {
      this.calls.push({ method, params, sessionId });
      throw new Error("fixture acquisition failure");
    }
    return super.send(method, params, sessionId);
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

class ClosingTabSocket extends FakeSocket {
  constructor(targetId, sessionId) {
    super();
    this.remote = new FakeConnection();
    this.targetId = targetId;
    this.sessionId = sessionId;
  }

  send(payload) {
    super.send(payload);
    const request = JSON.parse(payload);
    if (request.method === "Page.enable" && request.sessionId === this.sessionId) {
      this.remote.closedTargetIds.add(this.targetId);
      this.message({ method: "Target.detachedFromTarget", params: { sessionId: this.sessionId, targetId: this.targetId } });
      return;
    }
    void this.remote.send(request.method, request.params, request.sessionId)
      .then((result) => this.message({ id: request.id, result }));
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

 test("canonical display refusal prevents CDP layout changes", async () => {
  let connects = 0;
  const browser = new BrowserCdpClient({
    applyCanonicalDisplay: async () => { throw new Error("physical display refused"); },
    connectionFactory: async () => { connects += 1; return new FakeConnection(); },
  });
  await assert.rejects(browser.reconcile(viewport), /physical display refused/);
  assert.equal(connects, 0);
  assert.equal(browser.appViewport, undefined);
});

test("closing the final page creates a blank page before closing the window", async () => {
  const browser = new BrowserCdpClient();
  const calls = [];
  const connection = { async send(method, params) {
    calls.push([method, params]);
    if (method === "Target.getTargets") return { targetInfos: [{type:"page",targetId:"last"}] };
    return {success:true,targetId:"blank"};
  }};
  await browser.closePageTarget(connection, "last");
  assert.deepEqual(calls, [
    ["Target.getTargets", undefined],
    ["Target.createTarget", {url:"about:blank"}],
    ["Target.closeTarget", {targetId:"last"}],
  ]);
});

test("failed replacement leaves the last page open and releases the close queue", async () => {
  const browser = new BrowserCdpClient();
  let failure = true;
  let closed = 0;
  const connection = { async send(method) {
    if (method === "Target.getTargets") return {targetInfos:[{type:"page",targetId:"last"}]};
    if (method === "Target.createTarget" && failure) throw new Error("cannot create page");
    if (method === "Target.closeTarget") closed++;
    return {success:true};
  }};
  await assert.rejects(browser.closePageTarget(connection, "last"), /cannot create page/);
  assert.equal(closed, 0);
  failure = false;
  await browser.closePageTarget(connection, "last");
  assert.equal(closed, 1);
});

test("concurrent closes preserve a page after the final original tab closes", async () => {
  const browser = new BrowserCdpClient();
  const pages = new Set(["a", "b"]);
  const connection = { async send(method, params) {
    await new Promise(resolve => setImmediate(resolve));
    if (method === "Target.getTargets") return {targetInfos:[...pages].map(targetId=>({type:"page",targetId}))};
    if (method === "Target.createTarget") pages.add("blank");
    if (method === "Target.closeTarget") pages.delete(params.targetId);
    assert.ok(pages.size);
    return {success:true};
  }};
  await Promise.all([browser.closePageTarget(connection,"a"),browser.closePageTarget(connection,"b")]);
  assert.deepEqual([...pages], ["blank"]);
});

// MP-08/MP-10: a partially failed resize is never an observational preflight.
test("failed viewport change invalidates concurrent reconciliation admission", async () => {
  const connection = new FakeConnection();
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  await browser.reconcile(viewport);
  assert.equal(browser.canReconcileConcurrently(viewport), true);
  const send = connection.send.bind(connection);
  connection.send = async (method, params, sessionId) => {
    if (method === "Emulation.setDeviceMetricsOverride") throw new Error("resize failed");
    return send(method, params, sessionId);
  };
  await assert.rejects(browser.reconcile({ ...viewport, css_width: 900 }), /resize failed/);
  assert.equal(browser.canReconcileConcurrently(viewport), false);
  await browser.close();
});

// The Room browser bar and App panel layout are applied with the viewport: a
// change to either is never an observational preflight.
test("browser bar and App panel changes are not admitted as concurrent reconciliation", async () => {
  const browser = new BrowserCdpClient({ connectionFactory: async () => new FakeConnection() });
  const layout = { browserBarVisible: false, appPanelCssWidth: 320 };
  await browser.reconcile(viewport, layout);
  assert.equal(browser.canReconcileConcurrently(viewport, layout), true);
  assert.equal(browser.canReconcileConcurrently(viewport, { ...layout, browserBarVisible: true }), false);
  assert.equal(browser.canReconcileConcurrently(viewport, { ...layout, appPanelCssWidth: 400 }), false);
  assert.equal(browser.canReconcileConcurrently(viewport), false);
  await browser.close();
});

test("a failed physical display apply withdraws concurrent reconciliation admission", async () => {
  let refuse = false;
  const browser = new BrowserCdpClient({
    applyCanonicalDisplay: async () => { if (refuse) throw new Error("physical display refused"); },
    connectionFactory: async () => new FakeConnection(),
  });
  await browser.reconcile(viewport);
  assert.equal(browser.canReconcileConcurrently(viewport), true);
  refuse = true;
  await assert.rejects(browser.reconcile(viewport), /physical display refused/);
  assert.equal(browser.canReconcileConcurrently(viewport), false);
  await browser.close();
});

test("isolated-world focus keeps the physical visibility captured by input emulation", async () => {
  const browser = new BrowserCdpClient({ connectionFactory: async () => new FakeConnection() });
  assert.equal((await browser.reconcile(viewport)).focused_target_id, "target-b");
  browser.inputCapture.visibilityBySession.set("session-b", { visible: false });
  assert.equal((await browser.reconcile(viewport)).focused_target_id, null);
  browser.inputCapture.visibilityBySession.delete("session-b");
  assert.equal((await browser.reconcile(viewport)).focused_target_id, "target-b");
  await browser.close();
});


test("MP-08 CDP reconciliation retains Chromium popup creation origins", async () => {
  const connection = new FakeConnection(), send = connection.send.bind(connection)
  connection.send = async (method, ...args) => {
    const result = await send(method, ...args)
    if (method === "Target.getTargets") result.targetInfos.find(target => target.targetId === "target-b").openerId = "target-a"
    return result
  }
  const browser = new BrowserCdpClient({connectionFactory: async () => connection})
  const state = await browser.reconcile(viewport)
  assert.equal(state.tabs.find(tab => tab.target_id === "target-b").opener_target_id, "target-a")
  assert(!Object.hasOwn(state.tabs.find(tab => tab.target_id === "target-a"), "opener_target_id"))
})
