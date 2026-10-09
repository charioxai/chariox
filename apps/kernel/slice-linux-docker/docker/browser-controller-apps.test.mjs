import { BrowserCdpClient } from "./browser-controller-cdp.mjs";
import assert from "node:assert/strict";
import test from "node:test";
import vm from "node:vm";
import { PassThrough } from "node:stream";

import { APP_CSP, APP_PERMISSIONS_POLICY, AppTabs, appOrigin } from "./browser-controller-apps.mjs";
import { BrowserControllerStdioServer } from "./browser-controller.mjs";
import { appPlaceholder } from "./browser-app-restore.mjs";

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
    closePageTarget: (...args) => BrowserCdpClient.prototype.closePageTarget.call(browser, ...args),
    pageCloseQueue: Promise.resolve(),
    // t1 keeps session s1, as the other tests expect; later targets get their own.
    ensureTargetSession: async (_connection, targetId) => (targetId === "t1" ? "s1" : `s-${targetId}`),
  };
  return { connection: browser.connection, browser };
}

const asset = (path, body, type = "text/html") => ({ path, content_type: type, body_base64: Buffer.from(body).toString("base64") });

const navigated = (connection, loaderId, sessionId = "s1", parentId) =>
  connection.emit({ method: "Page.frameNavigated", sessionId, params: { frame: { id: "f", parentId, loaderId } } });

test("cold restored placeholder is reused only after Fetch and bridge are installed", async () => {
  const { browser, connection } = fakeBrowser();
  const origin = appOrigin("todo-1");
  connection.targets = [{targetId:"restored", type:"page", url:appPlaceholder(origin)},
    {targetId:"other", type:"page", url:appPlaceholder(appOrigin("other"))}];
  const tabs = new AppTabs(browser);
  await tabs.reconcile();
  assert.equal(connection.sent.some(message => message.method === "Target.closeTarget"), false);
  const opened = await tabs.open({origin_label:"todo-1", installation_id:"inst-1", assets:[asset("index.html", "hi")]});
  assert.equal(opened.target_id, "restored");
  assert.ok(connection.sent.some(message => message.method === "Target.activateTarget" && message.params.targetId === "restored"));
  assert.equal(connection.sent.some(message => message.method === "Target.createTarget"), false);
  const order = connection.sent.map(message => message.method);
  for (const method of ["Fetch.enable", "Runtime.addBinding", "Page.addScriptToEvaluateOnNewDocument"]) {
    assert.ok(order.indexOf(method) < order.indexOf("Page.navigate"));
  }
  assert.deepEqual(connection.sent.at(-1), {method:"Page.navigate", params:{url:origin + "/"}, sessionId:"s-restored"});
});

async function opened() {
  const { browser, connection } = fakeBrowser();
  const tabs = new AppTabs(browser);
  const result = await tabs.open({ origin_label: "todo-1", installation_id: "inst-1",
    assets: [asset("index.html", "<p>hi</p>"), asset("app.js", "1", "text/javascript")] });
  await navigated(connection, "doc-1");
  return { tabs, connection, result };
}

test("unclaimed restore placeholders expire, while a claimed tab survives the grace period", async () => {
  const { browser, connection } = fakeBrowser();
  connection.targets = ["claimed", "duplicate", "orphan"].map(targetId => ({targetId, type:"page", url:appPlaceholder(appOrigin("todo-1"))}));
  const tabs = new AppTabs(browser, {restoreGraceMs:20});
  await tabs.reconcile();
  await tabs.open({origin_label:"todo-1", installation_id:"inst-1", assets:[asset("index.html", "hi")]});
  await new Promise(resolve => setTimeout(resolve, 80));
  assert.deepEqual(connection.sent.filter(message => message.method === "Target.closeTarget").map(message => message.params.targetId).sort(), ["duplicate", "orphan"]);
  assert.equal([...tabs.apps.values()][0].targetId, "claimed");
  assert.equal(tabs.orphanRestores.size, 0);
});

test("controller reconnect does not restart an orphan placeholder's grace period", async () => {
  const { browser, connection } = fakeBrowser();
  connection.targets = [{targetId:"orphan", type:"page", url:appPlaceholder(appOrigin("todo-1"))}];
  const tabs = new AppTabs(browser, {restoreGraceMs:30});
  await tabs.reconcile();
  const deadline = tabs.orphanRestores.get("orphan").deadline;
  browser.connection = fakeConnection(connection.targets);
  await tabs.reconcile();
  assert.equal(tabs.orphanRestores.get("orphan").deadline, deadline);
  await new Promise(resolve => setTimeout(resolve, 80));
  assert.equal(connection.sent.some(message => message.method === "Target.closeTarget"), false);
  assert.equal(browser.connection.sent.some(message => message.method === "Target.closeTarget"), true);
});

test("a placeholder restored after the first sweep is still reclaimed", async () => {
  const { browser, connection } = fakeBrowser();
  const tabs = new AppTabs(browser, {restoreGraceMs:20});
  await tabs.reconcile();
  const targetInfo = {targetId:"late", type:"page", url:appPlaceholder(appOrigin("todo-1"))};
  connection.targets.push(targetInfo);
  await connection.emit({method:"Target.targetInfoChanged", params:{targetInfo}});
  await new Promise(resolve => setTimeout(resolve, 80));
  assert.equal(connection.sent.some(message => message.method === "Target.closeTarget" && message.params.targetId === "late"), true);
});

test("app origin labels are DNS labels", () => {
  assert.equal(appOrigin("todo-1"), "https://app.todo-1.invalid");
  for (const bad of ["", "A", "a.b", "-a", "a/b"]) assert.throws(() => appOrigin(bad));
});

test("open intercepts every request, installs the bridge, and navigates to the App origin", async () => {
  const { connection, result } = await opened();
  assert.deepEqual(result, { target_id: "t1", origin: "https://app.todo-1.invalid" });
  assert.deepEqual(connection.sent.map((m) => m.method), ["Target.getTargets", "Target.createTarget", "Browser.getWindowForTarget",
    "Browser.getWindowBounds", "Browser.setWindowBounds", "Fetch.enable",
    "Runtime.addBinding", "Page.addScriptToEvaluateOnNewDocument", "Page.navigate"]);
  // Its own fullscreen window: page coordinates are desktop coordinates.
  assert.equal(connection.sent.find(m => m.method === "Target.createTarget").params.newWindow, true);
  assert.equal(connection.windowState, "fullscreen");
  assert.equal(connection.sent.at(-1).params.url, "https://app.todo-1.invalid/");
});

test("opening an installation again shows its one App Tab with the current assets", async () => {
  const { tabs, connection, result } = await opened();
  connection.sent.length = 0;
  const again = await tabs.open({ origin_label: "todo-1", installation_id: "inst-1",
    entry: "v2.html", assets: [asset("v2.html", "<p>v2</p>")] });
  assert.deepEqual(again, result);
  assert.deepEqual(connection.sent.map((m) => m.method), ["Target.activateTarget", "Browser.getWindowForTarget",
    "Browser.getWindowBounds", "Page.navigate"]);
  assert.equal(connection.sent.at(-1).params.url, "https://app.todo-1.invalid/");
  assert.deepEqual((await tabs.takeCalls()).open_targets, ["t1"]);
  await connection.emit({ method: "Fetch.requestPaused", sessionId: "s1",
    params: { requestId: "r", request: { url: "https://app.todo-1.invalid/", method: "GET" } } });
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
  assert.equal(connection.sent[1].method, "Target.createTarget");
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
  await pause("r1", "https://app.todo-1.invalid/");
  await pause("r2", "https://app.todo-1.invalid/app.js?v=1");
  await pause("r3", "https://app.todo-1.invalid/missing");
  await pause("r4", "https://evil.test/steal");
  await pause("r5", "https://app.todo-1.invalid/", "POST");
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
  assert.deepEqual(connection.sent.filter((m) => m.method !== "Target.getTargets"), [{ method: "Target.closeTarget", params: { targetId: "p" }, sessionId: undefined }]);
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

for (const counts of [[80, 0, 0, 0], [20, 20, 20, 20]]) {
test(`an 80-call backlog (${counts.join("/")}) across four views settles every promise or returns retryable APP_BUSY`, async () => {
  const { browser, connection } = fakeBrowser();
  const tabs = new AppTabs(browser);
  const contexts = new Map();
  const bindings = [];
  const send = connection.send.bind(connection);
  connection.send = async (method, params, sessionId) => {
    if (method === "Runtime.evaluate") vm.runInContext(params.expression, contexts.get(sessionId));
    return send(method, params, sessionId);
  };
  for (let i = 1; i <= 4; i++) {
    const { target_id } = await tabs.open({ origin_label: `app-${i}`, installation_id: `inst-${i}`,
      assets: [asset("index.html", "<p>backlog</p>")] });
    const sessionId = await browser.ensureTargetSession(connection, target_id);
    const context = vm.createContext({ crypto: globalThis.crypto, __charioxAppCall: (payload) => {
      bindings.push(tabs.handle(connection, { method: "Runtime.bindingCalled", sessionId,
        params: { name: "__charioxAppCall", payload } }));
    } });
    contexts.set(sessionId, context);
    const bridge = connection.sent.find((m) => m.method === "Page.addScriptToEvaluateOnNewDocument"
      && m.sessionId === sessionId).params.source;
    vm.runInContext(bridge, context);
  }
  // One burst fills the Room queue before the next kernel poll. Each view
  // keeps real bridge promises, so a silently dropped call cannot pass.
  const outcomes = [...contexts.values()].map((context, i) => vm.runInContext(`
    Promise.all(Array.from({length:${counts[i]}}, () => chariox.call("usage").then(
      value => ({ok:true,value}), error => ({ok:false,code:error.code,message:error.message}))))`, context));
  await Promise.all(bindings);
  const refusals = connection.sent.filter((m) => m.method === "Runtime.evaluate");
  assert.equal(refusals.length, 16, "overflow must answer instead of silently dropping calls");
  const batch = await tabs.takeCalls();
  assert.equal(batch.calls.length, 64);
  assert.equal(batch.open_targets.length, 4);
  for (const call of batch.calls) {
    await tabs.respond({ target_id: call.target_id, document_id: call.document_id, call_id: call.call_id, result: { usage: 1 } });
  }
  let timer;
  try {
    const results = (await Promise.race([Promise.all(outcomes), new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error("view backlog timed out")), 1000);
    })])).flat();
    assert.equal(results.length, 80);
    assert.equal(results.filter((r) => r.ok).length, 64);
    const busy = results.filter((r) => !r.ok);
    assert.equal(busy.length, 16);
    for (const error of busy) {
      assert.equal(error.code, "APP_BUSY");
      assert.match(error.message, /queue.*64.*retry after 500 ms/i);
    }
    // A later request can use the capacity freed by the poll.
    vm.runInContext("chariox.call('usage')", [...contexts.values()][0]);
    await Promise.all(bindings);
    assert.equal((await tabs.takeCalls()).calls.length, 1);
  } finally { clearTimeout(timer); }
});
}

test("a saturated App call queue refuses kernel-owned panel requests with retryable busy", async () => {
  const { tabs, connection } = await opened();
  const app = tabs.apps.get("s1");
  for (let i = 0; i < 64; i++) await tabs.enqueueCall(app, JSON.stringify({ id: String(i), method: "usage" }), "s1");
  await tabs.enqueueCall(app, JSON.stringify({ id: "panel", method: "chariox.panel", params: { rect: null } }), "s1");
  assert.match(connection.sent.at(-1).params.expression, /__charioxAppResolve\("panel", false, .*APP_BUSY/);
  assert.equal((await tabs.takeCalls()).calls.length, 64);
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
    params: { requestId: "bad", request: { url: "https://app.todo-1.invalid/%E0%A4%A", method: "GET" } } });
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
    { targetId: "t1", url: "https://app.todo-1.invalid/" },
    { targetId: "legacy", url: "https://old.app.chariox.internal/" },
    { targetId: "user", url: "https://example.test/" },
    { targetId: "reserved", url: "https://unrelated.invalid/" },
    { targetId: "lookalike", url: "https://app.a.invalid.example.test/" },
  ]);
  assert.deepEqual((await tabs.takeCalls()).open_targets, []);
  assert.deepEqual(browser.connection.sent.filter((m) => m.method === "Target.closeTarget").map((m) => m.params.targetId), ["t1", "legacy"]);
  await assert.rejects(tabs.respond({ target_id: "t1", call_id: "1", result: null }));
});

test("a reconnect sweeps App-origin Tabs without waiting for an App command", async () => {
  const { browser } = fakeBrowser();
  new AppTabs(browser);
  const connection = fakeConnection([{ targetId: "left", url: "https://app.a1.invalid/" }]);
  browser.connection = connection;
  browser.onConnected(connection);
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(connection.sent.filter((m) => m.method === "Target.closeTarget").map((m) => m.params.targetId), ["left"]);
});

test("a real browser client opening its connection sweeps App Tabs without waiting on itself", async () => {
  const connection = fakeConnection([{ targetId: "left", url: "https://app.a1.invalid/" }]);
  connection.isOpen = () => true;
  connection.close = async () => {};
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection });
  new AppTabs(browser);
  const opened = await Promise.race([
    browser.ensureConnection(),
    new Promise((resolve) => setTimeout(() => resolve("deadlocked"), 1000)),
  ]);
  assert.equal(opened, connection);
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
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(connection.windowState, "fullscreen");
});

test("a stalled window check does not hold calls or replies in the controller queue", async (t) => {
  const { tabs, connection } = await opened();
  // Finish the connection sweep before stalling only the window inspection.
  await tabs.reconcile();
  const release = Promise.withResolvers();
  const send = connection.send.bind(connection);
  let checks = 0;
  connection.send = async (method, params, sessionId) => {
    if (method === "Browser.getWindowBounds") {
      checks += 1;
      await release.promise;
    }
    return send(method, params, sessionId);
  };
  t.after(() => release.resolve());
  await connection.emit({ method: "Runtime.bindingCalled", sessionId: "s1",
    params: { name: "__charioxAppCall", payload: JSON.stringify({ id: "neighbour", method: "usage" }) } });
  let batch;
  const drain = tabs.takeCalls().then((result) => { batch = result; });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(checks, 1, "the window check really is stalled");
  assert.equal(batch?.calls[0]?.call_id, "neighbour", "dispatch must not wait for window housekeeping");
  await tabs.respond({ target_id: "t1", call_id: "neighbour", result: { at: 1 } });
  assert.equal(connection.sent.at(-1).method, "Runtime.evaluate", "the reply leaves before the window check completes");
  // Faster bridge polling must not multiply outstanding CDP checks.
  for (let n = 0; n < 10; n++) await tabs.takeCalls();
  assert.equal(checks, 1);
  release.resolve();
  await drain;
});

test("stdio drains and answers a neighbour while fullscreen inspection is stalled", async (t) => {
  const { tabs, connection } = await opened();
  await tabs.reconcile();
  const release = Promise.withResolvers();
  const send = connection.send.bind(connection);
  connection.send = async (method, params, sessionId) => {
    if (method === "Browser.getWindowBounds") await release.promise;
    return send(method, params, sessionId);
  };
  const input = new PassThrough();
  const output = new PassThrough();
  const responses = [];
  let buffer = "";
  output.on("data", (chunk) => {
    buffer += chunk.toString();
    let end;
    while ((end = buffer.indexOf("\n")) !== -1) {
      responses.push(JSON.parse(buffer.slice(0, end)));
      buffer = buffer.slice(end + 1);
    }
  });
  // Use the real request handler and serial stdio queue, with only CDP faked.
  const browser = { ensureConnection: async () => connection, appTabs: tabs };
  const running = new BrowserControllerStdioServer({ input, output, browser }).run();
  t.after(async () => {
    release.resolve();
    input.end();
    await running;
    output.end();
  });
  await connection.emit({ method: "Runtime.bindingCalled", sessionId: "s1",
    params: { name: "__charioxAppCall", payload: JSON.stringify({ id: "neighbour", method: "usage" }) } });
  input.write(`${JSON.stringify({ id: 1, method: "browser.app.calls" })}\n`);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(responses[0]?.result?.calls[0]?.call_id, "neighbour");
  input.write(`${JSON.stringify({ id: 2, method: "browser.app.respond",
    params: { target_id: "t1", call_id: "neighbour", result: { at: 1 } } })}\n`);
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(responses[1], { id: 2, ok: true, result: { delivered: true } });
});

test("faster bridge drains keep the fullscreen check cadence bounded", async (t) => {
  let now = 1000;
  t.mock.method(performance, "now", () => now);
  const { tabs, connection } = await opened();
  await tabs.takeCalls();
  await new Promise((resolve) => setImmediate(resolve));
  connection.sent.length = 0;
  for (let n = 1; n < 10; n++) {
    now += 25;
    await tabs.takeCalls();
    await new Promise((resolve) => setImmediate(resolve));
  }
  assert.equal(connection.sent.length, 0, "rapid drains must not repeat window inspection");
  now += 25;
  await tabs.takeCalls();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(connection.sent.map((message) => message.method), ["Browser.getWindowForTarget", "Browser.getWindowBounds"]);
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
  // A call the old page queued just before the reload is dropped with it.
  await connection.emit({ method: "Runtime.bindingCalled", sessionId: "s1",
    params: { name: "__charioxAppCall", payload: JSON.stringify({ id: "old", method: "list_todos" }) } });
  assert.deepEqual(await tabs.reload({ target_id: "t1", assets: [asset("index.html", "<p>v2</p>")] }), { target_id: "t1" });
  assert.deepEqual(connection.sent.at(-1), { method: "Page.reload", params: { ignoreCache: true }, sessionId: "s1" });
  assert.deepEqual((await tabs.takeCalls()).calls, []);
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

test("view host helpers only queue the shared copy/link methods, never operate the browser or clipboard", async () => {
  const { connection } = await opened();
  const bridge = connection.sent.find((m) => m.method === "Page.addScriptToEvaluateOnNewDocument").params.source;
  const sent = [];
  const context = vm.createContext({ crypto: globalThis.crypto, __charioxAppCall: payload => sent.push(JSON.parse(payload)),
    navigator: { clipboard: { writeText() { assert.fail("App view must not copy directly"); } } },
    open() { assert.fail("App view must not open directly"); } });
  vm.runInContext(bridge, context);
  vm.runInContext("chariox.host.openLink('https://example.org/a?x=%20'); chariox.host.writeClipboard('copy text')", context);
  assert.deepEqual(sent.map(({ method, params }) => ({ method, params })), [
    { method: "host.open_link", params: { url: "https://example.org/a?x=%20" } },
    { method: "host.clipboard_write", params: { text: "copy text" } },
  ]);
  assert.equal(vm.runInContext("'readClipboard' in chariox.host", context), false);
});

test("an idle drain wakes on enqueue without advancing its fallback timer", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { tabs, connection } = await opened();
  let batch;
  const drain = tabs.takeCalls().then(result => { batch = result; });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(tabs.callWaiters.size, 1);
  t.mock.timers.tick(249);
  assert.equal(batch, undefined);
  await connection.emit({ method: "Runtime.bindingCalled", sessionId: "s1",
    params: { name: "__charioxAppCall", payload: JSON.stringify({ id: "idle", method: "usage" }) } });
  await drain;
  assert.equal(batch.calls[0].call_id, "idle");
  assert.equal(tabs.callWaiters.size, 0);
  // A call queued between requests is drained immediately, without a lost wake.
  await connection.emit({ method: "Runtime.bindingCalled", sessionId: "s1",
    params: { name: "__charioxAppCall", payload: JSON.stringify({ id: "between", method: "usage" }) } });
  assert.equal((await tabs.takeCalls()).calls[0].call_id, "between");
});

test("an empty drain times out at the fallback cadence and cleans its waiter", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { tabs } = await opened();
  const drain = tabs.takeCalls();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(tabs.callWaiters.size, 1);
  t.mock.timers.tick(250);
  assert.deepEqual((await drain).calls, []);
  assert.equal(tabs.callWaiters.size, 0);
});

test("navigation keeps the enqueue wait armed and target close cancels it", async t => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { tabs, connection } = await opened();
  const navigation = tabs.takeCalls();
  await new Promise(resolve => setImmediate(resolve));
  await navigated(connection, "doc-2");
  assert.equal(tabs.callWaiters.size, 1);
  await connection.emit({ method: "Runtime.bindingCalled", sessionId: "s1",
    params: { name: "__charioxAppCall", payload: JSON.stringify({ id: "new-doc", method: "usage" }) } });
  const batch = await navigation;
  assert.equal(batch.documents.t1, "doc-2");
  assert.equal(batch.calls[0].document_id, "doc-2");
  const close = tabs.takeCalls();
  await new Promise(resolve => setImmediate(resolve));
  await connection.emit({ method: "Target.detachedFromTarget", params: { sessionId: "s1" } });
  assert.deepEqual((await close).open_targets, []);
  assert.equal(tabs.callWaiters.size, 0);
});

test("explicit App close is target-bound and drops its queued calls and interception", async () => {
  const {tabs,connection,result} = await opened();
  await connection.emit({method:"Runtime.bindingCalled",sessionId:"s1",params:{name:"__charioxAppCall",payload:JSON.stringify({id:"close-pending",method:"echo",params:{text:"queued"}})}});
  await tabs.close({target_id:result.target_id});
  assert.equal(tabs.apps.size,0);
  const drained = await tabs.takeCalls();
  assert.deepEqual(drained.open_targets,[]);
  assert.deepEqual(drained.calls,[]);
  assert(connection.sent.some(c=>c.method==="Target.closeTarget" && c.params.targetId===result.target_id));
  await assert.rejects(tabs.close({target_id:"foreign-target"}));
  assert(!connection.sent.some(c=>c.method==="Target.closeTarget" && c.params.targetId==="foreign-target"));
});


test("user-domain instances of one App have independent targets and do not adopt Room placeholders", async () => {
  const {browser,connection}=fakeBrowser();
  connection.targets=[{targetId:"restored",type:"page",url:appPlaceholder(appOrigin("a"))}];
  const tabs=new AppTabs(browser);
  const params={origin_label:"a",installation_id:"same",assets:[asset("index.html","hi")]};
  const first=await tabs.open({...params,instance_id:"user-app-a"});
  const second=await tabs.open({...params,instance_id:"user-app-b"});
  assert.notEqual(first.target_id,second.target_id);
  assert.notEqual(first.target_id,"restored");
  assert.deepEqual(await tabs.open({...params,instance_id:"user-app-a"}),first);
  await tabs.close({target_id:first.target_id});
  assert.deepEqual((await tabs.takeCalls()).open_targets,[second.target_id]);
  for (const instance_id of ["", "../other", {}, "x".repeat(129)]) {
    await assert.rejects(tabs.open({...params,instance_id}));
  }
});
