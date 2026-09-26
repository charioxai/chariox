import assert from "node:assert/strict";
import test from "node:test";

import { APP_CSP, AppTabs, appOrigin } from "./browser-controller-apps.mjs";

function fakeConnection(targets = []) {
  const sent = [];
  let listener;
  return {
    sent,
    targets,
    windowState: "normal",
    subscribe(fn) { listener = fn; return () => { listener = null; }; },
    async send(method, params, sessionId) {
      sent.push({ method, params, sessionId });
      if (method === "Target.getTargets") return { targetInfos: this.targets };
      if (method === "Browser.getWindowForTarget") return { windowId: 1 };
      if (method === "Browser.getWindowBounds") return { bounds: { windowState: this.windowState } };
      if (method === "Browser.setWindowBounds") this.windowState = params.bounds.windowState;
      return method === "Target.createTarget" ? { targetId: "t1" } : {};
    },
    async emit(message) { listener?.(message); await new Promise((r) => setImmediate(r)); },
  };
}

function fakeBrowser() {
  const browser = {
    connection: fakeConnection(),
    ensureConnection: async () => browser.connection,
    ensureTargetSession: async () => "s1",
  };
  return { connection: browser.connection, browser };
}

const asset = (path, body, type = "text/html") => ({ path, content_type: type, body_base64: Buffer.from(body).toString("base64") });

async function opened() {
  const { browser, connection } = fakeBrowser();
  const tabs = new AppTabs(browser);
  const result = await tabs.open({ origin_label: "todo-1", installation_id: "inst-1",
    assets: [asset("index.html", "<p>hi</p>"), asset("app.js", "1", "text/javascript")] });
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
    "Browser.getWindowBounds", "Page.reload"]);
  assert.deepEqual((await tabs.takeCalls()).open_targets, ["t1"]);
  await connection.emit({ method: "Fetch.requestPaused", sessionId: "s1",
    params: { requestId: "r", request: { url: "https://todo-1.app.chariox.internal/", method: "GET" } } });
  assert.equal(Buffer.from(connection.sent.at(-1).params.body, "base64").toString(), "<p>v2</p>");
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
  assert.deepEqual((await tabs.takeCalls()).calls, [{ installation_id: "inst-1", target_id: "t1", call_id: "1",
    method: "list_todos", params: { open_only: true } }]);
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

test("a reserved panel is answered by the controller and reported only while fullscreen", async () => {
  const { tabs, connection } = await opened();
  const bind = (id, rect) => connection.emit({ method: "Runtime.bindingCalled", sessionId: "s1",
    params: { name: "__charioxAppCall", payload: JSON.stringify({ id, method: "chariox.panel", params: { rect } }) } });
  const resolved = () => connection.sent.filter((m) => m.method === "Runtime.evaluate").map((m) => m.params.expression);
  await bind("1", { x: 880.4, y: 0, width: 400, height: 800 });
  await bind("2", { x: 0, y: 0, width: 100, height: 100 });
  await bind("3", { x: -1, y: 0, width: 400, height: 800 });
  assert.deepEqual(resolved(), [
    'globalThis.__charioxAppResolve("1", true, {"reserved":true})',
    'globalThis.__charioxAppResolve("2", false, {"code":"INVALID_PANEL","message":"A panel is {x, y, width, height} in CSS pixels, at least 240x160"})',
    'globalThis.__charioxAppResolve("3", false, {"code":"INVALID_PANEL","message":"A panel is {x, y, width, height} in CSS pixels, at least 240x160"})',
  ]);
  // Panel requests never reach the kernel as App calls.
  let poll = await tabs.takeCalls();
  assert.deepEqual(poll.calls, []);
  assert.deepEqual(poll.panels, [{ target_id: "t1", x: 880, y: 0, width: 400, height: 800 }]);
  // Someone left fullscreen: no panel until the window is fullscreen again.
  connection.windowState = "normal";
  assert.deepEqual((await tabs.takeCalls()).panels, []);
  assert.equal(connection.windowState, "fullscreen");
  assert.equal((await tabs.takeCalls()).panels.length, 1);
  await bind("4", null);
  assert.equal(resolved().at(-1), 'globalThis.__charioxAppResolve("4", true, {"released":true})');
  assert.deepEqual((await tabs.takeCalls()).panels, []);
});

test("the bridge exposes panel reserve and release next to call", async () => {
  const { connection } = await opened();
  const bridge = connection.sent.find((m) => m.method === "Page.addScriptToEvaluateOnNewDocument").params.source;
  assert.match(bridge, /panel: Object\.freeze\(\{/);
  assert.match(bridge, /request\("chariox\.panel", \{ rect \}\)/);
  assert.match(bridge, /request\("chariox\.panel", \{ rect: null \}\)/);
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
