// MD-2/MD-4: focused adapter tests, not native-browser acceptance.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { KernelBrowserHost, navigationUrl } from "./kernel-browser-host.mjs";
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
      sent.push({ method, params, session });
      if (method === "Target.getTargets") return { targetInfos: [...pages].map(([targetId, tab]) => ({ targetId, type: "page", ...tab })) };
      if (method === "Target.createTarget") { const id = `target-${++next}`; pages.set(id, { url: params.url }); return { targetId: id }; }
      if (method === "Target.closeTarget") {
        pages.delete(params.targetId);
        if (pages.size === 0 && chromium.child) chromium.child.exitCode = 0;
        return {};
      }
      if (method === "Page.captureScreenshot") return { data: Buffer.from("test-frame").toString("base64") };
      if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "frame" } } };
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
  return { host: new KernelBrowserHost(root, { chromium, browserFactory }), chromium, handlers, sent };
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

test("MD integration: focused browser input cannot impersonate a human App call; App tabs are not restored", () => using(async ({ host, sent }, root) => {
  const opened = await host.request({op:"open",url:"https://app.a.invalid/"});
  const target = host.tabs.get(opened.tab_id).target_id;
  host.browser.appTabs = {apps:new Map([["app", {targetId:target}]])};
  const command = {op:"input",tab_id:opened.tab_id,generation:opened.generation,input:{kind:"click",x:12,y:20}};
  for (const op of ["input", "navigate", "close"]) {
    await assert.rejects(host.request({...command,op,url:"https://app.a.invalid/",_agent_input:true}), /human App/);
  }
  assert(!sent.some(call=>call.method==="Input.dispatchMouseEvent"));
  await host.request(command);
  assert(sent.some(call=>call.method==="Input.dispatchMouseEvent"));
  const {readFile} = await import("node:fs/promises");
  assert.deepEqual(JSON.parse(await readFile(path.join(root,"tabs.json"),"utf8")).tabs, []);
}));

test("MD integration: stale-generation App drains cannot reach the new controller", () => using(async ({ host, sent }) => {
  await host.start();
  const before = sent.length;
  const result = await host.handle({id:"stale",method:"browser.app.calls",params:{_host_generation:host.generation-1}});
  assert.equal(result.ok,false);
  assert.equal(sent.length,before);
}));

test("MD integration: generation-bound App calls never restart a stopped browser", () => using(async ({host,chromium}) => {
  await host.start();
  const generation=host.generation;
  await host.stop();
  for (const method of ["browser.app.calls","browser.app.respond","browser.app.close"]) {
    const reply=await host.handle({id:method,method,params:{_host_generation:generation}});
    assert.equal(reply.ok,false);
    assert.equal(chromium.child,null);
    assert.equal(host.generation,generation);
  }
}));


test("MD integration: target close revokes its frame subscription", () => using(async ({host,handlers,sent}) => {
  const tab=await host.request({op:"open",url:"about:blank"});
  const stream=await host.request({op:"subscribe",tab_id:tab.tab_id,generation:tab.generation});
  await host.browser.manageTab({target_id:host.tabs.get(tab.tab_id).target_id});
  await host.reconcile();
  assert.equal(host.streams.size,0);
  assert.equal(handlers.size,0);
  assert(sent.some(call=>call.method==="Page.stopScreencast"));
  await assert.rejects(host.request({op:"poll",...stream}),/stale/);
}));
