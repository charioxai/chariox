// MD-2/MD-4: focused adapter tests, not native-browser acceptance.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import os from "node:os";
import { PassThrough } from "node:stream";
import readline from "node:readline";
import { BrowserControllerStdioServer } from "./browser-controller.mjs";
import path from "node:path";
import { KernelBrowserHost, navigationUrl } from "./kernel-browser-host.mjs";
import { candidates as macCandidates, launchEnvironment as macEnvironment } from "./kernel-browser-macos.mjs";
import { launchEnvironment as linuxEnvironment } from "./kernel-browser-linux.mjs";
import { launchArguments, HostChromium } from "./kernel-browser-process.mjs";

function fixture(root) {
  const pages = new Map();
  let next = 0;
  const handlers = new Set();
  const sent = [];
  const connection = {
    isOpen: () => true,
    subscribe: handler => { handlers.add(handler); return () => handlers.delete(handler); },
    send: async (method, params, session) => {
      if (connection.beforeSend) await connection.beforeSend(method, params, session);
      sent.push({ method, params, session });
      if (method === "Target.getTargets") return { targetInfos: [...pages].map(([targetId, tab]) => ({ targetId, type: "page", ...tab })) };
      if (method === "Target.createTarget") { const id = `target-${++next}`; pages.set(id, { url: params.url }); return { targetId: id }; }
      if (method === "Target.closeTarget") {
        pages.delete(params.targetId);
        if (pages.size === 0 && chromium.child) chromium.child.exitCode = 0;
        return {};
      }
      if (method === "Page.captureScreenshot") return { data: Buffer.from("test-frame").toString("base64") };
      if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "frame", loaderId: pages.get(session?.replace("session-", ""))?.document_id ?? `doc-${session?.replace("session-", "")}` } } };
      if (method === "Page.createIsolatedWorld") return { executionContextId: 42 };
      if (method === "Runtime.evaluate") return { result: { value: fixture.secretFocused ?? false } };
      return {};
    },
  };
  const chromium = { child: null, start: async () => { chromium.child = { exitCode: null, signalCode: null }; return "http://127.0.0.1:1"; }, stop: async () => { chromium.child = null; } };
  const browserFactory = () => ({ connection, ensureConnection: async () => connection, close: async () => {},
    reconcile: async () => ({ tabs: [...pages].map(([target_id, tab]) => ({ target_id, document_id: `doc-${target_id}`, title: "fixture", ...tab })) }),
    manageTab: async ({ target_id }) => pages.delete(target_id),
    navigate: async ({ target_id, url }) => { pages.set(target_id, { url }); },
    snapshot: async () => ({ text: "fixture" }),
    resolvePageTarget: async target => ({ connection, sessionId: `session-${target}` }),
    inputCapture: { run: async (_connection, _session, operation) => operation() },
  });
  return { host: new KernelBrowserHost(root, { chromium, browserFactory }), chromium, handlers, sent, pages, connection };
}

async function using(callback) {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-md2-test-"));
  const context = fixture(root);
  try { await callback(context, root); } finally { fixture.secretFocused = false; await context.host.stop(); await rm(root, { recursive: true, force: true }); }
}

test("MD-2: native launch keeps sandbox and uses a separate dynamic loopback profile", () => {
  const args = launchArguments("/tmp/private-profile", true);
  assert(args.includes("--remote-debugging-port=0"));
  assert(args.includes("--remote-debugging-address=127.0.0.1"));
  assert(args.includes("--user-data-dir=/tmp/private-profile"));
  assert(!args.some(arg => /no-sandbox|disable-setuid-sandbox/.test(arg)));
});
test("MD-2: URL boundary rejects local files, script URLs and credentials", () => {
  for (const url of ["file:///tmp/a", "javascript:alert(1)", "chrome://settings", "https://alice:secret@example.com"]) assert.throws(() => navigationUrl(url));
  assert.equal(navigationUrl("https://example.com"), "https://example.com/");
  assert.equal(navigationUrl("about:blank"), "about:blank");
});
test("MD-4: stable tabs survive browser crash and kernel/controller recreation", () => using(async ({ host, chromium }, root) => {
  const opened = await host.request({ op: "open", url: "http://127.0.0.1/fixture" });
  const { tab_id, generation } = opened;
  assert.equal(opened.tabs.length, 1);
  await host.request({ op: "navigate", tab_id, generation, url: "http://127.0.0.1/second" });
  chromium.child.exitCode = 1;
  const recovered = await host.request({ op: "state" });
  assert.equal(recovered.generation, generation + 1);
  assert.equal(recovered.tabs[0].tab_id, tab_id);
  assert.equal(recovered.tabs[0].url, "http://127.0.0.1/second");
  await assert.rejects(host.request({ op: "screenshot", tab_id, generation }), /stale/);
  await host.stop();
  const restarted = fixture(root);
  try {
    const state = await restarted.host.request({ op: "state" });
    assert.equal(state.tabs[0].tab_id, tab_id);
    assert.equal(state.generation, generation + 2);
    await restarted.host.request({ op: "close", tab_id, generation: state.generation });
    assert.equal((await restarted.host.request({ op: "state" })).tabs.length, 0);
  } finally { await restarted.host.stop(); }
}));
test("MD-2: latest-frame subscription is bounded and invalidated by recovery", () => using(async ({ host, chromium, handlers, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const binding = { tab_id: opened.tab_id, generation: opened.generation };
  const screenshot = await host.request({ op: "screenshot", ...binding });
  assert.equal(screenshot.mime_type, "image/png");
  const subscription = await host.request({ op: "subscribe", ...binding });
  const session = sent.find(call => call.method === "Page.startScreencast").session;
  for (let count = 0; count < 100; count++) for (const handler of handlers) handler({ method: "Page.screencastFrame", sessionId: session, params: { data: `frame-${count}`, sessionId: count } });
  const polled = await host.request({ op: "poll", ...subscription });
  assert.equal(polled.frame.sequence, 100);
  assert.equal(polled.frame.data_base64, "frame-99");
  assert.equal(sent.filter(call => call.method === "Page.screencastFrameAck").length, 100);
  await host.request({ op: "unsubscribe", ...subscription });
  assert.equal(handlers.size, 0);
  assert(sent.some(call => call.method === "Page.stopScreencast"));
  chromium.child.exitCode = 1;
  await assert.rejects(host.request({ op: "poll", ...subscription }), /stale/);
}));
test("MD-4: reconciliation retries a navigation race without recreating a tab", () => using(async ({ host, sent }) => {
  await host.request({ op: "open", url: "about:blank" });
  const reconcile = host.browser.reconcile;
  const creations = sent.filter(call => call.method === "Target.createTarget").length;
  let reads = 0;
  host.browser.reconcile = async (...args) => {
    if (++reads < 3) throw Object.assign(new Error("changed document"), { code: "stale_document_reference" });
    return reconcile(...args);
  };
  assert.equal((await host.request({ op: "state" })).tabs.length, 1);
  assert.equal(reads, 3);
  assert.equal(sent.filter(call => call.method === "Target.createTarget").length, creations);
  host.browser.reconcile = async () => { throw Object.assign(new Error("unavailable"), { code: "different_error" }); };
  await assert.rejects(host.request({ op: "state" }), /unavailable/);
}));
test("MD-5: multiple subscribers share one CDP source and acknowledgment", () => using(async ({ host, handlers, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const binding = { tab_id: opened.tab_id, generation: opened.generation };
  const first = await host.request({ op: "subscribe", ...binding });
  const second = await host.request({ op: "subscribe", ...binding });
  const starts = sent.filter(call => call.method === "Page.startScreencast");
  assert.equal(starts.length, 1);
  for (const handler of handlers) handler({ method: "Page.screencastFrame", sessionId: starts[0].session, params: { data: "frame", sessionId: 1 } });
  assert.equal(sent.filter(call => call.method === "Page.screencastFrameAck").length, 1);
  for (const subscription of [first, second]) assert.equal((await host.request({ op: "poll", ...subscription })).frame.data_base64, "frame");
  await host.request({ op: "unsubscribe", ...first });
  assert(!sent.some(call => call.method === "Page.stopScreencast"));
  await host.request({ op: "unsubscribe", ...second });
  assert.equal(sent.filter(call => call.method === "Page.stopScreencast").length, 1);
}));
test("MD-2: text input uses isolated focus checks and rejects secret fields", () => using(async ({ host, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const binding = { tab_id: opened.tab_id, generation: opened.generation };
  fixture.secretFocused = true;
  await assert.rejects(host.request({ op: "input", ...binding, input: { kind: "text", text: "never-insert" } }), /Vault/);
  assert(!sent.some(call => call.method === "Input.insertText"));
  fixture.secretFocused = false;
  await host.request({ op: "input", ...binding, input: { kind: "text", text: "fixture" } });
  assert(sent.some(call => call.method === "Input.insertText"));
  await assert.rejects(host.request({ op: "input", ...binding, input: { kind: "click", x: 1280, y: 0 } }), /viewport/);
}));

test("MD-2: closing the last user tab keeps a hidden browser target alive", () => using(async ({ host, chromium, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  await host.request({ op: "close", tab_id: opened.tab_id, generation: opened.generation });
  const state = await host.request({ op: "state" });
  assert.equal(state.tabs.length, 0);
  assert.equal(state.generation, opened.generation);
  assert.equal(chromium.child.exitCode, null);
  assert(!sent.some(call => call.method === "Target.closeTarget" && call.params.targetId === host.keepaliveTarget));
}));
test("MD-2: failed child spawn has no PID to kill or await", async () => {
  const chromium = new HostChromium("/unused");
  chromium.child = { pid: undefined, kill: () => assert.fail("no child process exists") };
  await chromium.stop();
  assert.equal(chromium.child, null);
});

test("MD-5: protection flushes old frames and masks new/retired frames across recovery", () => using(async ({ host, handlers, chromium, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const subscription = await host.request({ op: "subscribe", tab_id: opened.tab_id, generation: opened.generation });
  const session = sent.find(call => call.method === "Page.startScreencast").session;
  const emit = () => { for (const handler of handlers) handler({ method: "Page.screencastFrame", sessionId: session, params: { data: "unsafe-raw-pixels", sessionId: 1 } }); };
  emit();
  assert.equal((await host.request({ op: "poll", ...subscription })).frame.data_base64, "unsafe-raw-pixels");
  const policy = { unknown: false, values: ["synthetic-only"], targets: [] };
  await host.protect(policy);
  assert.equal((await host.request({ op: "poll", ...subscription })).frame.mime_type, "image/png");
  emit();
  await host.protect(policy); // An unchanged policy must not erase every poll.
  const protectedFrame = (await host.request({ op: "poll", ...subscription })).frame;
  assert.equal(protectedFrame.mime_type, "image/png");
  assert.notEqual(protectedFrame.data_base64, "unsafe-raw-pixels");
  const capture = await host.request({ op: "screenshot", tab_id: opened.tab_id, generation: opened.generation });
  assert.equal(capture.data_base64, protectedFrame.data_base64); // unbound mock layout => full mask
  const second = await host.request({ op: "subscribe", tab_id: opened.tab_id, generation: opened.generation });
  assert.equal((await host.request({ op: "poll", ...second })).frame.mime_type, "image/png"); // no repaint required
  chromium.child.exitCode = 1;
  await host.request({ op: "state" });
  assert(host.browser.protectedValues.has("synthetic-only"));
  await assert.rejects(host.request({ op: "poll", ...subscription }), /stale/);
}));

test("MD-5: unavailable observation policy fences captures and leaves shutdown available", () => using(async ({ host }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  await host.protect({ unknown: true, values: [], targets: [] });
  await assert.rejects(host.request({ op: "screenshot", tab_id: opened.tab_id, generation: opened.generation }), /registry/);
  assert.equal((await host.request({ op: "stop" })).state, "stopped");
}));
test("MD-5: metadata scrubs echoes and never persists a secret-bearing restore URL", () => using(async ({ host }, root) => {
  const opened = await host.request({ op: "open", url: "https://example.com/?q=synthetic-protected-value" });
  await host.protect({ unknown: false, values: ["synthetic-protected-value"], targets: [] });
  const state = await host.request({ op: "state" });
  assert(!JSON.stringify(state).includes("synthetic-protected-value"));
  const saved = await readFile(path.join(root, "tabs.json"), "utf8");
  assert(!saved.includes("synthetic-protected-value"));
  assert.equal(JSON.parse(saved).tabs[0].url, "about:blank");
  assert.equal(JSON.parse(saved).tabs[0].tab_id, opened.tab_id);
}));

 test("MD-2: native Mac discovery and launch are independent of X11", () => {
  const candidates = macCandidates({ HOME: "/Users/md-test" });
  assert(candidates.includes("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"));
  assert(candidates.includes("/Users/md-test/Applications/Chromium.app/Contents/MacOS/Chromium"));
  const environment = { HOME: "/private/drill/home", DISPLAY: ":99", XAUTHORITY: "/unused", WAYLAND_DISPLAY: "wayland-0" };
  assert.deepEqual(macEnvironment(environment), { HOME: environment.HOME });
  assert.equal(environment.DISPLAY, ":99");
  assert.throws(() => linuxEnvironment({}, 0), /normal Unix user/);
  assert.deepEqual(linuxEnvironment({ DISPLAY: ":99" }, 501), { DISPLAY: ":99" });
  const args = launchArguments("/private/drill/profile", false);
  assert(!args.some(arg => /headless|no-sandbox|password-store|mock-keychain/.test(arg)));
 });


test("MD-4 reviewer R2: native unsupported URLs restore as blank without poisoning other tabs", () => using(async ({ host, chromium, pages }, root) => {
  const opened = await host.request({ op: "open", url: "https://example.com/kept" });
  pages.set("native-settings", { url: "chrome://settings/" });
  const state = await host.request({ op: "state" });
  const native = state.tabs.find(tab => tab.url === "chrome://settings/");
  assert(native);
  const saved = JSON.parse(await readFile(path.join(root, "tabs.json"), "utf8"));
  assert.equal(saved.tabs.find(tab => tab.tab_id === native.tab_id).url, "about:blank");
  // Recover legacy registries too, before the normalization existed.
  saved.tabs.find(tab => tab.tab_id === native.tab_id).url = "chrome://settings/";
  await writeFile(path.join(root, "tabs.json"), JSON.stringify(saved));
  chromium.child.exitCode = 1;
  const recovered = await host.request({ op: "state" });
  assert.equal(recovered.tabs.find(tab => tab.tab_id === native.tab_id).url, "about:blank");
  assert.equal(recovered.tabs.find(tab => tab.tab_id === opened.tab_id).url, "https://example.com/kept");
  await host.request({ op: "close", tab_id: native.tab_id, generation: recovered.generation });
}));

test("MD-4 reviewer R3: discovery and legacy restoration stay within the durable tab limit", () => using(async ({ host, chromium, pages }, root) => {
  const opened = await host.request({ op: "open", url: "https://example.com/kept" });
  for (let i = 0; i < 130; i++) pages.set(`native-${i}`, { url: `https://example.com/${i}` });
  const state = await host.request({ op: "state" });
  assert.equal(state.tabs.length, 128);
  assert(state.tabs.some(tab => tab.tab_id === opened.tab_id));
  assert.equal(pages.size, 129); // includes the internal keepalive
  const saved = JSON.parse(await readFile(path.join(root, "tabs.json"), "utf8"));
  assert.equal(saved.tabs.length, 128);
  saved.tabs.push({ tab_id: "host-tab-legacy-excess", url: "https://example.com/excess" });
  await writeFile(path.join(root, "tabs.json"), JSON.stringify(saved));
  chromium.child.exitCode = 1;
  const recovered = await host.request({ op: "state" });
  assert.equal(recovered.tabs.length, 128);
  assert.equal(recovered.tabs[0].tab_id, opened.tab_id);
  await host.request({ op: "close", tab_id: opened.tab_id, generation: recovered.generation });
  assert.equal((await host.request({ op: "state" })).tabs.length, 127);
}));


test("MD-3 b220: cancelled pending input never inserts text", () => using(async ({ host, sent, connection }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const entered = Promise.withResolvers(), release = Promise.withResolvers();
  const cancel = new AbortController();
  connection.beforeSend = async method => { if (method === "Runtime.evaluate") { entered.resolve(); await release.promise; } };
  const pending = host.request({ op: "input", tab_id: opened.tab_id, generation: opened.generation,
    document_id: opened.tabs[0].document_id, input: { kind: "text", text: "synthetic" } }, { signal: cancel.signal });
  await entered.promise;
  cancel.abort(); release.resolve();
  await assert.rejects(pending, error => error.code === "browser_action_cancelled");
  assert(!sent.some(call => call.method === "Input.insertText"));
}));

test("MD-3 b220: revocation between key transitions prevents further physical input", () => using(async ({ host, sent, connection }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const cancel = new AbortController();
  connection.beforeSend = async (method, params) => { if (method === "Input.dispatchKeyEvent" && params.type === "keyDown") cancel.abort(); };
  await assert.rejects(host.request({ op: "input", tab_id: opened.tab_id, generation: opened.generation,
    document_id: opened.tabs[0].document_id, input: { kind: "key", key: "Enter" } }, { signal: cancel.signal }), error => error.code === "browser_action_cancelled");
  assert.equal(sent.filter(call => call.method === "Input.dispatchKeyEvent").length, 1);
}));

test("MD-3 b220: input rejects replacement documents and missing observation binding", () => using(async ({ host, sent, pages }) => {
  const opened = await host.request({ op: "open", url: "https://example.com/before" });
  const command = { op: "input", tab_id: opened.tab_id, generation: opened.generation,
    document_id: opened.tabs[0].document_id, input: { kind: "click", x: 50, y: 50 } };
  const target = host.tabs.get(opened.tab_id).target_id;
  pages.set(target, { url: "https://example.com/after", document_id: "replacement" });
  await assert.rejects(host.request(command), /document/);
  delete command.document_id;
  await assert.rejects(host.request(command), /document/);
  assert(!sent.some(call => call.method.startsWith("Input.")));
}));

test("MD-3 b220: navigation during text preparation rejects input before insert", () => using(async ({ host, sent, connection, pages }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const target = host.tabs.get(opened.tab_id).target_id;
  connection.beforeSend = async method => { if (method === "Runtime.evaluate") pages.set(target, { url: "about:blank", document_id: "replacement" }); };
  await assert.rejects(host.request({ op: "input", tab_id: opened.tab_id, generation: opened.generation,
    document_id: opened.tabs[0].document_id, input: { kind: "text", text: "synthetic" } }), /document/);
  assert(!sent.some(call => call.method === "Input.insertText"));
}));


test("MD-3 b220: production stdio cancellation reaches held host input and settles before retry", { timeout: 5000 }, () => using(async ({ host, connection, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const input = new PassThrough(), output = new PassThrough();
  const lines = readline.createInterface({ input: output }), waiters = new Map();
  lines.on("line", line => { const reply = JSON.parse(line); waiters.get(reply.id)?.resolve(reply); });
  const server = new BrowserControllerStdioServer({ input, output, handleRequest: (request, options) => host.handle(request, options) });
  const running = server.run();
  const rpc = (id, method, params) => {
    const result = Promise.withResolvers(); waiters.set(id, result);
    input.write(`${JSON.stringify({ id, method, params })}\n`);
    return result.promise;
  };
  const entered = Promise.withResolvers(), release = Promise.withResolvers();
  connection.beforeSend = async method => { if (method === "Runtime.evaluate") { entered.resolve(); await release.promise; } };
  try {
    const command = { op: "input", tab_id: opened.tab_id, generation: opened.generation,
      document_id: opened.tabs[0].document_id, input: { kind: "text", text: "synthetic" } };
    const pending = rpc(1, "host.browser", command);
    await entered.promise;
    assert.equal((await rpc(2, "browser.cancel", { request_id: 1 })).result.accepted, true);
    release.resolve();
    const settled = await pending;
    assert.equal(settled.ok, false);
    assert.equal(settled.error.code, "browser_action_cancelled");
    assert(!sent.some(call => call.method === "Input.insertText"));
    connection.beforeSend = null;
    const recovered = await host.request({ op: "state" });
    assert.equal(recovered.generation, opened.generation + 1);
    assert.equal((await rpc(3, "host.browser", { ...command, generation: recovered.generation, document_id: recovered.tabs[0].document_id })).ok, true);
    assert.equal(sent.filter(call => call.method === "Input.insertText").length, 1);
  } finally { release.resolve(); input.end(); await running; lines.close(); output.end(); }
}));


test("MD-3: one terminal/agent observation cannot rebind another terminal's stale input", () => using(async ({ host, pages, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank", observed_by: "terminal-a" });
  const target = host.tabs.get(opened.tab_id).target_id;
  pages.set(target, { url: "https://example.com/replacement", document_id: "replacement" });
  const current = await host.request({ op: "state", observed_by: "terminal-b" });
  await host.request({ op: "screenshot", focused_agent: true, tab_id: opened.tab_id, generation: opened.generation });
  const input = { op: "input", observed_by: "terminal-a", tab_id: opened.tab_id, generation: opened.generation, input: { kind: "click", x: 20, y: 20 } };
  await assert.rejects(host.request(input), /document/);
  await assert.rejects(host.request({ ...input, focused_agent: true }), /document/);
  assert(!sent.some(call => call.method.startsWith("Input.")));
  await host.request({ ...input, focused_agent: true, document_id: current.tabs[0].document_id });
  assert.equal(sent.filter(call => call.method === "Input.dispatchMouseEvent").length, 2);
}));

test("MD-3: bound capture carries the captured document and rejects a capture navigation race", () => using(async ({ host, connection, pages }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const binding = { tab_id: opened.tab_id, generation: opened.generation, bound_frames: true };
  const frame = await host.request({ op: "screenshot", ...binding });
  assert.equal(frame.document_id, opened.tabs[0].document_id);
  const tab = host.tabs.get(opened.tab_id);
  connection.beforeSend = async method => {
    if (method === "Page.captureScreenshot") pages.get(tab.target_id).document_id = "replacement-document";
  };
  await assert.rejects(host.request({ op: "screenshot", ...binding }), /moved away/);
}));

test("MD-3: bound stream captures a document-bound source rather than annotating old CDP pixels", () => using(async ({ host, handlers, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const subscription = await host.request({ op: "subscribe", tab_id: opened.tab_id, generation: opened.generation, bound_frames: true });
  const session = sent.find(call => call.method === "Page.startScreencast").session;
  for (const handler of handlers) handler({ method: "Page.screencastFrame", sessionId: session, params: { data: "unbound-old-pixels", sessionId: 42 } });
  for (let attempt = 0; attempt < 50; attempt++) {
    const { frame } = await host.request({ op: "poll", ...subscription });
    if (frame?.document_id) {
      assert.equal(frame.document_id, opened.tabs[0].document_id);
      assert.notEqual(frame.data_base64, "unbound-old-pixels");
      return;
    }
    await new Promise(resolve => setTimeout(resolve, 2));
  }
  assert.fail("bound stream frame never arrived");
}));
test('MD-DISPLAY navigation rebinds one display and still rejects old document input',()=>using(async({host,pages,connection})=>{
 const {encodePng}=await import('./kernel-browser-pixels.mjs');
 const flag=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{
  const opened=await host.request({op:'open',url:'http://127.0.0.1/one'}),target=host.tabs.get(opened.tab_id).target_id;
  const send=connection.send;
  connection.send=async(method,params,...rest)=>method==='Target.getTargetInfo'?{targetInfo:pages.get(params.targetId)}:send(method,params,...rest);
  host.screenshot=async(tab,clip)=>{
   const document=pages.get(target).document_id??`doc-${target}`;
   assert.equal(tab.document_id,document,'capture must bind current native document');
   const width=clip?clip.width*clip.scale:1280,height=clip?clip.height*clip.scale:800;
   return {...tab,generation:opened.generation,data_base64:encodePng(width,height,Buffer.alloc(width*height*4,255))};
  };
  const b=await host.request({op:'display_subscribe',tab_id:opened.tab_id,generation:opened.generation,codecs:['png'],bitrate:2_000_000,device_scale_factor:1});
  const next=sequence=>host.request({op:'screenshot',display_subscription_id:b.subscription_id,generation:b.generation,after_sequence:sequence});
  const first=(await next(0)).display_frame;
  pages.set(target,{url:'http://127.0.0.1/two',title:'two',document_id:'navigation-doc'});
  const fresh=(await next(first.sequence)).display_frame;
  assert.equal(fresh.document_id,'navigation-doc');assert.equal(fresh.kind,'png');assert.equal(fresh.sequence,first.sequence+1);
  await assert.rejects(host.request({op:'input',tab_id:opened.tab_id,generation:opened.generation,document_id:first.document_id,input:{kind:'click',x:10,y:10}}),/stale input document/);
 }finally{if(flag===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=flag}
}));
test('MD-DISPLAY scrolled viewport falls back to full protected capture',()=>using(async({host,connection})=>{
 const {encodePng,decodePng}=await import('./kernel-browser-pixels.mjs');
 const flag=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{
  const opened=await host.request({op:'open',url:'http://127.0.0.1/scroll'}),send=connection.send;
  connection.send=async(method,...args)=>method==='Page.getLayoutMetrics'?{cssVisualViewport:{pageX:0,pageY:480,scale:1}}:send(method,...args);
  const pixels=Buffer.alloc(1280*800*4,255);
  host.screenshot=async(tab,clip)=>{assert.equal(clip,null,'scrolled native crop is not a viewport rectangle');return {...tab,generation:opened.generation,data_base64:encodePng(1280,800,pixels)}};
  const b=await host.request({op:'display_subscribe',tab_id:opened.tab_id,generation:opened.generation,codecs:['png'],bitrate:2_000_000,device_scale_factor:1});
  const next=sequence=>host.request({op:'screenshot',display_subscription_id:b.subscription_id,generation:b.generation,after_sequence:sequence});
  const first=(await next(0)).display_frame;pixels[(50*1280+50)*4]=0;
  const frame=(await next(first.sequence)).display_frame;assert.equal(frame.kind,'tiles');
  const tile=frame.tiles.find(tile=>tile.x<=50&&tile.y<=50&&tile.x+tile.width>50&&tile.y+tile.height>50);
  assert.equal(decodePng(tile.data_base64).pixels[((50-tile.y)*tile.width+50-tile.x)*4],0);
 }finally{if(flag===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=flag}
}));
