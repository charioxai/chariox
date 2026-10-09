// MD-2/MD-4: focused adapter tests, not native-browser acceptance.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import os from "node:os";
import { PassThrough } from "node:stream";
import readline from "node:readline";
import { BrowserControllerStdioServer } from "./browser-controller.mjs";
import path from "node:path";
import { encodePng, decodePng, maskPng } from "./kernel-browser-pixels.mjs";
import { KernelBrowserHost, navigationUrl } from "./kernel-browser-host.mjs";
import { candidates as macCandidates, launchEnvironment as macEnvironment } from "./kernel-browser-macos.mjs";
import { launchEnvironment as linuxEnvironment } from "./kernel-browser-linux.mjs";
import { launchArguments, HostChromium, chromiumTemporaryEnvironment } from "./kernel-browser-process.mjs";

test('MP-08 / MP-11: Chromium temporary sockets use the held private directory through a bounded path', () => {
  const environment = { TMPDIR: '/private/' + 'long/'.repeat(60), XAUTHORITY: '/private/authority' };
  const bounded = chromiumTemporaryEnvironment(environment, 5, 12345);
  assert.equal(bounded.TMPDIR, '/proc/12345/fd/5');
  assert.equal(bounded.XAUTHORITY, environment.XAUTHORITY);
  assert(!bounded.TMPDIR.includes(environment.TMPDIR));
  for (const pid of [0, 1, -1, NaN]) assert.throws(() => chromiumTemporaryEnvironment(environment, 5, pid));
});

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
      if (method === 'Target.getTargetInfo') return {targetInfo:pages.get(params.targetId)};
      if (method === "Target.createTarget") { const id = `target-${++next}`; pages.set(id, { url: params.url }); return { targetId: id }; }
      if (method === "Target.closeTarget") {
        pages.delete(params.targetId);
        if (pages.size === 0 && chromium.child) chromium.child.exitCode = 0;
        return {};
      }
      if (method === 'DOM.getDocument') return {root:{nodeId:1}};
      if (method === 'DOM.querySelectorAll') return {nodeIds:[]};
      if (method === "Page.captureScreenshot") return { data: encodePng(1280,800,Buffer.alloc(1280*800*4,255)) };
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

test('MP-08/MP-10/MP-11 agent wheel awaits Chromium even with an attached display',()=>using(async({host,connection})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.displays.set('viewer',{tab_id:tab.tab_id,observed_by:'adapter',expires:Date.now()+60000,close:async()=>{}});
 const source={attested:true,wheel:()=>assert.fail('MP-11: agent input cannot use native wheel'),close:async()=>{}};
 host.compositors.set(tab.tab_id,{source,ready:Promise.resolve(source)});
 let release,dispatched;const ack=new Promise(r=>release=r),started=new Promise(r=>dispatched=r);
 const send=connection.send;
 connection.send=async(method,params,session)=>{if(method==='Input.dispatchMouseEvent'){dispatched();await ack;return {}}return send(method,params,session)};
 let settled=false;
 const operation=host.request({op:'input',_agent_input:true,tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120}}).then(()=>settled=true);
 try{await started;await new Promise(r=>setImmediate(r));assert.equal(settled,false,'agent snapshot ordering requires the wheel ack');}finally{release();await operation}
}));

test('MP-08/MP-10/MP-11 refused viewer wheel falls back without stopping Chromium',()=>using(async({host,chromium,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 const source={attested:true,valid:()=>true,wheel:()=>false,close:async()=>{}};
 host.compositors.set(tab.tab_id,{source,ready:Promise.resolve(source)});
 host.displays.set('viewer',{tab_id:tab.tab_id,observed_by:'adapter',expires:Date.now()+60000,close:async()=>{}});
 const result=await host.handle({id:1,method:'host.browser',params:{op:'input',_display_input:true,tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120}}});
 assert.equal(result.ok,true);assert.ok(chromium.child);assert.equal(host.generation,opened.generation);
 assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,1);
}));

test('MP-08/MP-10/MP-11 an unchanged admitted native credit performs no CDP observation',()=>using(async({host,sent})=>{
 const old=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{
  const {DisplayStream}=await import('./kernel-browser-display.mjs');
  const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
  const sample={serial:1,document_id:tab.document_id,tab_id:tab.tab_id,raw:{},motion:true};
  const source={attested:true,closed:false,policy:host.protection,changedAt:performance.now(),valid:()=>true,allowed:()=>true,sample:()=>sample,close:async()=>{}};
  let parked=0;const stream=new DisplayStream({subscription_id:'empty',tab_id:tab.tab_id,observed_by:'adapter',codec:'avc1.420033',device_scale_factor:1,bitrate:8000000},{encoder:{close:async()=>{}}});
  Object.assign(stream,{document_id:tab.document_id,previous:{signature:'base'},compositorSerial:1,creditEpoch:0,refinerDocument:tab.document_id,
   refiner:{quietMs:300,request:()=>null,invalidate(){},close:async()=>{}},producer:{source,frames:[],waitReady:async()=>{parked++;},take:()=>null,feedback(){},invalidate(){},close:async()=>{}}});
  host.compositors.set(tab.tab_id,{document:tab.document_id,source,ready:Promise.resolve(source)});host.displays.set(stream.subscription_id,stream);
  const before=sent.length;
  const result=await host.request({op:'screenshot',display_subscription_id:'empty',generation:opened.generation,after_sequence:0});
  assert.equal(parked,1,'unchanged native credit parks on source readiness');assert.equal(result.frame_sent,false);assert.equal(sent.length,before,'empty native credits cannot repeat CDP observation');
 }finally{if(old===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=old}
}));

test("MP-11: retained input succeeds and leaves Chromium live", () => using(async ({ host, sent }) => {
  const opened = await host.request({ op:'open', url:'https://example.com' });
  const tab = opened.tabs[0];
  const reply = await host.handle({ id:'denied', method:'host.browser', params: {
    op:'input', tab_id:tab.tab_id, generation:opened.generation, document_id:tab.document_id,
    input:{kind:'click',x:1,y:2}, _retained_agent:true,
  } });
  assert.equal(reply.ok, true);
  assert.equal(sent.filter(({method}) => method.startsWith('Input.')).length, 2);
  assert.equal((await host.request({op:'state'})).generation, opened.generation);
}));

test("MP-11: revocation closes idle and in-flight grant streams without another holder's stream", () => using(async ({ host }) => {
  const opened = await host.request({ op: "open", url: "https://example.com" });
  const command = { op: "subscribe", tab_id: opened.tab_id, generation: opened.generation };
  const idle = await host.request({ ...command, _subscription_owner: "old-grant" });
  const retained = await host.request({ ...command, _subscription_owner: "another-grant" });
  // The kernel settles an in-flight subscribe before sending owner retirement;
  // its ID need not have reached the grant registry to be cancelled.
  await host.request({ ...command, _subscription_owner: "old-grant" });
  const result = await host.handle({ id: 99, method: "host.revoke_subscriptions", params: { subscription_ids: [], subscription_owners: ["old-grant"] } });
  assert.equal(result.ok, true);
  assert.equal(host.streams.size, 1);
  assert(host.streams.has(retained.subscription_id));
  assert(!host.streams.has(idle.subscription_id));
}));

test("MD-2: native launch keeps sandbox and a private inherited CDP pipe", () => {
  const args = launchArguments("/tmp/private-profile", true);
  assert(args.includes("--remote-debugging-pipe"));
  assert(!args.some(arg => /remote-debugging-(port|address)/.test(arg)));
  assert(args.includes("--user-data-dir=/tmp/private-profile"));
  assert(!args.some(arg => /no-sandbox|disable-setuid-sandbox/.test(arg)));
});
test('MP-08 / MP-11: owned headed Chromium enables its native accessibility tree', () => {
  assert(launchArguments('/tmp/private-profile', false, false, true).includes('--force-renderer-accessibility'));
  assert(!launchArguments('/tmp/private-profile', true, false, true).includes('--force-renderer-accessibility'));
  assert(!launchArguments('/tmp/private-profile', false).includes('--force-renderer-accessibility'));
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
  const recovered = await host.request({ op: "start" });
  assert.equal(recovered.generation, generation + 1);
  assert.equal(recovered.tabs[0].tab_id, tab_id);
  assert.equal(recovered.tabs[0].url, "http://127.0.0.1/second");
  await assert.rejects(host.request({ op: "screenshot", tab_id, generation }), error => ["user_domain_stale_epoch","user_domain_stale_reference"].includes(error.code));
  await host.stop();
  const restarted = fixture(root);
  try {
    const state = await restarted.host.request({ op: "start" });
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
  let polled;for(let attempt=0;attempt<50;attempt++){
    polled=await host.request({op:'poll',...subscription});
    if(decodePng(polled.frame.data_base64).pixels[0]===255)break;
    await new Promise(resolve=>setTimeout(resolve,5));
  }
  assert(polled.frame.sequence>0&&polled.frame.sequence<=100);
  assert.equal(polled.frame.mime_type,'image/png');assert.equal(decodePng(polled.frame.data_base64).pixels[0],255);
  assert.equal(sent.filter(call => call.method === "Page.screencastFrameAck").length, 100);
  await host.request({ op: "unsubscribe", ...subscription });
  assert.equal(handlers.size, 0);
  assert(sent.some(call => call.method === "Page.stopScreencast"));
  chromium.child.exitCode = 1;
  await assert.rejects(host.request({ op: "poll", ...subscription }), /unavailable/);
  await host.request({ op: "start" });
  await assert.rejects(host.request({ op: "poll", ...subscription }), {code:'user_domain_stale_reference'});
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
  for (const subscription of [first, second]) {
    let frame;for(let attempt=0;attempt<50;attempt++){
      frame=(await host.request({op:'poll',...subscription})).frame;
      if(decodePng(frame.data_base64).pixels[0]===255)break;
      await new Promise(resolve=>setTimeout(resolve,5));
    }
    assert.equal(frame.mime_type,'image/png');assert.equal(decodePng(frame.data_base64).pixels[0],255);
  }
  await host.request({ op: "unsubscribe", ...first });
  assert(!sent.some(call => call.method === "Page.stopScreencast"));
  await host.request({ op: "unsubscribe", ...second });
  assert.equal(sent.filter(call => call.method === "Page.stopScreencast").length, 1);
}));
test("MD-2: text input uses isolated focus checks and rejects secret fields", () => using(async ({ host, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const binding = { tab_id: opened.tab_id, generation: opened.generation };
  fixture.secretFocused = true;
  await assert.rejects(host.request({ op: "input", ...binding, input: { kind: "text", text: "never-insert" } }), error => error.code === "user_domain_sensitive_requires_focus");
  assert(!sent.some(call => call.method === "Input.insertText"));
  fixture.secretFocused = false;
  await host.request({ op: "input", ...binding, input: { kind: "text", text: "fixture" } });
  assert(sent.some(call => call.method === "Input.insertText"));
  await assert.rejects(host.request({ op: "input", ...binding, input: { kind: "click", x: 1280, y: 0 } }), /viewport/);
}));

test("MP-08 / MP-10 / MP-11: native navigation adopts the keepalive tab and reserves a background replacement", () => using(async ({ host, chromium, sent, pages }) => {
  const initial = await host.request({ op: "start" });
  const navigatedTarget = host.keepaliveTarget;
  const creations = sent.filter(call => call.method === "Target.createTarget").length;
  pages.set(navigatedTarget, { url: "https://www.libreoffice.org/", document_id: "native-navigation" });
  const state = await host.request({ op: "state" });
  assert.equal(state.tabs.length, 1);
  assert.equal(state.tabs[0].url, "https://www.libreoffice.org/");
  assert.notEqual(host.keepaliveTarget, navigatedTarget);
  const replacement = sent.filter(call => call.method === "Target.createTarget").slice(creations);
  assert.deepEqual(replacement.map(call => call.params), [{ url: "about:blank", background: true }]);
  assert.equal((await host.request({ op: "state" })).tabs[0].tab_id, state.tabs[0].tab_id);
  assert.equal(sent.filter(call => call.method === "Target.createTarget").length, creations + 1);
  await host.request({ op: "close", tab_id: state.tabs[0].tab_id, generation: state.generation });
  assert.equal((await host.request({ op: "state" })).tabs.length, 0);
  assert.equal(state.generation, initial.generation);
  assert.equal(chromium.child.exitCode, null);
  assert(pages.has(host.keepaliveTarget));
  assert(!pages.has(navigatedTarget));

}));

test('MD-DISPLAY-02 bound input checks its document before dispatch without a full preflight reconciliation', () => using(async ({host, sent, pages}) => {
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 const reconcile=host.browser.reconcile;let reconciles=0;
 host.browser.reconcile=async(...args)=>{reconciles++;return reconcile(...args)};
 host.browser.connection.beforeSend=async method=>{
  if(method==='Input.dispatchMouseEvent')assert.equal(reconciles,0,'bound physical input must not await unrelated tab reconciliation');
 };
 const command={op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'click',x:2,y:2}};
 await host.request(command);assert.equal(reconciles,1,'post-input reconciliation still discovers popups/navigation');
 const before=sent.filter(x=>x.method==='Input.dispatchMouseEvent').length;
 pages.get(host.tabs.get(tab.tab_id).target_id).document_id='replacement';
 await assert.rejects(host.request(command),{code:'user_domain_stale_reference'});
 assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,before);
}));

test('MP-08/MP-10 physical input preserves video references while retiring exact work by epoch',()=>using(async({host})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 let retired=0,wakes=0;
 host.compositors.set(tab.tab_id,{document:tab.document_id,ready:Promise.resolve(),source:{wake:()=>wakes++,close:async()=>{}}});
 host.displays.set('test-display',{tab_id:tab.tab_id,expires:Infinity,producer:{retireUnsent:()=>retired++},close:async()=>{}});
 for(const input of [{kind:'click',x:2,y:2},{kind:'text',text:'a'},{kind:'key',key:'Tab'}])
  await host.request({op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input});
 assert.equal(host.inputEpochs.get(tab.tab_id),3,'every dispatched input must retire stale exact work');
 assert.equal(retired,0,'input cannot force row IDRs for already queued normal video');
 assert.equal(wakes,6,'wake owned source before and after every physical input');
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

test("MP-08/MP-11: policy flushes old frames without masks for registration alone across recovery", () => using(async ({ host, handlers, chromium, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const subscription = await host.request({ op: "subscribe", tab_id: opened.tab_id, generation: opened.generation });
  const session = sent.find(call => call.method === "Page.startScreencast").session;
  const emit = () => { for (const handler of handlers) handler({ method: "Page.screencastFrame", sessionId: session, params: { data: "unsafe-raw-pixels", sessionId: 1 } }); };
  emit();
  assert.notEqual((await host.request({ op: "poll", ...subscription })).frame.data_base64, "unsafe-raw-pixels");
  const policy = { unknown: false, values: ["synthetic-only"], targets: [] };
  await host.protect(policy);
  assert.equal((await host.request({ op: "poll", ...subscription })).frame.mime_type, "image/png");
  emit();
  await host.protect(policy); // An unchanged policy must not erase every poll.
  const protectedFrame = (await host.request({ op: "poll", ...subscription })).frame;
  assert.equal(protectedFrame.mime_type, "image/png");
  assert.notEqual(protectedFrame.data_base64, "unsafe-raw-pixels");
  const capture = await host.request({ op: "screenshot", tab_id: opened.tab_id, generation: opened.generation });
  assert.equal(decodePng(capture.data_base64).pixels[0],255); // MP-11 registration alone leaves source pixels intact
  const second = await host.request({ op: "subscribe", tab_id: opened.tab_id, generation: opened.generation });
  assert.equal((await host.request({ op: "poll", ...second })).frame.mime_type, "image/png"); // no repaint required
  chromium.child.exitCode = 1;
  await host.request({ op: "start" });
  assert(host.browser.protectedValues.has("synthetic-only"));
  await assert.rejects(host.request({ op: "poll", ...subscription }), error => ["user_domain_stale_epoch","user_domain_stale_reference"].includes(error.code));
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
  const recovered = await host.request({ op: "start" });
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
  const recovered = await host.request({ op: "start" });
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
  await assert.rejects(host.request(command), error => error.code === "user_domain_stale_reference");
  delete command.document_id;
  await assert.rejects(host.request(command), error => error.code === "user_domain_stale_reference");
  assert(!sent.some(call => call.method.startsWith("Input.")));
}));

test("MD-3 b220: navigation during text preparation rejects input before insert", () => using(async ({ host, sent, connection, pages }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const target = host.tabs.get(opened.tab_id).target_id;
  connection.beforeSend = async method => { if (method === "Runtime.evaluate") pages.set(target, { url: "about:blank", document_id: "replacement" }); };
  await assert.rejects(host.request({ op: "input", tab_id: opened.tab_id, generation: opened.generation,
    document_id: opened.tabs[0].document_id, input: { kind: "text", text: "synthetic" } }), error => error.code === "stale_document_reference");
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
    const recovered = await host.request({ op: "start" });
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
  await assert.rejects(host.request(input), error => error.code === "user_domain_stale_reference");
  await assert.rejects(host.request({ ...input, focused_agent: true }), error => error.code === "user_domain_stale_reference");
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

test("MD integration: focused browser input cannot impersonate a human App call; App tabs are not restored", () => using(async ({ host, sent }, root) => {
  const opened = await host.request({op:"open",url:"https://app.a.invalid/"});
  const target = host.tabs.get(opened.tab_id).target_id;
  host.browser.appTabs = {apps:new Map([["app", {targetId:target}]])};
  const command = {op:"input",tab_id:opened.tab_id,generation:opened.generation,input:{kind:"click",x:12,y:20}};
  for (const op of ["input", "navigate", "close"]) {
    await assert.rejects(host.request({...command,op,url:"https://app.a.invalid/",_agent_input:true}), error => error.code === "user_domain_not_granted");
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
  await assert.rejects(host.request({op:"poll",...stream}),error => error.code === "user_domain_stale_reference");
}));


test("human Enter carries Chromium's native text event; key-up never retypes it", () => using(async ({ host, sent }) => {
  const tab = await host.request({ op: "open", url: "about:blank" });
  await host.request({ op: "input", tab_id: tab.tab_id, generation: tab.generation, input: { kind: "key", key: "Enter" } });
  assert.deepEqual(sent.filter(call => call.method === "Input.dispatchKeyEvent").map(call => call.params), [
    { type: "keyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, text: "\r", unmodifiedText: "\r" },
    { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13 },
  ]);
}));

for (const kind of ["key", "click"]) {
  test(`MD-3: ${kind} down followed by a CDP error fences uncertain physical input`, () => using(async ({ host, connection, sent, chromium }) => {
    const opened = await host.request({ op: "open", url: "about:blank" });
    let failed = false;
    connection.beforeSend = async (method, params) => {
      if (!failed && ((kind === "key" && method === "Input.dispatchKeyEvent" && params.type === "keyUp") ||
          (kind === "click" && method === "Input.dispatchMouseEvent" && params.type === "mouseReleased"))) {
        failed = true; throw new Error("fixture CDP socket error");
      }
    };
    await assert.rejects(host.request({ op: "input", ...opened,
      input: kind === "key" ? { kind, key: "Tab" } : { kind, x: 2, y: 2 } }), /CDP socket error/);
    assert(sent.some(entry => entry.params?.type === (kind === "key" ? "keyDown" : "mousePressed")));
    assert.equal(chromium.child, null, "MD-3: uncertain held input must end before another actor dispatches");
    connection.beforeSend = null;
    const recovered = await host.request({ op: "start" });
    assert.equal(recovered.generation, opened.generation + 1);
    await host.request({ op: "input", tab_id: opened.tab_id, generation: recovered.generation,
      observed_by: "terminal:next", input: { kind: "key", key: "Tab" } }).catch(async error => {
        // A new actor must observe its document before input.
        await host.request({ op: "state", observed_by: "terminal:next" });
        await host.request({ op: "input", tab_id: opened.tab_id, generation: recovered.generation,
          observed_by: "terminal:next", input: { kind: "key", key: "Tab" } });
      });
  }));
}

test("display subscription captures the current document after navigation", () => using(async ({host,connection,pages}) => {
  const original = process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;
  process.env.CHARIOX_KERNEL_BROWSER_DISPLAY = "1";
  const send = connection.send;
  connection.send = async (method,params,session) => {
    if(method === "Page.captureScreenshot") {
      const width=Math.round((params.clip?.width??1280)*(params.clip?.scale??1));
      const height=Math.round((params.clip?.height??800)*(params.clip?.scale??1));
      return {data:encodePng(width,height,Buffer.alloc(width*height*4,255))};
    }
    if(method === "Target.getTargetInfo") return {targetInfo:pages.get(params.targetId)};
    return send(method,params,session);
  };
  try {
    const opened=await host.request({op:"open",url:"about:blank"});
    const binding={tab_id:opened.tab_id,generation:opened.generation};
    const stream=await host.request({op:"display_subscribe",...binding,codecs:["png"],bitrate:8_000_000,device_scale_factor:1});
    const poll={op:"screenshot",generation:opened.generation,display_subscription_id:stream.subscription_id,after_sequence:0};
    const first=await host.request(poll);
    assert.equal(first.display_frame.document_id,opened.tabs[0].document_id);
    pages.get(host.tabs.get(opened.tab_id).target_id).document_id="next-document";
    const next=await host.request({...poll,after_sequence:first.display_frame.sequence});
    assert.equal(next.display_frame.document_id,"next-document");
    assert.ok(next.display_frame.sequence>first.display_frame.sequence);
  } finally {
    if(original===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;
    else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=original;
  }
}));

// Fail-first coverage for each of the nine live hardening gaps.
const refusalCases = [
  ['foreign tab snapshot', 'snapshot', 'not_granted'],
  ['foreign tab screenshot', 'screenshot', 'not_granted'],
  ['foreign browser region capture', 'screenshot', 'not_granted'],
  ['foreign note-window observation', 'note_selection', 'not_granted'],
  ['forged resource references', 'snapshot', 'not_granted'],
  ['stale browser input document', 'input', 'stale_reference'],
  ['stale App input document', 'input', 'stale_reference'],
  ['cross-identity capture correlation replay', 'screenshot', 'not_granted'],
  ['stale capture generation', 'screenshot', 'stale_epoch'],
];
for (const [label, op, reason] of refusalCases) test('MD434 explicit refusal: '+label, () => using(async ({host}) => {
  const opened=await host.handle({id:1,method:'host.browser',params:{op:'open',url:'https://fixture.invalid/'}});
  const generation=host.generation;
  const tab=[...host.tabs.values()][0];
  const params={op,generation,tab_id:reason==='not_granted'?'foreign-or-forged-id':tab.tab_id,observed_by:'terminal:fixture'};
  if(op==='input'){params.document_id='old-document';params.input={kind:'key',key:'Tab'};}
  if(reason==='stale_epoch')params.generation++;
  const result=await host.handle({id:2,method:'host.browser',params});
  assert.equal(result.ok,false);
  assert.equal(result.error.code,'user_domain_'+reason);
  assert.equal(result.error.message,'User-domain request refused');
  assert.ok(!JSON.stringify(result.error).includes(params.tab_id));
}));

test('MD434 page/transport errors cannot forge host policy refusals', () => using(async ({host}) => {
 await host.request({op:'open',url:'https://fixture.invalid/'});
 const tab=[...host.tabs.values()][0];
 host.browser.snapshot=async()=>{throw Object.assign(new Error('User-domain request refused'),{code:'user_domain_not_granted'});};
 const result=await host.handle({id:1,method:'host.browser',params:{op:'snapshot',tab_id:tab.tab_id,generation:host.generation}});
 assert.equal(result.error.code,'kernel_browser_failed');
}));

test("MP-11: state reads never start or recover Chromium; explicit retained start can", () => using(async ({ host, chromium, sent }) => {
  for (const retained of [true,false]) await assert.rejects(host.request({op:'state',_retained_agent:retained}), {code:'browser_unavailable'});
  assert.equal(chromium.child,null); assert.deepEqual(sent,[]);
  const opened=await host.request({op:'open',url:'https://example.com'});
  for (const stopped of [false,true]) {
    if (stopped) await host.stop(); else chromium.child.exitCode=1;
    const generation=host.generation, requests=sent.length;
    for (const retained of [true,false]) {
      const reply=await host.handle({id:'read',method:'host.browser',params:{op:'state',_retained_agent:retained}});
      assert.equal(reply.ok,false); assert.equal(reply.error.code,'browser_unavailable');
      assert.equal(host.generation,generation); assert.equal(sent.length,requests);
    }
    const recovered=await host.request({op:'start',_retained_agent:true});
    assert.equal(recovered.generation,generation+1);
    assert.equal(recovered.tabs[0].tab_id,opened.tab_id);
  }
}));

test("MP-11: mirror reads and cleanup never start or recover Chromium", () => using(async ({ host, chromium, sent }) => {
  const previous = process.env.CHARIOX_KERNEL_BROWSER_MIRROR;
  process.env.CHARIOX_KERNEL_BROWSER_MIRROR = "1";
  try {
    let launches = 0;
    const start = chromium.start;
    chromium.start = async () => { launches++; return start(); };
    const opened = await host.request({ op:'open', url:'https://example.com' });
    for (const stopped of [false, true]) {
      if (stopped) await host.stop(); else chromium.child.exitCode = 1;
      const generation = host.generation, requests = sent.length, before = launches;
      for (const op of ['mirror_subscribe', 'mirror_next', 'mirror_close']) {
        const reply = await host.handle({ id:op, method:'host.browser', params:{ op, tab_id:opened.tabs[0].tab_id, subscription_id:'s', generation } });
        assert.equal(reply.ok, false, op); assert.equal(reply.error.code, 'browser_unavailable', op);
        assert.equal(launches, before, op); assert.equal(host.generation, generation, op);
        assert.equal(sent.length, requests, `${op} must not restore tabs`);
      }
    }
  } finally {
    if (previous === undefined) delete process.env.CHARIOX_KERNEL_BROWSER_MIRROR; else process.env.CHARIOX_KERNEL_BROWSER_MIRROR = previous;
  }
}));

test('MP-11: only pre-dispatch mirror epoch refusal survives host error sanitization', () => using(async ({host,sent}) => {
  const opened=await host.request({op:'open',url:'about:blank'});
  const tab=host.tabs.get(opened.tab_id),scope='epoch-test';
  host.mirror.streams.set('epoch-test',{scope,tab_id:tab.tab_id,document_id:tab.document_id,expires:Date.now()+60000,policy:host.protection,epochs:[]});
  host.request=()=>host.mirror.resolveInput(tab,{subscription_id:'epoch-test',sequence:1,action:{kind:'key',key:'Enter'}},scope);
  const before=sent.length;
  const refused=await host.handle({id:'epoch',method:'host.browser',params:{op:'input'}});
  assert.equal(refused.ok,false);
  assert.equal(refused.error.code,'kernel_browser_failed');
  assert.equal(refused.error.message,'MP-11: stale mirror input epoch');
  assert.equal(sent.length,before,'admission must not focus or dispatch');
  for(const error of [new Error('MP-11: stale mirror input epoch'),Object.assign(new Error('page secret'),{code:'mirror_input_epoch_stale'})]) {
    host.request=async()=>{throw error};
    const rejected=await host.handle({id:'other',method:'host.browser',params:{op:'input'}});
    assert.equal(rejected.error.message,'MD-2: host browser operation failed; refresh state or check host browser readiness');
  }
}));

test('private CDP pipe loss retires the browser generation even while child is live', () => using(async ({host,chromium,connection}) => {
  let open=true;
  chromium.connection=connection;
  connection.isOpen=()=>open;
  const start=chromium.start;
  chromium.start=async()=>{open=true;return start()};
  const before=await host.request({op:'open',url:'https://example.com/'});
  open=false;
  await assert.rejects(host.request({op:'state'}),{code:'browser_unavailable'});
  const after=await host.request({op:'start'});
  assert.equal(after.generation,before.generation+1);
  assert.equal(after.tabs[0].tab_id,before.tab_id);
  await assert.rejects(host.request({op:'screenshot',tab_id:before.tab_id,generation:before.generation}),{code:'user_domain_stale_epoch'});
}));

test('MP-11 agent native input cannot bypass App human-channel admission', () => using(async ({host}) => {
  await host.request({op:'open',url:'https://example.com'});
  host.browser.appTabs={apps:new Map([['app',{targetId:'target-1'}]])};
  let calls=0;host.nativeComputer.execute=async()=>{calls++;return {};};
  const result=await host.handle({id:'native-app-denial',method:'host.computer',params:{op:'input',surface_id:'s',generation:'g',input:{kind:'click',x:1,y:1},_agent_input:true,observed_by:'agent:a'}});
  assert.equal(result.ok,false);assert.equal(calls,0);

}));

test('MP-08/MP-11 protected full captures cannot interrupt an in-flight pointer operation',()=>using(async({host,connection})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=host.tabs.get(opened.tab_id);
 let release,enter,captured=false;
 const held=new Promise(r=>release=r),entered=new Promise(r=>enter=r);
 connection.beforeSend=async method=>{if(method==='Page.captureScreenshot')captured=true;};
 const input=host.sampleLane(tab).run('input',async()=>{enter();await held;});await entered;
 const screenshot=host.request({op:'screenshot',tab_id:tab.tab_id,generation:opened.generation});
 await new Promise(r=>setImmediate(r));assert.equal(captured,false,'capture waits for paired pointer completion');
 release();await Promise.all([input,screenshot]);assert.equal(captured,true);
}));

// MP-08/MP-10: run this bound admission check under both supported host geometries.
test('MP-08/MP-10 #893 unsupported DPR fails before emulation/subscription mutation',()=>using(async({host,sent})=>{
 const {displayGeometry}=await import('./kernel-browser-geometry.mjs');
 const old=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{
  const opened=await host.request({op:'open',url:'about:blank'});
  const before=sent.filter(x=>x.method==='Emulation.setDeviceMetricsOverride').length;
  const dpr=displayGeometry.width===1920?2:3;
  await assert.rejects(host.request({op:'display_subscribe',tab_id:opened.tab_id,generation:opened.generation,codecs:['png'],bitrate:8000000,device_scale_factor:dpr}),/negotiation/);
  assert.equal(host.displays.size,0);assert.equal(host.scales.size,0);
  assert.equal(sent.filter(x=>x.method==='Emulation.setDeviceMetricsOverride').length,before);
 }finally{if(old===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=old}
}));

test('MP-08/MP-10/MP-11 viewer wheel on the CDP capture fallback uses awaited input and keeps Chromium live',()=>using(async({host,chromium,sent})=>{
 const {CompositorSource}=await import('./kernel-browser-compositor.mjs');
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 const source=new CompositorSource({tab,policy:host.protection,allowed:()=>true});source.attested=true;source.close=async()=>{};
 host.compositors.set(tab.tab_id,{source,ready:Promise.resolve(source)});
 host.displays.set('viewer',{tab_id:tab.tab_id,observed_by:'adapter',expires:Date.now()+60000,close:async()=>{}});
 const result=await host.handle({id:1,method:'host.browser',params:{op:'input',_display_input:true,tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120}}});
 assert.equal(result.ok,true);assert(chromium.child);assert.equal(host.generation,opened.generation);
 assert.equal(sent.filter(call=>call.method==='Input.dispatchMouseEvent').length,1);
}));

// MP-08/MP-10/MP-11: native bounds and the compositor image share one DPR.
for(const dpr of [1,2])test(`MP-10 display negotiates native view image scale at DPR${dpr}`,()=>using(async({host,sent})=>{
 const original=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{const tab=await host.request({op:'open',url:'about:blank'});
 await host.request({op:'display_subscribe',tab_id:tab.tab_id,generation:tab.generation,codecs:['png'],bitrate:8000000,device_scale_factor:dpr});
 assert.deepEqual(sent.find(c=>c.method==='Emulation.setDeviceMetricsOverride').params,{width:1280,height:800,deviceScaleFactor:dpr,scale:dpr,mobile:false});
 }finally{if(original===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=original;}
}));
