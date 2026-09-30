import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";

import { APP_CSP, APP_PERMISSIONS_POLICY, AppTabs, appOrigin } from "./browser-controller-apps.mjs";

function fakeConnection(targets = []) {
  const sent = [];
  let listener;
  return {
    sent,
    targets,
    windowState: "normal",
    created: 0,
    subscribe(fn) { listener = fn; return () => { listener = null; }; },
    async send(method, params, sessionId) {
      sent.push({ method, params, sessionId });
      if (method === "Target.getTargets") return { targetInfos: this.targets };
      if (method === "Browser.getWindowForTarget") return { windowId: 1 };
      if (method === "Browser.getWindowBounds") return { bounds: { windowState: this.windowState } };
      if (method === "Browser.setWindowBounds") this.windowState = params.bounds.windowState;
      if (method === "Page.getFrameTree") return { frameTree: { frame: { loaderId: this.loaderId } } };
      return method === "Target.createTarget" ? { targetId: `t${++this.created}` } : {};
    },
    async emit(message) { listener?.(message); await new Promise((r) => setImmediate(r)); },
  };
}

function fakeBrowser() {
  const browser = {
    connection: fakeConnection(),
    ensureConnection: async () => browser.connection,
    // t1 keeps session s1, as the other tests expect; later targets get their own.
    ensureTargetSession: async (_connection, targetId) => (targetId === "t1" ? "s1" : `s-${targetId}`),
  };
  return { connection: browser.connection, browser };
}

const asset = (path, body, type = "text/html") => ({ path, content_type: type, body_base64: Buffer.from(body).toString("base64") });

const navigated = (connection, loaderId, sessionId = "s1", parentId) =>
  connection.emit({ method: "Page.frameNavigated", sessionId, params: { frame: { id: "f", parentId, loaderId } } });

async function opened() {
  const { browser, connection } = fakeBrowser();
  const tabs = new AppTabs(browser);
  const result = await tabs.open({ origin_label: "todo-1", installation_id: "inst-1",
    assets: [asset("index.html", "<p>hi</p>"), asset("app.js", "1", "text/javascript")] });
  await navigated(connection, "doc-1");
  return { tabs, connection, result };
}

test("app origin labels are DNS labels", () => {
  assert.equal(appOrigin("todo-1"), "https://todo-1.app.chariox.internal");
  for (const bad of ["", "A", "a.b", "-a", "a/b"]) assert.throws(() => appOrigin(bad));
});

test("open intercepts every request, installs the bridge, and navigates to the App origin", async () => {
  const { connection, result } = await opened();
  assert.deepEqual(result, { target_id: "t1", origin: "https://todo-1.app.chariox.internal" });
  assert.deepEqual(connection.sent.map((m) => m.method), ["Target.createTarget", "Browser.getWindowForTarget",
    "Browser.getWindowBounds", "Browser.setWindowBounds", "Fetch.enable",
    "Runtime.addBinding", "Page.addScriptToEvaluateOnNewDocument", "Page.navigate"]);
  // Its own fullscreen window: page coordinates are desktop coordinates.
  assert.equal(connection.sent[0].params.newWindow, true);
  assert.equal(connection.windowState, "fullscreen");
  assert.equal(connection.sent.at(-1).params.url, "https://todo-1.app.chariox.internal/");
});

test("opening an installation again shows its one App Tab with the current assets", async () => {
  const { tabs, connection, result } = await opened();
  connection.sent.length = 0;
  const again = await tabs.open({ origin_label: "todo-1", installation_id: "inst-1",
    entry: "v2.html", assets: [asset("v2.html", "<p>v2</p>")] });
  assert.deepEqual(again, result);
  assert.deepEqual(connection.sent.map((m) => m.method), ["Target.activateTarget", "Browser.getWindowForTarget",
    "Browser.getWindowBounds", "Page.navigate"]);
  assert.equal(connection.sent.at(-1).params.url, "https://todo-1.app.chariox.internal/");
  assert.deepEqual((await tabs.takeCalls()).open_targets, ["t1"]);
  await connection.emit({ method: "Fetch.requestPaused", sessionId: "s1",
    params: { requestId: "r", request: { url: "https://todo-1.app.chariox.internal/", method: "GET" } } });
  assert.equal(Buffer.from(connection.sent.at(-1).params.body, "base64").toString(), "<p>v2</p>");
});

test("another installation or origin gets its own window, and a closed Tab a new one", async () => {
  const { tabs, connection } = await opened();
  const other = await tabs.open({ origin_label: "docs-1", installation_id: "inst-2", assets: [asset("index.html", "d")] });
  assert.equal(other.target_id, "t2");
  assert.deepEqual((await tabs.takeCalls()).open_targets, ["t1", "t2"]);
  // The first Tab is closed in the browser; opening its App again makes a new one.
  await connection.emit({ method: "Target.detachedFromTarget", params: { sessionId: "s1", targetId: "t1" } });
  connection.sent.length = 0;
  const reopened = await tabs.open({ origin_label: "todo-1", installation_id: "inst-1", assets: [asset("index.html", "x")] });
  assert.equal(reopened.target_id, "t3");
  assert.equal(connection.sent[0].method, "Target.createTarget");
});

test("bridge call ids differ between documents, so a late answer cannot match a new call", async () => {
  const { connection } = await opened();
  const bridge = connection.sent.find((m) => m.method === "Page.addScriptToEvaluateOnNewDocument").params.source;
  const firstIds = () => {
    const sent = [];
    const context = vm.createContext({ crypto: globalThis.crypto, __charioxAppCall: (payload) => sent.push(JSON.parse(payload).id) });
    vm.runInContext(bridge, context);
    vm.runInContext("chariox.call('list_todos')", context);
    return sent[0];
  };
  const [a, b] = [firstIds(), firstIds()];
  assert.match(a, /^[0-9a-f-]{36}:1$/);
  assert.notEqual(a, b);
});

test("rejects unsafe asset paths and missing entries", async () => {
  const { browser } = fakeBrowser();
  const tabs = new AppTabs(browser);
  for (const assets of [[asset("../x", "")], [asset("a//b", "")], [asset("app.js", "")], []]) {
    await assert.rejects(tabs.open({ origin_label: "a", installation_id: "i", assets }));
  }
});

test("serves verified assets with CSP and blocks everything else", async () => {
  const { connection } = await opened();
  connection.sent.length = 0;
  const pause = (requestId, url, method = "GET") => connection.emit({ method: "Fetch.requestPaused", sessionId: "s1",
    params: { requestId, request: { url, method } } });
  await pause("r1", "https://todo-1.app.chariox.internal/");
  await pause("r2", "https://todo-1.app.chariox.internal/app.js?v=1");
  await pause("r3", "https://todo-1.app.chariox.internal/missing");
  await pause("r4", "https://evil.test/steal");
  await pause("r5", "https://todo-1.app.chariox.internal/", "POST");
  const [root, js, missing, other, post] = connection.sent;
  assert.equal(root.method, "Fetch.fulfillRequest");
  assert.equal(Buffer.from(root.params.body, "base64").toString(), "<p>hi</p>");
  assert.ok(root.params.responseHeaders.some((h) => h.name === "Content-Security-Policy" && h.value === APP_CSP));
  assert.ok(root.params.responseHeaders.some((h) => h.name === "Permissions-Policy" && h.value === APP_PERMISSIONS_POLICY));
  assert.equal(js.params.responseCode, 200);
  assert.equal(missing.params.responseCode, 404);
  assert.deepEqual([other.method, other.params.errorReason], ["Fetch.failRequest", "BlockedByClient"]);
  assert.equal(post.method, "Fetch.failRequest");
});

test("closes popups opened by an App tab", async () => {
  const { connection } = await opened();
  connection.sent.length = 0;
  await connection.emit({ method: "Target.targetCreated", params: { targetInfo: { targetId: "p", openerId: "t1" } } });
  await connection.emit({ method: "Target.targetCreated", params: { targetInfo: { targetId: "q" } } });
  assert.deepEqual(connection.sent, [{ method: "Target.closeTarget", params: { targetId: "p" }, sessionId: undefined }]);
});

test("queues bridge calls bound to the installation and resolves responses", async () => {
  const { tabs, connection } = await opened();
  const bind = (payload, name = "__charioxAppCall") => connection.emit({ method: "Runtime.bindingCalled", sessionId: "s1", params: { name, payload } });
  await bind(JSON.stringify({ id: "1", method: "list_todos", params: { open_only: true } }));
  await bind("not json");
  await bind(JSON.stringify({ id: "2", method: "x" }), "other");
  assert.deepEqual((await tabs.takeCalls()).calls, [{ installation_id: "inst-1", target_id: "t1",
    document_id: "doc-1", call_id: "1", method: "list_todos", params: { open_only: true } }]);
  assert.deepEqual((await tabs.takeCalls()).calls, []);
  connection.sent.length = 0;
  await tabs.respond({ target_id: "t1", call_id: "1", result: { todos: [] } });
  assert.equal(connection.sent[0].method, "Runtime.evaluate");
  assert.equal(connection.sent[0].params.expression, 'globalThis.__charioxAppResolve("1", true, {"todos":[]})');
  await assert.rejects(tabs.respond({ target_id: "nope", call_id: "1" }));
});

test("the App document cannot open popups or new windows and has no WebRTC", async () => {
  assert.match(APP_CSP, /sandbox allow-scripts allow-same-origin allow-forms(;|$)/);
  assert.doesNotMatch(APP_CSP, /allow-popups|allow-top-navigation|allow-downloads|allow-modals/);
  const { connection } = await opened();
  const bridge = connection.sent.find((m) => m.method === "Page.addScriptToEvaluateOnNewDocument").params.source;
  assert.match(bridge, /RTCPeerConnection/);
  assert.ok(bridge.indexOf("RTCPeerConnection") < bridge.indexOf("__charioxAppCall"), "WebRTC is removed before the bridge exists");
});

test("malformed escapes are 404s and DNS prefetch is off", async () => {
  const { connection } = await opened();
  connection.sent.length = 0;
  await connection.emit({ method: "Fetch.requestPaused", sessionId: "s1",
    params: { requestId: "bad", request: { url: "https://todo-1.app.chariox.internal/%E0%A4%A", method: "GET" } } });
  assert.equal(connection.sent[0].params.responseCode, 404);
  assert.ok(connection.sent[0].params.responseHeaders.some((h) => h.name === "X-DNS-Prefetch-Control" && h.value === "off"));
  assert.ok(connection.sent[0].params.responseHeaders.some((h) => h.name === "Permissions-Policy" && h.value === APP_PERMISSIONS_POLICY));
});

test("App pages may not capture the screen, go fullscreen or reach devices", () => {
  for (const feature of ["display-capture", "fullscreen", "camera", "microphone", "geolocation", "clipboard-read", "usb"]) {
    assert.ok(APP_PERMISSIONS_POLICY.split(", ").includes(`${feature}=()`), feature);
  }
});

test("polls report open App targets so closed views can be dropped", async () => {
  const { tabs, connection } = await opened();
  assert.deepEqual((await tabs.takeCalls()).open_targets, ["t1"]);
  await connection.emit({ method: "Target.detachedFromTarget", params: { sessionId: "s1" } });
  assert.deepEqual((await tabs.takeCalls()).open_targets, []);
});

test("a CDP reconnect drops unconfined App Tabs and closes unowned App-origin Tabs", async () => {
  const { browser } = fakeBrowser();
  const tabs = new AppTabs(browser);
  await tabs.open({ origin_label: "todo-1", installation_id: "inst-1", assets: [asset("index.html", "x")] });
  assert.deepEqual((await tabs.takeCalls()).open_targets, ["t1"]);
  // The socket dropped and another command reconnected: interception is gone.
  browser.connection = fakeConnection([
    { targetId: "t1", url: "https://todo-1.app.chariox.internal/" },
    { targetId: "user", url: "https://example.test/" },
  ]);
  assert.deepEqual((await tabs.takeCalls()).open_targets, []);
  assert.deepEqual(browser.connection.sent.filter((m) => m.method === "Target.closeTarget").map((m) => m.params.targetId), ["t1"]);
  await assert.rejects(tabs.respond({ target_id: "t1", call_id: "1", result: null }));
});

test("a reconnect sweeps App-origin Tabs without waiting for an App command", async () => {
  const { browser } = fakeBrowser();
  new AppTabs(browser);
  const connection = fakeConnection([{ targetId: "left", url: "https://a1.app.chariox.internal/" }]);
  browser.connection = connection;
  browser.onConnected(connection);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(connection.sent.filter((m) => m.method === "Target.closeTarget").map((m) => m.params.targetId), ["left"]);
});

test("App pages lay out beside the panel only when the kernel sizes it", async () => {
  const { browser, connection } = fakeBrowser();
  const metrics = { width: 900, height: 800, deviceScaleFactor: 1, mobile: false, screenWidth: 1280, screenHeight: 800 };
  let panel = null;
  browser.appMetrics = () => panel;
  const tabs = new AppTabs(browser);
  await tabs.open({ origin_label: "todo-1", installation_id: "inst-1", assets: [asset("index.html", "<p>hi</p>")] });
  // A kernel without the panel: the page keeps the whole viewport.
  assert.equal(connection.sent.some((m) => m.method === "Emulation.setDeviceMetricsOverride"), false);
  assert.equal((await tabs.takeCalls()).app_panels, false);
  panel = metrics;
  await tabs.open({ origin_label: "docs-1", installation_id: "inst-2", assets: [asset("index.html", "<p>hi</p>")] });
  const override = connection.sent.find((m) => m.method === "Emulation.setDeviceMetricsOverride");
  assert.deepEqual(override, { method: "Emulation.setDeviceMetricsOverride", params: metrics, sessionId: "s-t2" });
  // Before navigation, so the App's first layout already leaves the panel free.
  const sent = connection.sent.map((m) => m.method);
  assert.ok(sent.lastIndexOf("Emulation.setDeviceMetricsOverride") < sent.lastIndexOf("Page.navigate"));
  assert.equal((await tabs.takeCalls()).app_panels, true);
});

test("polls keep every App window fullscreen", async () => {
  const { tabs, connection } = await opened();
  connection.windowState = "normal";
  await tabs.takeCalls();
  assert.equal(connection.windowState, "fullscreen");
});

test("the bridge asks the kernel for a panel placement; it never exposes the panel", async () => {
  const { connection } = await opened();
  const bridge = connection.sent.find((m) => m.method === "Page.addScriptToEvaluateOnNewDocument").params.source;
  assert.match(bridge, /set\(layout\) \{ return request\("chariox\.panel", layout \?\? \{\}\); \}/);
  assert.match(bridge, /get\(\) \{ return request\("chariox\.panel", \{\}\); \}/);
});

test("the kernel resizes an App page beside its panel", async () => {
  const { browser, connection } = fakeBrowser();
  browser.appMetrics = (page) => page && { width: page.width, height: page.height, deviceScaleFactor: 1, mobile: false };
  const tabs = new AppTabs(browser);
  await tabs.open({ origin_label: "todo-1", installation_id: "inst-1", assets: [asset("index.html", "<p>hi</p>")],
    page: { width: 1280, height: 540 } });
  const overrides = () => connection.sent.filter((m) => m.method === "Emulation.setDeviceMetricsOverride").map((m) => m.params)
  assert.deepEqual(overrides().at(-1), { width: 1280, height: 540, deviceScaleFactor: 1, mobile: false });
  // Minimized: the page keeps all but a bar.
  await tabs.layout({ target_id: "t1", page: { width: 1280, height: 768 } });
  assert.deepEqual(overrides().at(-1), { width: 1280, height: 768, deviceScaleFactor: 1, mobile: false });
  await assert.rejects(tabs.layout({ target_id: "nope", page: { width: 1, height: 1 } }));
});

test("reload serves the new generation's assets to the same Tab and reloads it", async () => {
  const { tabs, connection } = await opened();
  await assert.rejects(tabs.reload({ target_id: "other", assets: [asset("index.html", "x")] }));
  await assert.rejects(tabs.reload({ target_id: "t1", assets: [asset("app.js", "2", "text/javascript")] }));
  assert.deepEqual(await tabs.reload({ target_id: "t1", assets: [asset("index.html", "<p>v2</p>")] }), { target_id: "t1" });
  assert.deepEqual(connection.sent.at(-1), { method: "Page.reload", params: { ignoreCache: true }, sessionId: "s1" });
  const app = [...tabs.apps.values()][0];
  assert.equal(Buffer.from(app.assets.get("index.html").body, "base64").toString(), "<p>v2</p>");
  assert.equal(app.assets.has("app.js"), false);
});

test("a call from a document that is gone is not run, and its answer is not delivered", async () => {
  const { tabs, connection } = await opened();
  const bind = (id) => connection.emit({ method: "Runtime.bindingCalled", sessionId: "s1",
    params: { name: "__charioxAppCall", payload: JSON.stringify({ id, method: "slow" }) } });
  await bind("a:1");
  let poll = await tabs.takeCalls();
  assert.deepEqual(poll.calls.map((call) => [call.call_id, call.document_id]), [["a:1", "doc-1"]]);
  assert.deepEqual(poll.documents, { t1: "doc-1" });
  // A frame inside the page navigating is not a new document.
  await navigated(connection, "frame-doc", "s1", "f0");
  // A call queued just before the Tab reloads is dropped with its document.
  await bind("a:2");
  await navigated(connection, "doc-2");
  poll = await tabs.takeCalls();
  assert.deepEqual(poll.calls, []);
  assert.deepEqual(poll.documents, { t1: "doc-2" });
  connection.sent.length = 0;
  assert.deepEqual(await tabs.respond({ target_id: "t1", call_id: "a:1", result: 1 }), { delivered: false });
  assert.deepEqual(connection.sent, []);
  await bind("b:1");
  const [call] = (await tabs.takeCalls()).calls;
  assert.equal(call.document_id, "doc-2");
  assert.deepEqual(await tabs.respond({ target_id: "t1", call_id: "b:1", result: 1 }), { delivered: true });
  assert.equal(connection.sent.at(-1).method, "Runtime.evaluate");
  // Each call is answered once.
  assert.deepEqual(await tabs.respond({ target_id: "t1", call_id: "b:1", result: 1 }), { delivered: false });
  // A closed Tab reports no document; its queued calls are dropped too.
  await bind("b:2");
  await connection.emit({ method: "Target.detachedFromTarget", params: { sessionId: "s1" } });
  poll = await tabs.takeCalls();
  assert.deepEqual([poll.calls, poll.open_targets, poll.documents], [[], [], {}]);
});

test("a call made before any navigation was seen asks the Tab for its document", async () => {
  const { browser, connection } = fakeBrowser();
  const tabs = new AppTabs(browser);
  await tabs.open({ origin_label: "todo-1", installation_id: "inst-1", assets: [asset("index.html", "x")] });
  connection.loaderId = "doc-9";
  await connection.emit({ method: "Runtime.bindingCalled", sessionId: "s1",
    params: { name: "__charioxAppCall", payload: JSON.stringify({ id: "1", method: "m" }) } });
  const poll = await tabs.takeCalls();
  assert.equal(poll.calls[0].document_id, "doc-9");
  assert.deepEqual(poll.documents, { t1: "doc-9" });
});
