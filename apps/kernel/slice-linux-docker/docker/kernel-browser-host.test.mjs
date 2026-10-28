// MD-2/MD-4: focused adapter tests, not native-browser acceptance.
import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import os from "node:os";
import { PassThrough } from "node:stream";
import readline from "node:readline";
import { BrowserCdpClient } from "./browser-controller-cdp.mjs";
import { BrowserMemory } from "./browser-memory.mjs";
import { BrowserControllerStdioServer } from "./browser-controller.mjs";
import path from "node:path";
import { encodePng, decodePng, maskPng } from "./kernel-browser-pixels.mjs";
import { KernelBrowserHost, navigationUrl, scheduleHostRequest } from "./kernel-browser-host.mjs";
import { candidates as macCandidates, launchEnvironment as macEnvironment } from "./kernel-browser-macos.mjs";
import { launchEnvironment as linuxEnvironment } from "./kernel-browser-linux.mjs";
import { launchArguments, HostChromium, chromiumTemporaryEnvironment } from "./kernel-browser-process.mjs";
import { PointerOutsideViewport } from "./kernel-browser-refusal.mjs";
import { BrowserDialogDefaults } from "./browser-controller-dialogs.mjs";
// These fixtures exercise the host with the mirror and display kill-switched; tests that need them on set "1" themselves.
process.env.CHARIOX_KERNEL_BROWSER_MIRROR ??= '0'; process.env.CHARIOX_KERNEL_BROWSER_DISPLAY ??= '0';

test('MP-08 / MP-11: Chromium temporary sockets use the held private directory through a bounded path', () => {
  const environment = { TMPDIR: '/private/' + 'long/'.repeat(60), XAUTHORITY: '/private/authority' };
  const bounded = chromiumTemporaryEnvironment(environment, 5, 12345);
  assert.equal(bounded.TMPDIR, '/proc/12345/fd/5');
  assert.equal(bounded.XAUTHORITY, environment.XAUTHORITY);
  assert(!bounded.TMPDIR.includes(environment.TMPDIR));
  for (const pid of [0, 1, -1, NaN]) assert.throws(() => chromiumTemporaryEnvironment(environment, 5, pid));
});

function fixture(root, { realCdp = false } = {}) {
  fixture.popups = [];
  const pages = new Map();
  let next = 0;
  const handlers = new Set();
  const sent = [];
  const connection = {
    isOpen: () => true,
    close: async () => {},
    subscribe: handler => { handlers.add(handler); return () => handlers.delete(handler); },
    send: async (method, params, session) => {
      if (connection.beforeSend) await connection.beforeSend(method, params, session);
      sent.push({ method, params, session });
      if (method === "Target.getTargets") return { targetInfos: [...pages].map(([targetId, tab]) => ({ targetId, type: "page", ...tab })) };
      if (method === 'Target.getTargetInfo') return {targetInfo:pages.get(params.targetId)};
      if (method === "Target.createTarget") { const id = `target-${++next}`; pages.set(id, { url: params.url }); return { targetId: id }; }
      if (method === "Target.attachToTarget") return { sessionId: `session-${params.targetId}` };
      if (method === "Target.closeTarget") {
        pages.delete(params.targetId);
        if (pages.size === 0 && chromium.child) chromium.child.exitCode = 0;
        return {};
      }
      if (method === 'DOM.getDocument') return {root:{nodeId:1}};
      if (method === 'DOM.querySelectorAll') return {nodeIds:[]};
      if (method === "Page.captureScreenshot") return { data: encodePng(1280*(connection.density??1),800*(connection.density??1),Buffer.alloc(1280*800*4*(connection.density??1)**2,255)) };
      const target = session?.replace("session-", "");
      if (method === "Page.getFrameTree") return { frameTree: { frame: { id: "frame", url: "about:blank", loaderId: pages.get(target)?.document_id ?? `doc-${target}` } } };
      if (method === "Page.createIsolatedWorld") return { executionContextId: 42 };
      if (method === "Runtime.evaluate" && params.expression.includes("'popup")) { const id = `target-${++next}`; pages.set(id, { url: "about:blank" }); fixture.popups.push(id); return { result: { value: true } }; }
      if (method === "Runtime.evaluate" && params.expression.includes("visibilityState")) return { result: { value: {visible:true,dpr:chromium.scale??1,width:1280,height:800} } };
      if (method === "Page.getLayoutMetrics") return { cssVisualViewport: { scale: 1, zoom: 1, pageX: 0, pageY: 0 } };
      if (method === "DOM.performSearch") return { searchId: "search", resultCount: fixture.protectedField && params.query.includes("password") ? 1 : 0 };
      if (method === "DOM.getSearchResults") return { nodeIds: [2] };
      if (method === "DOM.describeNode") return { node: { localName: "input", backendNodeId: 7 } };
      if (method === "Runtime.evaluate") return { result: { value: fixture.secretFocused ?? false } };
      return {};
    },
  };
  const chromium = { child: null, start: async () => { chromium.child = { exitCode: null, signalCode: null }; return "http://127.0.0.1:1"; }, stop: async () => { chromium.child = null; } };
  const browserFactory = () => ({ connection, ensureConnection: async () => connection,ensureTargetSession:async (_connection,target)=>`session-${target}`, close: async () => {},
    reconcile: async viewport => { connection.density=viewport.device_scale_factor; return { tabs: [...pages].map(([target_id, tab]) => ({ target_id, document_id: `doc-${target_id}`, title: "fixture", ...tab })) }; },
    manageTab: async ({ target_id }) => pages.delete(target_id),
    navigate: async ({ target_id, url }) => { pages.set(target_id, { url }); },
    snapshot: async () => ({ text: "fixture" }),
    resolvePageTarget: async target => ({ connection, sessionId: `session-${target}` }),
    clearViewport: async target => connection.send('Emulation.clearDeviceMetricsOverride', {}, `session-${target}`),
    inputCapture: { hold: async()=>async()=>{}, run: async (_connection, _session, operation) => operation() },
  });
  return { host: new KernelBrowserHost(root, { chromium, browserFactory: realCdp
    ? () => new BrowserCdpClient({ connectionFactory: async () => connection }) : browserFactory }), chromium, handlers, sent, pages, connection };
}

async function using(callback, options) {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-md2-test-"));
  const context = fixture(root, options);
  try { await callback(context, root); } finally { fixture.secretFocused = false; fixture.protectedField = false; await context.host.stop(); await rm(root, { recursive: true, force: true }); }
}

test("MP-08/MP-09/MP-10/MP-11 a dialog without Chrome chrome is shared and answerable while its renderer is blocked", () => using(async ({ host }) => {
  const opened = await host.request({ op: "open", url: "https://example.com" });
  const tab = host.tabs.get(opened.tabs[0].tab_id);
  host.browser.dialogDefaults = new BrowserDialogDefaults();
  host.browser.dialogDefaults.observe({ method: "Page.javascriptDialogOpening", params: {
    type: "alert", message: "Hello from the page", url: "https://example.com/path" } }, tab.target_id, tab.document_id);
  host.browser.reconcile = async () => { throw Error("blocked renderer must not be queried"); };
  const state = await host.handle({ id: 1, method: "host.browser", params: { op: "state", state_wait: true } });
  assert.equal(state.ok, true);
  assert.equal(state.result.chrome_window, null);
  assert.equal(state.result.dialogs.length, 1);
  const dialog = state.result.dialogs[0];
  assert.equal(dialog.message, "Hello from the page");
  assert.equal(dialog.origin, "https://example.com");
  assert.equal(dialog.tab_id, tab.tab_id);
  assert.equal(typeof state.result._state_signal, "number");
  const answer = { op: "dialog", ...dialog, action: "accept" };
  host.browser.handleDialog = async request => {
    assert.equal(request.target_id, tab.target_id);
    assert.equal(request.action, "accept");
    host.browser.dialogDefaults.delete(tab.target_id);
  };
  const answered = await host.handle({ id: 2, method: "host.browser", params: answer });
  assert.equal(answered.ok, true);
  assert.deepEqual(answered.result.dialogs, []);
  const stale = await host.handle({ id: 3, method: "host.browser", params: answer });
  assert.equal(stale.ok, false);
  assert.equal(stale.error.code, "browser_dialog_invalid");
  assert.deepEqual(scheduleHostRequest({ method: "host.browser", params: answer }), { kind: "bridge" });
}));

test("MP-08/MP-09/MP-10/MP-11 a dialog opening during a state read releases that read", () => using(async ({ host }) => {
  const opened = await host.request({ op: "open", url: "https://example.com" });
  const tab = host.tabs.get(opened.tabs[0].tab_id);
  host.browser.dialogDefaults = new BrowserDialogDefaults();
  host.browser.reconcile = async () => {
    host.browser.dialogDefaults.observe({ method: "Page.javascriptDialogOpening", params: {
      type: "confirm", message: "Continue?", url: "https://example.com" } }, tab.target_id, "new-document");
    host.stateSignal.notify();
    return new Promise(() => {});
  };
  const state = await host.request({ op: "state", state_wait: true });
  assert.equal(state.dialogs[0].document_id, "new-document");
  assert.equal(state.tabs[0].document_id, "new-document");
  assert.equal(host.stateSignal.waiters.size, 0);
}));

test("MP-08/MP-10 idle App bridge drains do not hold host input or replies", { timeout: 5000 }, () => using(async ({ host, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const input = new PassThrough(), output = new PassThrough();
  const lines = readline.createInterface({ input: output }), waiters = new Map();
  lines.on("line", line => { const reply = JSON.parse(line); waiters.get(reply.id)?.resolve(reply); });
  const entered = Promise.withResolvers(), release = Promise.withResolvers();
  const server = new BrowserControllerStdioServer({ input, output, scheduleRequest: scheduleHostRequest,
    handleRequest: async (request, options) => {
      if (request.method === "browser.app.calls") { entered.resolve(); await release.promise; return {id: request.id, ok: true, result: {calls: []}}; }
      if (request.method === "browser.app.respond") return {id: request.id, ok: true, result: {delivered: true}};
      return host.handle(request, options);
    } });
  const running = server.run();
  const rpc = (id, method, params = {}) => {
    const result = Promise.withResolvers(); waiters.set(id, result);
    input.write(JSON.stringify({id, method, params}) + "\n"); return result.promise;
  };
  try {
    const drain = rpc(1, "browser.app.calls"); await entered.promise;
    const result = await Promise.race([Promise.all([
      rpc(2, "host.browser", {op: "input", tab_id: opened.tab_id, generation: opened.generation,
        document_id: opened.tabs[0].document_id, input: {kind: "text", text: "typed"}}),
      rpc(3, "browser.app.respond"),
    ]), new Promise(resolve => setTimeout(() => resolve(null), 200))]);
    assert(result, "host input and App replies must finish while the idle drain is still held");
    assert(result.every(reply => reply.ok));
    assert(sent.some(call => call.method === "Input.insertText"));
    release.resolve(); await drain;
  } finally { release.resolve(); input.end(); await running; lines.close(); output.end(); }
}));

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

// MP-08/MP-10: protocol 491 push credits wait on readiness and return a
// frame, or nothing after a bounded budget; reset retires the stream base.
test('MP-08/MP-10 push credit waits on encoder readiness and retires the base on reset',()=>using(async({host,sent})=>{
 const old=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{
  const {DisplayStream}=await import('./kernel-browser-display.mjs');
  const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
  const sample={serial:1,document_id:tab.document_id,tab_id:tab.tab_id,raw:{},motion:true};
  const source={attested:true,closed:false,policy:host.protection,changedAt:-Infinity,valid:()=>true,allowed:()=>true,sample:()=>sample,close:async()=>{}};
  let waits=[],invalidated=0;const stream=new DisplayStream({subscription_id:'push',tab_id:tab.tab_id,observed_by:'adapter',codec:'avc1.420033',device_scale_factor:1,bitrate:8000000},{encoder:{close:async()=>{}}});
  const base=()=>Object.assign(stream,{document_id:tab.document_id,previous:{signature:'base'},exact:true,compositorSerial:1,compositorRegionRevision:undefined,creditEpoch:0,refinerDocument:tab.document_id,
   refiner:{quietNativeMs:50,request:()=>null,invalidate(){},close:async()=>{}},producer:{source,frames:[],waitReady:async ms=>{waits.push(ms);await new Promise(r=>setTimeout(r,Math.min(ms,5)))},take:()=>null,feedback(){},invalidate(){},retireUnsent(){},close:async()=>{}}});
  base();const invalidate=stream.invalidate.bind(stream);stream.invalidate=()=>{invalidated++;invalidate();base();};
  host.compositors.set(tab.tab_id,{document:tab.document_id,source,ready:Promise.resolve(source)});host.displays.set(stream.subscription_id,stream);
  const before=sent.length,started=performance.now();
  const credit={op:'screenshot',display_subscription_id:'push',generation:opened.generation,after_sequence:0,push:{reset:false,congested:false}};
  const result=await host.request(credit);
  assert.equal(result.frame_sent,false);assert.ok(performance.now()-started>=90,'an empty push credit waits for its budget');
  assert.ok(waits.length>=1&&waits.every(ms=>ms>0&&ms<=100),'waits are bounded by the credit budget');
  assert.equal(sent.length,before,'unchanged push credits perform no CDP observation');assert.equal(invalidated,0);
  await host.request({...credit,push:{reset:true,congested:false}});assert.equal(invalidated,1,'reset retires the reference chain once');
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
test('MP-08 / MP-11: owned Chromium never forces its renderer accessibility tree', () => {
  for (const [headless, display] of [[false, false], [false, true], [true, false]])
    assert(!launchArguments('/tmp/private-profile', headless, display).some(arg => arg.startsWith('--force-renderer-accessibility')));
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
    if(polled.frame&&decodePng(polled.frame.data_base64).pixels[0]===255)break;
    await new Promise(resolve=>setTimeout(resolve,5));
  }
  assert(polled.frame.sequence>0&&polled.frame.sequence<=100);
  assert.equal(polled.frame.mime_type,'image/png');assert.equal(decodePng(polled.frame.data_base64).pixels[0],255);
  assert.equal(sent.filter(call => call.method === "Page.screencastFrameAck").length, 100);
  await host.request({ op: "unsubscribe", ...subscription });
  assert.equal(handlers.size, 2, "lockdown and browser recovery observers remain");
  assert(sent.some(call => call.method === "Page.stopScreencast"));
  chromium.child.exitCode = 1;
  await assert.rejects(host.request({ op: "poll", ...subscription }), {code:"user_domain_stale_reference"});
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
      if(frame&&decodePng(frame.data_base64).pixels[0]===255)break;
      await new Promise(resolve=>setTimeout(resolve,5));
    }
    assert.equal(frame.mime_type,'image/png');assert.equal(decodePng(frame.data_base64).pixels[0],255);
  }
  await host.request({ op: "unsubscribe", ...first });
  assert(!sent.some(call => call.method === "Page.stopScreencast"));
  await host.request({ op: "unsubscribe", ...second });
  assert.equal(sent.filter(call => call.method === "Page.stopScreencast").length, 1);
}));
test("MD-2: text input reaches any focused field, secret or not (A-L23)", () => using(async ({ host, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const binding = { tab_id: opened.tab_id, generation: opened.generation };
  fixture.secretFocused = true;
  await host.request({ op: "input", ...binding, input: { kind: "text", text: "fixture" } });
  assert(sent.some(call => call.method === "Input.insertText"));
  fixture.secretFocused = false;
  await assert.rejects(host.request({ op: "input", ...binding, input: { kind: "click", x: 1280, y: 0 } }), /viewport/);
  // Protocol 530: middle/right buttons are the band's (Chrome's own UI), never a page click.
  for (const input of [{ kind: "click", x: 1, y: 1, button: "right" }, { kind: "click", x: 1, y: 1, button: "middle" }])
    await assert.rejects(host.request({ op: "input", ...binding, input }), /unsupported input/);
  await host.request({ op: "input", ...binding, input: { kind: "drag", x: 1, y: 1, to_x: 5, to_y: 5 } });
  await assert.rejects(host.request({ op: "input", ...binding, input: { kind: "drag", x: 1, y: 1, to_x: 1280, to_y: 5 } }), /viewport/);
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

// MP-08/MP-10 (vidlat r23): a person's wheel cannot change the Tab list, so it does not pay a full reconcile per event;
// a click, an agent's scroll and every state read still do.
test('MP-08 a displayed wheel replies from the last observation, other input reconciles', () => using(async ({host}) => {
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 const reconcile=host.browser.reconcile;let reconciles=0;
 host.browser.reconcile=async(...args)=>{reconciles++;return reconcile(...args)};
 const scroll=extra=>({op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,...extra,input:{kind:'scroll',x:2,y:2,delta_x:0,delta_y:120}});
 const wheel=await host.request(scroll({_display_input:true}));
 assert.equal(reconciles,0,'a displayed wheel must not reconcile');
 assert.equal(wheel.tabs[0].tab_id,tab.tab_id);
 await host.request(scroll({_agent_input:true}));
 assert.equal(reconciles,1,'an agent scroll still reconciles');
 await host.request({...scroll({}),input:{kind:'click',x:2,y:2}});
 assert.equal(reconciles,2,'a click still reconciles');
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

test("MP-08/MP-11 policy updates retire cached observations without a broad mask",()=>using(async({host,handlers,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),subscription=await host.request({op:'subscribe',tab_id:opened.tab_id,generation:opened.generation});
 const session=sent.find(call=>call.method==='Page.startScreencast').session;
 const waitFrame=async()=>{for(let n=0;n<100;n++){const frame=(await host.request({op:'poll',...subscription})).frame;if(frame)return frame;await new Promise(r=>setTimeout(r,5));}assert.fail('fresh capture required');};
 assert.equal(decodePng((await waitFrame()).data_base64).pixels[0],255);
 await host.protect({unknown:false,values:['synthetic-only'],targets:[]});
 assert.equal((await host.request({op:'poll',...subscription})).frame,null,'old observation retires instead of a whole-page blackout');
 for(const stream of host.streams.values())stream.nextCapture=0;
 for(const handler of handlers)handler({method:'Page.screencastFrame',sessionId:session,params:{data:'untrusted-raw',sessionId:1}});
 assert.equal(decodePng((await waitFrame()).data_base64).pixels[0],255,'registration without a fill leaves ordinary pixels visible');
}));

test("MP-08/MP-11: policy flushes old frames without masks for registration alone across recovery", () => using(async ({ host, handlers, chromium, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const subscription = await host.request({ op: "subscribe", tab_id: opened.tab_id, generation: opened.generation });
  const session = sent.find(call => call.method === "Page.startScreencast").session;
  const emit = () => { for (const handler of handlers) handler({ method: "Page.screencastFrame", sessionId: session, params: { data: "unsafe-raw-pixels", sessionId: 1 } }); };
  emit();
  assert.notEqual((await host.request({ op: "poll", ...subscription })).frame?.data_base64, "unsafe-raw-pixels");
  const policy = { unknown: false, values: ["synthetic-only"], targets: [] };
  await host.protect(policy);
  assert.equal((await host.request({ op: "poll", ...subscription })).frame, null);
  emit();
  await host.protect(policy); // An unchanged policy must not erase every poll.
  const protectedFrame = (await host.request({ op: "poll", ...subscription })).frame;
  assert.notEqual(protectedFrame?.data_base64, "unsafe-raw-pixels");
  const capture = await host.request({ op: "screenshot", tab_id: opened.tab_id, generation: opened.generation });
  assert.equal(decodePng(capture.data_base64).pixels[0],255); // MP-11 registration alone leaves source pixels intact
  const second = await host.request({ op: "subscribe", tab_id: opened.tab_id, generation: opened.generation });
  assert.equal((await host.request({ op: "poll", ...second })).frame, null); // capture is asynchronous
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

test('MP-11 legacy observation retires an in-flight frame after document changes',()=>using(async({host,connection,handlers,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'});let entered,release;
 const waiting=new Promise(resolve=>entered=resolve),held=new Promise(resolve=>release=resolve);
 connection.beforeSend=async method=>{if(method==='Page.captureScreenshot'){entered();await held;}};
 const subscription=await host.request({op:'subscribe',tab_id:opened.tab_id,generation:opened.generation});await waiting;
 const session=sent.find(call=>call.method==='Page.startScreencast').session;
 for(const handler of handlers)handler({sessionId:session,method:'DOM.documentUpdated',params:{}});
 release();for(let n=0;n<50&&host.streams.get(subscription.subscription_id).capturing;n++)await new Promise(resolve=>setTimeout(resolve,5));
 assert.equal((await host.request({op:'poll',...subscription})).frame,null,'retired capture cannot overwrite the observation');
}));
test('MP-11 legacy observation retires an in-flight frame after attribute-only protection changes',()=>using(async({host,connection,handlers,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'});let entered,release;
 const waiting=new Promise(resolve=>entered=resolve),held=new Promise(resolve=>release=resolve);
 connection.beforeSend=async method=>{if(method==='Page.captureScreenshot'){entered();await held;}};
 const subscription=await host.request({op:'subscribe',tab_id:opened.tab_id,generation:opened.generation});await waiting;
 const session=sent.find(call=>call.method==='Page.startScreencast').session;
 for(const handler of handlers)handler({sessionId:session,method:'DOM.attributeModified',params:{name:'type'}});
 release();for(let n=0;n<50&&host.streams.get(subscription.subscription_id).capturing;n++)await new Promise(resolve=>setTimeout(resolve,5));
 assert.equal((await host.request({op:'poll',...subscription})).frame,null,'retired capture cannot be returned after a field type change');
}));
test('MP-11 legacy observation keeps its bound frame through DOM tree churn',()=>using(async({host,handlers,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'});
 const subscription=await host.request({op:'subscribe',tab_id:opened.tab_id,generation:opened.generation});
 const stream=host.streams.get(subscription.subscription_id);
 for(let n=0;n<50&&(stream.capturing||!stream.latest||decodePng(stream.latest.data_base64).pixels[0]!==255);n++)await new Promise(resolve=>setTimeout(resolve,5));
 const session=sent.find(call=>call.method==='Page.startScreencast').session;
 for(const method of ['DOM.childNodeInserted','DOM.childNodeCountUpdated'])for(const handler of handlers)handler({sessionId:session,method,params:{}});
 for(const handler of handlers)handler({sessionId:session,method:'DOM.attributeModified',params:{name:'class'}});
 assert.equal((await host.request({op:'poll',...subscription})).frame,null,'geometry changes retire the observation without a blanket frame');
}));
test('MP-11 a refused compositor attestation is cached per document with backoff',()=>using(async({host,connection,handlers,pages})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=host.tabs.get(opened.tab_id);
 let attempts=0,shots=0;host.browser.inputCapture.hold=async()=>{attempts++;return async()=>{}};
 const send=connection.send;
 connection.send=async(method,params,session)=>{
  // An animated page: every protected capture differs from the previous one.
  if(method==='Page.captureScreenshot')return {data:encodePng(1280,800,Buffer.alloc(1280*800*4,++shots%2?255:0))};
  const result=await send(method,params,session);
  if(method==='Page.startScreencast')for(const handler of handlers)handler({method:'Page.screencastFrame',sessionId:session,params:{data:encodePng(1280,800,Buffer.alloc(1280*800*4,255)),sessionId:1}});
  return result;
 };
 const stream={codec:'avc1.420033',device_scale_factor:1};
 for(let n=0;n<4;n++)assert.equal(await host.compositorFor(tab,stream),null);
 assert.equal(attempts,1,'credits cannot rebuild a refused source');
 host.compositors.get(tab.tab_id).retryAt=-Infinity;
 assert.equal(await host.compositorFor(tab,stream),null);assert.equal(attempts,2,'backoff expiry retries');
 assert(host.compositors.get(tab.tab_id).retryAt>performance.now()+1500,'consecutive refusals back off further');
 pages.get(tab.target_id).document_id='next';tab.document_id='next';
 assert.equal(await host.compositorFor(tab,stream),null);assert.equal(attempts,3,'a new document retries at once');
}));
test('MP-11 overlapping saves serialize the shared tabs.json.new writer',()=>using(async({host},root)=>{
 await host.request({op:'open',url:'about:blank'});
 await Promise.all(Array.from({length:20},()=>{host.lastSaved=null;return host.save();}));
 assert.equal(JSON.parse(await readFile(path.join(root,'tabs.json'),'utf8')).tabs.length,1);
}));
test('MP-11 a compositor close rejected by a dead connection cannot abort stop',()=>using(async({host,chromium})=>{
 const opened=await host.request({op:'open',url:'about:blank'});
 host.compositors.set(opened.tab_id,{ready:Promise.resolve(null),source:{close:async()=>{throw Error('connection closed')}}});
 assert.equal((await host.request({op:'stop'})).state,'stopped');assert.equal(chromium.child,null);
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
  connection.beforeSend = async method => { if (method === "Page.getFrameTree") { entered.resolve(); await release.promise; } };
  const pending = host.request({ op: "input", tab_id: opened.tab_id, generation: opened.generation,
    document_id: opened.tabs[0].document_id, input: { kind: "text", text: "synthetic" } }, { signal: cancel.signal });
  await entered.promise;
  cancel.abort(); release.resolve();
  await assert.rejects(pending, error => error.code === "browser_action_cancelled");
  assert(!sent.some(call => call.method === "Input.insertText"));
}));

test("MP-08/MP-11: revocation after a key press permits only its non-text release", () => using(async ({ host, sent, connection }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const cancel = new AbortController();
  connection.beforeSend = async (method, params) => { if (method === "Input.dispatchKeyEvent" && params.type === "keyDown") cancel.abort(); };
  await assert.rejects(host.request({ op: "input", tab_id: opened.tab_id, generation: opened.generation,
    document_id: opened.tabs[0].document_id, input: { kind: "key", key: "Enter" } }, { signal: cancel.signal }), error => error.code === "browser_action_cancelled");
  const keys=sent.filter(call => call.method === "Input.dispatchKeyEvent");
  assert.deepEqual(keys.map(call=>call.params.type),["keyDown","keyUp"]);
  assert.equal(keys[1].params.text,undefined);
  assert.equal(keys[1].params.unmodifiedText,undefined);
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
  let reads = 0;
  connection.beforeSend = async method => { if (method === "Page.getFrameTree" && ++reads === 2) pages.set(target, { url: "about:blank", document_id: "replacement" }); };
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
  connection.beforeSend = async method => { if (method === "Page.getFrameTree") { entered.resolve(); await release.promise; } };
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

test("MP-08/MP-11: agents drive App tabs only with the kernel's App view claim; App tabs are not restored", () => using(async ({ host, sent }, root) => {
  const opened = await host.request({op:"open",url:"https://app.a.invalid/"});
  const target = host.tabs.get(opened.tab_id).target_id;
  const inputs = [];
  host.browser.appTabs = {apps:new Map([["app", {targetId:target, origin:"https://app.a.invalid"}]]),
    noteAgent: (targetId, agent) => inputs.push([targetId, agent])};
  const agent = {focused_agent:true,_agent_input:true,observed_by:"agent:agent-7"};
  const command = {op:"input",tab_id:opened.tab_id,generation:opened.generation,document_id:`doc-${target}`,input:{kind:"click",x:12,y:20}};
  for (const op of ["input", "navigate", "close"]) {
    await assert.rejects(host.request({...command,...agent,op,url:"https://app.a.invalid/"}), error => error.code === "user_domain_not_granted");
  }
  assert(!sent.some(call=>call.method==="Input.dispatchMouseEvent"));
  const claimed = {...command,...agent,_app_view_tab:opened.tab_id};
  await host.request(claimed);
  assert(sent.some(call=>call.method==="Input.dispatchMouseEvent"));
  assert.deepEqual(inputs, [[target, "agent-7"]]);
  await host.request({...command});
  // A person's input is not an agent's: the Tab's record does not change.
  assert.deepEqual(inputs.slice(1), []);
  // The App page stays on its own origin, whoever navigates it.
  await assert.rejects(host.request({...claimed,op:"navigate",url:"https://example.com/"}), error => error.code === "user_domain_not_granted");
  await assert.rejects(host.request({...command,op:"navigate",url:"https://example.com/"}), error => error.code === "user_domain_not_granted");
  // An agent's navigation is its input too.
  inputs.length = 0;
  await host.request({...claimed,op:"navigate",url:"https://app.a.invalid/list"});
  await host.request({...command,op:"navigate",url:"https://app.a.invalid/"});
  assert.deepEqual(inputs, [[target, "agent-7"]]);
  const {readFile} = await import("node:fs/promises");
  assert.deepEqual(JSON.parse(await readFile(path.join(root,"tabs.json"),"utf8")).tabs, []);
  await host.request({...claimed,op:"close"});
  assert(!host.tabs.has(opened.tab_id));
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
  assert.equal(handlers.size,2,'lockdown and browser recovery observers remain');
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
      const width=Math.round((params.clip?.width??1280)*(params.clip?.scale??1)*(connection.density??1));
      const height=Math.round((params.clip?.height??800)*(params.clip?.scale??1)*(connection.density??1));
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
  ['foreign tab snapshot', 'snapshot', 'stale_reference', true],
  ['foreign tab screenshot', 'screenshot', 'stale_reference', true],
  ['foreign browser region capture', 'screenshot', 'stale_reference', true],
  ['foreign note-window observation', 'note_selection', 'stale_reference', true],
  ['forged resource references', 'snapshot', 'stale_reference', true],
  ['stale browser input document', 'input', 'stale_reference'],
  ['stale App input document', 'input', 'stale_reference'],
  ['cross-identity capture correlation replay', 'screenshot', 'stale_reference', true],
  ['stale capture generation', 'screenshot', 'stale_epoch'],
];
for (const [label, op, reason, missing] of refusalCases) test('MD434 explicit refusal: '+label, () => using(async ({host}) => {
  const opened=await host.handle({id:1,method:'host.browser',params:{op:'open',url:'https://fixture.invalid/'}});
  const generation=host.generation;
  const tab=[...host.tabs.values()][0];
  const params={op,generation,tab_id:missing?'foreign-or-forged-id':tab.tab_id,observed_by:'terminal:fixture'};
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

test("MP-08/MP-10 state follows browser recovery but never starts an explicitly stopped browser", () => using(async ({host,chromium,sent}) => {
  for(const retained of [true,false])await assert.rejects(host.request({op:'state',_retained_agent:retained}),{code:'browser_unavailable'});
  assert.equal(chromium.child,null);assert.deepEqual(sent,[]);
  const opened=await host.request({op:'open',url:'https://example.com'});
  chromium.child.exitCode=1;
  const recovered=await host.request({op:'state'});
  assert.equal(recovered.generation,opened.generation+1);
  assert.equal(recovered.tabs[0].tab_id,opened.tab_id);
  await host.stop();const requests=sent.length;
  for(const retained of [true,false])await assert.rejects(host.request({op:'state',_retained_agent:retained}),{code:'browser_unavailable'});
  assert.equal(sent.length,requests);
}));

test("MP-08/MP-10 mirror reads and cleanup never start an explicitly stopped browser", () => using(async ({ host, chromium, sent }) => {
  const previous = process.env.CHARIOX_KERNEL_BROWSER_MIRROR;
  process.env.CHARIOX_KERNEL_BROWSER_MIRROR = "1";
  try {
    let launches = 0;
    const start = chromium.start;
    chromium.start = async () => { launches++; return start(); };
    const opened = await host.request({ op:'open', url:'https://example.com' });
    for (const stopped of [true]) {
      await host.stop();
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

test('MP-11: only typed pre-dispatch refusals survive host error sanitization', () => using(async ({host,sent}) => {
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
  for(const error of [new Error('MP-11: stale mirror input epoch'),new Error(new PointerOutsideViewport().message),Object.assign(new Error('page secret'),{code:'mirror_input_epoch_stale'})]) {
    host.request=async()=>{throw error};
    const rejected=await host.handle({id:'other',method:'host.browser',params:{op:'input'}});
    assert.match(rejected.error.message,/^MD-2: host browser operation failed; refresh state or check host browser readiness \(/);
    assert.notEqual(rejected.error.message,'MP-11: stale mirror input epoch');
    assert(!rejected.error.message.includes('page secret'));
  }
}));

test('MP-08/MP-10/MP-11 an agent click outside the viewport says so, in CSS pixels, without dispatch', () => using(async ({host, sent}) => {
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0],viewport=(await host.request({op:'state'})).viewport;
 const before=sent.filter(x=>x.method==='Input.dispatchMouseEvent').length;
 // A device-pixel guess from a DPR 2 screenshot lands beyond the CSS viewport.
 const refused=await host.handle({id:'outside',method:'host.browser',params:{op:'input',_agent_input:true,tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'click',x:viewport.css_width+158,y:355}}});
 assert.equal(refused.ok,false);
 assert.equal(refused.error.code,'kernel_browser_failed');
 assert.equal(refused.error.message,'MD-2: pointer outside viewport; x/y are CSS pixels inside state viewport css_width x css_height (screenshot pixels / device_scale_factor)');
 assert.equal(sent.filter(x=>x.method==='Input.dispatchMouseEvent').length,before,'nothing is dispatched');
 assert.equal(host.browser?.connection!==undefined,true,'a refused coordinate does not stop the browser');
}));

test('private CDP pipe loss retires the browser generation even while child is live', () => using(async ({host,chromium,connection}) => {
  let open=true;
  chromium.connection=connection;
  connection.isOpen=()=>open;
  const start=chromium.start;
  chromium.start=async()=>{open=true;return start()};
  const before=await host.request({op:'open',url:'https://example.com/'});
  open=false;
  const after=await host.request({op:'state'});
  assert.equal(after.generation,before.generation+1);
  assert.equal(after.tabs[0].tab_id,before.tab_id);
  await assert.rejects(host.request({op:'screenshot',tab_id:before.tab_id,generation:before.generation}),{code:'user_domain_stale_epoch'});
}));

// MP-08 / MP-10 / MP-11 A07: the code exception is an internal owner path.
test("MP-08/MP-10/MP-11 owner click keeps the kernel-bound full URL through host dispatch", () => using(async ({ host }) => {
  const url = "https://example.test/confirm?id=first#confirm";
  const opened = await host.request({ op: "open", url });
  const tab = opened.tabs[0];
  let captured;
  host.browser.performAction = async action => { captured = action; return {}; };
  const result = await host.handle({ id: "owner-click", method: "host.secret", params: {
    tab_id: tab.tab_id, generation: opened.generation, document_id: tab.document_id,
    node_ref: "backend:103", observed_by: "terminal:owner",
    action: { kind: "click", expected_document_url: url },
  } });
  assert.equal(result.ok, true);
  assert.equal(captured.action.kind, "click");
  assert.equal(captured.action.expected_document_url, url);
  assert.equal(captured.document_id, tab.document_id);
  assert.equal(captured.node_ref, "backend:103");
}));

test("MP-08/MP-10/MP-11 owner code flag reaches the bound controller action", () => using(async ({ host }) => {
  const opened = await host.request({ op: "open", url: "https://example.test/verification" });
  const tab = opened.tabs[0];
  let captured;
  host.browser.performAction = async action => { captured = action; return {}; };
  const params = { tab_id: tab.tab_id, generation: opened.generation, document_id: tab.document_id,
    node_ref: "backend:104", observed_by: "terminal:owner", mask_code_input: true,
    action: { kind: "fill", text: "am7-fixture-code", expected_document_url: "https://example.test/verification" } };
  const result = await host.handle({ id: "owner-code", method: "host.secret", params });
  assert.equal(result.ok, true);
  assert.equal(captured.action.mask_code_input, true);
  assert.equal(captured.document_id, tab.document_id);
  captured = null;
  const denied = await host.handle({ id: "agent-code", method: "host.secret", params: { ...params, observed_by: "agent:unadmitted" } });
  assert.equal(denied.ok, false);
  assert.equal(captured, null, "a non-terminal code exception cannot reach physical input");
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
 const old=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{
  const opened=await host.request({op:'open',url:'about:blank'});
  const before=sent.filter(x=>x.method==='Emulation.setDeviceMetricsOverride').length;
  const dpr=3;
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

// MP-08/MP-10/MP-11 (review #893 P1): display_subscribe is an observation.
// It never stops or relaunches the browser (tab state, generation and agent
// references survive); the DSF-2 window serves both densities 1:1.
for(const dpr of [1,2])test(`MP-11 a DPR${dpr} viewer subscribe never relaunches the browser`,()=>using(async({host,sent,chromium})=>{
 const original=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{chromium.scale=2;const tab=await host.request({op:'open',url:'about:blank'});
 const launched=chromium.child,stops=chromium.stop;let stopped=0;chromium.stop=async()=>{stopped++;return stops()};
 const subscribed=await host.request({op:'display_subscribe',tab_id:tab.tab_id,generation:tab.generation,codecs:['png'],bitrate:8000000,device_scale_factor:dpr});
 assert.equal(stopped,0,'no stop/relaunch from an observation');assert.equal(chromium.child,launched);
 assert.equal(subscribed.generation,tab.generation);assert.equal(host.generation,tab.generation);assert.equal(chromium.scale,2);
 assert.equal(subscribed.device_scale_factor,2);assert.equal(tab.viewport.device_scale_factor,2);assert.equal(sent.filter(c=>c.method==='Emulation.setDeviceMetricsOverride').length,0,'viewer subscription never changes the viewport');
 }finally{if(original===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=original;}
}));

// MP-08/MP-11: on the owned Computer desktop the Browser window is the real
// visible one; an emulated DPR outliving its viewer leaves it unbindable.
test('MP-08 a desktop tab drops the Browser panel DPR emulation with its last viewer',()=>using(async({host,sent})=>{
 const original=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{const tab=await host.request({op:'open',url:'about:blank'});host.chromium.desktop={binding:()=>null};
 const subscribed=await host.request({op:'display_subscribe',tab_id:tab.tab_id,generation:tab.generation,codecs:['png'],bitrate:8000000,device_scale_factor:2});
 assert.equal(subscribed.generation,tab.generation,'the shared desktop never restarts Chromium');
 assert.equal(sent.filter(c=>c.method==='Emulation.setDeviceMetricsOverride').length,0,'a Browser viewer never emulates density on the desktop');
 await host.request({op:'unsubscribe',subscription_id:subscribed.subscription_id,generation:subscribed.generation});
 assert.equal(sent.filter(c=>c.method==='Emulation.clearDeviceMetricsOverride').length,1);assert.equal(host.scales.has(tab.tab_id),false);
 }finally{delete host.chromium.desktop;if(original===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=original;}
}));

// MP-08/MP-10: overlapping reconciles must not race one temporary tabs file.
test("MP-08/MP-10 concurrent tab saves are serialized", async () => {
  const { mkdtemp, readFile, rm } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const root = await mkdtemp(path.join(tmpdir(), "tabs-save-"));
  try {
    const host = new KernelBrowserHost(root);
    host.tabs.set("a", { tab_id: "a", url: "https://example.com/" });
    const first = host.save();
    host.tabs.set("b", { tab_id: "b", url: "https://example.org/" });
    await Promise.all([first, host.save(), host.save()]);
    assert.equal(JSON.parse(await readFile(path.join(root, "tabs.json"), "utf8")).tabs.length, 2);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("MP-08/MP-10 a refused native start retries after one second, backing off to a minute", async () => {
  const { nativeRetryDelayMs } = await import("./kernel-browser-host.mjs");
  assert.deepEqual([1, 2, 3, 4, 5, 6, 7, 12].map(nativeRetryDelayMs), [1000, 2000, 4000, 8000, 16000, 32000, 60000, 60000]);
});

// MP-08/MP-10/MP-11: a human navigation must be visible on the owned desktop.
test('human navigation foregrounds the requested tab after retiring native claims',()=>using(async({host,pages})=>{
 const first=await host.request({op:'open',url:'https://en.wikipedia.org/wiki/Linux'});
 const firstTarget=host.tabs.get(first.tab_id).target_id;
 const second=await host.request({op:'open',url:'https://developer.mozilla.org/en-US/docs/Web/JavaScript'});
 host.chromium.desktop={};
 const old=host.foreground.claim(second.tab_id),events=[];
 host.compositors.set(second.tab_id,{ready:Promise.resolve(),source:{close:async()=>{events.push('retire')}}});
 host.browser.manageTab=async({target_id,action})=>{
  assert.equal(action,'activate');assert.equal(target_id,firstTarget);
  assert(!host.foreground.holds(second.tab_id,old),'old native source claim must be gone before physical focus');
  events.push('activate');
 };
 const navigate=host.browser.navigate;host.browser.navigate=async p=>{events.push('navigate');await navigate(p)};
 await host.request({op:'navigate',tab_id:first.tab_id,generation:second.generation,url:'https://www.wikipedia.org/'});
 assert.deepEqual(events,['retire','activate','navigate']);
 assert.equal(pages.get(firstTarget).url,'https://www.wikipedia.org/');
 host.chromium.desktop=null;
}));

// MP-08/MP-11: background agent navigation cannot acquire the human foreground.
for (const flags of [{_agent_input:true},{focused_agent:'agent-a'}]) test('MP-08/MP-11 agent navigation preserves physical focus '+Object.keys(flags)[0],()=>using(async({host})=>{
 const opened=await host.request({op:'open',url:'https://en.wikipedia.org/wiki/Linux'});
 host.chromium.desktop={};
 host.browser.manageTab=async()=>{throw new Error('agent navigation must not activate a desktop tab')};
 await host.request({op:'navigate',tab_id:opened.tab_id,generation:opened.generation,url:'https://www.wikipedia.org/',...flags});
 host.chromium.desktop=null;
}));

test("MP-08 popup origins stay internal and distinguish unrelated native tabs", () => using(async ({host,pages}) => {
  const opened = await host.request({op:"open",url:"https://en.wikipedia.org"})
  const opener = host.tabs.get(opened.tab_id).target_id
  pages.set("native-human", {url:"https://github.com"})
  pages.set("popup", {url:"https://www.wikipedia.org",opener_target_id:opener})
  const state = await host.request({op:"state"})
  const popup = state.tabs.find(tab => tab.url === "https://www.wikipedia.org")
  const native = state.tabs.find(tab => tab.url === "https://github.com")
  assert.equal(state._tab_creation_actions[popup.tab_id], undefined)
  assert.equal(state._tab_creation_actions[native.tab_id], undefined)
  assert(!JSON.stringify(state.tabs).includes("opener_target_id"))
}))

test("MP-08 completed agent input cannot attribute a later native popup on the same document", () => using(async ({host, pages, connection, handlers}) => {
  const opened = await host.request({op:"open", url:"https://www.wikipedia.org"});
  const source = host.tabs.get(opened.tab_id);
  const input = {op:"input", tab_id:source.tab_id, generation:opened.generation,
    document_id:source.document_id, _action_id:"action-agent", input:{kind:"click", x:10, y:10}};
  connection.beforeSend = async (method, params) => {
    if (method !== "Input.dispatchMouseEvent" || params.type !== "mouseReleased") return;
    // CDP creation arrives during the actual dispatch. Inventory is delayed.
    for (const handler of handlers) handler({method:"Target.targetCreated", params:{targetInfo:{type:"page", targetId:"agent-popup", openerId:source.target_id}}});
  };
  await host.request(input);
  connection.beforeSend = null;
  pages.set("agent-popup", {url:"https://en.wikipedia.org", opener_target_id:source.target_id});
  // Native click after completion bypasses kernel begin/takeover.
  for (const handler of handlers) handler({method:"Target.targetCreated", params:{targetInfo:{type:"page", targetId:"human-popup", openerId:source.target_id}}});
  pages.set("human-popup", {url:"https://github.com", opener_target_id:source.target_id});
  const state = await host.request({op:"state"});
  const agent = state.tabs.find(tab => tab.url === "https://en.wikipedia.org");
  const human = state.tabs.find(tab => tab.url === "https://github.com");
  assert.equal(state._tab_openers?.[human.tab_id], undefined, "opener alone carries no actor evidence");
  assert.equal(state._tab_creation_actions?.[agent.tab_id], "action-agent");
  assert.equal(state._tab_creation_actions?.[human.tab_id], undefined);
  assert.equal(handlers.size, 3, "activation, lockdown and recovery observers remain until host shutdown");
}));

// MP-08/MP-10/MP-11: shared viewers observe one canonical tab, never change
// its pixel density or disturb an already attached native capture.
for (const first of [1,2]) test(`MP-08 mixed-DPR viewers keep native geometry when DPR${first} subscribes first`,()=>using(async({host,sent,chromium})=>{
 const before=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try {
  chromium.scale=2;const tab=await host.request({op:'open',url:'about:blank'});
  const command={op:'display_subscribe',tab_id:tab.tab_id,generation:tab.generation,codecs:['png'],bitrate:8000000};
  const owner=await host.request({...command,device_scale_factor:first});
  const metrics=sent.filter(c=>c.method==='Emulation.setDeviceMetricsOverride');
  const watcher=await host.request({...command,device_scale_factor:3-first});
  assert.equal(watcher.device_scale_factor,2,'actual canonical density is returned to the second viewer');
  assert.equal(host.displays.get(watcher.subscription_id).device_scale_factor,2);
  assert.notEqual(owner.subscription_id,watcher.subscription_id,'credits stay independent');
  assert.equal(host.generation,tab.generation,'observation preserves the document generation');
  assert.deepEqual(sent.filter(c=>c.method==='Emulation.setDeviceMetricsOverride'),metrics,'a watcher never changes the live viewport');
 } finally {if(before===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=before;}
}));

// MP-08/MP-10/MP-11: requested viewer DPR never selects native capture geometry.
test('MP-08/MP-10/MP-11 canonical native display accepts a DPR2 viewer',()=>using(async({host,sent})=>{
 const previous=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try {
  const {hostViewport}=await import('./kernel-browser-geometry.mjs'),viewport=hostViewport();
  const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
  const subscribed=await host.request({op:'display_subscribe',tab_id:tab.tab_id,generation:opened.generation,codecs:['png'],bitrate:8000000,device_scale_factor:2});
  const stream=host.displays.get(subscribed.subscription_id);
  assert.equal(subscribed.device_scale_factor,viewport.device_scale_factor);assert.equal(stream.device_scale_factor,viewport.device_scale_factor);
  assert.equal(stream.css_width,viewport.css_width);assert.equal(stream.css_height,viewport.css_height);
  assert.equal(sent.filter(c=>c.method==='Emulation.setDeviceMetricsOverride').length,0);
 } finally {if(previous===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=previous}
}));
test('MP-08/MP-10/MP-11 canonical native 1920 display accepts a DPR2 viewer',async()=>{
 const {spawnSync}=await import('node:child_process');
 const run=spawnSync(process.execPath,['--test','--test-name-pattern=^MP-08/MP-10/MP-11 canonical native display accepts',import.meta.filename],{env:{...process.env,NODE_TEST_CONTEXT:undefined,CHARIOX_BROWSER_DISPLAY_GEOMETRY:'1920x1080'},encoding:'utf8'});
 assert.match(run.stdout,/ok 1 - MP-08\/MP-10\/MP-11 canonical native display accepts/);assert.equal(run.status,0,run.stdout+run.stderr);
});

// MP-08/MP-10/MP-11: a client's state poll waits for every page's reconcile
// (~40 ms on a busy page); a keystroke arriving meanwhile must not queue behind it.
test("MP-08/MP-10/MP-11 a state poll in flight does not hold input", { timeout: 5000 }, () => using(async ({ host, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  host.browser.canReconcileConcurrently = () => true;
  const input = new PassThrough(), output = new PassThrough();
  const lines = readline.createInterface({ input: output }), waiters = new Map();
  lines.on("line", line => { const reply = JSON.parse(line); waiters.get(reply.id)?.resolve(reply); });
  const server = new BrowserControllerStdioServer({ input, output, scheduleRequest: request => scheduleHostRequest(request, host.browser),
    handleRequest: (request, options) => host.handle(request, options) });
  const running = server.run();
  const rpc = (id, method, params) => {
    const result = Promise.withResolvers(); waiters.set(id, result);
    input.write(`${JSON.stringify({ id, method, params })}\n`);
    return result.promise;
  };
  const reconcile = host.browser.reconcile, entered = Promise.withResolvers(), release = Promise.withResolvers();
  let held = true;
  host.browser.reconcile = async (...args) => {
    const observed = await reconcile(...args);
    if (held) { held = false; entered.resolve(); await release.promise; }
    return observed;
  };
  try {
    const state = rpc(1, "host.browser", { op: "state" });
    await entered.promise;
    const typed = await Promise.race([
      rpc(2, "host.browser", { op: "input", tab_id: opened.tab_id, generation: opened.generation,
        document_id: opened.tabs[0].document_id, input: { kind: "text", text: "a" } }),
      new Promise(resolve => setTimeout(() => resolve(null), 1000)),
    ]);
    assert.ok(typed?.ok, "input waited behind the state poll");
    assert.equal(sent.filter(call => call.method === "Input.insertText").length, 1);
    release.resolve();
    const polled = await state;
    assert.equal(polled.ok, true);
    assert.ok(polled.result._observation <= typed.result._observation, "the later input observation stays the newest");
  } finally { release.resolve(); input.end(); await running; lines.close(); output.end(); }
}));

test("MP-11 an older tab observation never replaces a newer one", () => using(async ({ host, pages }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  const reconcile = host.browser.reconcile, release = Promise.withResolvers();
  let first = true;
  host.browser.reconcile = async (...args) => {
    const observed = await reconcile(...args);
    if (first) { first = false; await release.promise; }
    return observed;
  };
  const older = host.reconcile();
  await new Promise(resolve => setImmediate(resolve));
  pages.set("target-new", { url: "about:blank" });
  const newer = await host.reconcile();
  release.resolve();
  const stale = await older;
  assert.equal(newer.tabs.length, 2);
  assert.equal(stale._observation, newer._observation, "a stale observation reports the current tabs");
  assert.equal(stale.tabs.length, 2);
  assert.equal(host.tabs.size, 2);
  assert.ok(host.tabs.has(opened.tab_id));
}));

test('MP-08/MP-10/MP-11 canonical mirror density does not depend on video', async()=>{
  const {hostViewport,displayGeometry}=await import('./kernel-browser-geometry.mjs');
  const beforeMirror=process.env.CHARIOX_KERNEL_BROWSER_MIRROR,beforeVideo=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;
  try {
    process.env.CHARIOX_KERNEL_BROWSER_MIRROR='1';process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='0';
    const mirrored=hostViewport();
    process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
    assert.deepEqual(mirrored,hostViewport(),'enabling video cannot change the shared browser density');
    assert.equal(mirrored.device_scale_factor,displayGeometry.dpr);
  } finally {
    if(beforeMirror===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_MIRROR;else process.env.CHARIOX_KERNEL_BROWSER_MIRROR=beforeMirror;
    if(beforeVideo===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=beforeVideo;
  }
});

test('MP-08/MP-10/MP-11: protected still captures keep the hidden render widget active without OS focus',()=>using(async({host,connection})=>{
 const opened=await host.request({op:'open',url:'https://en.wikipedia.org/'});
 let rendering=false,holds=0;
 host.browser.inputCapture.run=async(_connection,_session,operation)=>{assert.equal(rendering,false);rendering=true;holds++;try{return await operation()}finally{rendering=false}};
 connection.beforeSend=async method=>{if(method==='Page.captureScreenshot')assert.equal(rendering,true,'a hidden widget cannot paint without its existing transient capture lease');assert.notEqual(method,'Page.bringToFront')};
 const captured=await host.request({op:'screenshot',_capture_protection:true,tab_id:opened.tab_id,generation:opened.generation});
 assert(captured.data_base64);assert.equal(holds,1);assert.equal(rendering,false);
}));

// MP-08/MP-10 (owner A/B 2026-10-14, findings 6/8): a typed URL waited for full all-tab reconciles before
// and after the navigation (up to 25 s on a loading page), holding the mutation lane for the next one.
test('MP-08/MP-10 navigate answers at commit without all-tab reconciles', () => using(async ({ host, pages }) => {
  const opened = await host.request({ op: 'open', url: 'about:blank' });
  const browser = host.browser, reconcile = browser.reconcile;
  let reconciles = 0;
  browser.reconcile = async (...args) => { reconciles++; return reconcile(...args); };
  browser.navigate = async ({ target_id, url }) => { pages.set(target_id, { url, document_id: 'doc-next' }); return { target_id, document_id: 'doc-next', url }; };
  const result = await host.request({ op: 'navigate', tab_id: opened.tab_id, generation: opened.generation, url: 'https://example.com/next' });
  assert.equal(reconciles, 0, 'no all-tab reconcile on the navigation path');
  const tab = result.tabs.find(entry => entry.tab_id === opened.tab_id);
  assert.equal(tab.document_id, 'doc-next'); assert.equal(tab.url, 'https://example.com/next');
}));

// MP-08/MP-10 (review #1009 @ec754ee48 P2): a redirect commits another address; the navigation reply,
// the saved Tab and later tab-scoped reads report the committed URL, not the requested one.
test('MP-08/MP-10 navigate reports the committed URL after a redirect', () => using(async ({ host, pages }) => {
  const opened = await host.request({ op: 'open', url: 'about:blank' });
  const target = host.tabs.get(opened.tab_id).target_id;
  host.browser.navigate = async ({ target_id, url }) => { pages.set(target_id, { url: 'https://example.com/landed', document_id: 'doc-next' }); return { target_id, document_id: 'doc-next', url }; };
  const result = await host.request({ op: 'navigate', tab_id: opened.tab_id, generation: opened.generation, url: 'https://example.com/next' });
  assert.equal(result.tabs.find(entry => entry.tab_id === opened.tab_id).url, 'https://example.com/landed');
  assert.equal(host.tabs.get(opened.tab_id).url, 'https://example.com/landed');
  assert.equal((await host.displayTarget({ generation: opened.generation, tab_id: opened.tab_id })).url, 'https://example.com/landed');
  assert.equal(pages.get(target).document_id, 'doc-next');
}));

// r55: a selection note names its Tab and reads only that Tab live. The all-tab reconcile behind it waited for a
// parsing page's renderer (5 s CDP timeouts, 6-24 s per note) while the kernel held its backend for the request.
for (const op of ['note_selection']) test(`MP-08/MP-10/MP-11 ${op} reads only its own Tab`, () => using(async ({ host }) => {
  const opened = await host.request({ op: 'open', url: 'about:blank' });
  const browser = host.browser, reconcile = browser.reconcile;
  let reconciles = 0;
  browser.reconcile = async (...args) => { reconciles++; return reconcile(...args); };
  browser.observeNote = async () => ({ quote: 'q' });
  const result = await host.request({ op, tab_id: opened.tab_id, generation: opened.generation, observed_by: 'terminal:fixture' });
  assert.equal(reconciles, 0, 'no all-tab reconcile');
  assert.deepEqual(result.observation, { quote: 'q' });
}));

// MP-08/MP-10 (review #1009 P2): the kernel's navigate/subscribe preflight names its Tab and reads only that
// Tab live; an all-tab reconcile waits for every busy page and held the controller lock meanwhile.
test('MP-08/MP-10 a tab-scoped state preflight reads only its own Tab', () => using(async ({ host }) => {
  const opened = await host.request({ op: 'open', url: 'about:blank' });
  const browser = host.browser, reconcile = browser.reconcile;
  let reconciles = 0;
  browser.reconcile = async (...args) => { reconciles++; return reconcile(...args); };
  const state = await host.request({ op: 'state', tab_id: opened.tab_id, generation: opened.generation });
  assert.equal(reconciles, 0, 'no all-tab reconcile');
  assert.ok(state._observation > opened._observation, 'a newer observation for the kernel ledger');
  assert.ok(state.tabs.some(tab => tab.tab_id === opened.tab_id));
  await assert.rejects(host.request({ op: 'state', tab_id: 'host-tab-unknown', generation: opened.generation }), error => error.code === 'user_domain_stale_reference');
  await host.request({ op: 'state' }); assert.equal(reconciles, 1, 'a client state read keeps its reconcile');
}));

// MP-08/MP-10 (BROWSER-UX-14, native tab strip): the internal keepalive lives in a strip-less popup window, so
// Chrome's own tab strip shows only the person's Tabs; the launch tab is closed and new Tabs open in a tabbed window.
test('MP-08/MP-10 the keepalive is a popup window outside the native tab strip', () => using(async ({ host, connection, sent }) => {
  const opened = await host.request({ op: 'open', url: 'about:blank' });
  assert.equal(fixture.popups.length, 1, 'one keepalive popup');
  assert.equal(host.keepaliveTarget, fixture.popups[0]);
  assert.equal(sent.find(call => call.method === 'Runtime.evaluate' && call.params.expression.includes("'popup")).params.userGesture, true);
  const { targetInfos } = await connection.send('Target.getTargets');
  assert.deepEqual(targetInfos.map(t => t.targetId).sort(), [fixture.popups[0], host.tabs.get(opened.tab_id).target_id].sort(), 'the launch tab is closed');
  assert.ok(!opened.tabs.some(tab => tab.target_id === fixture.popups[0]));
}));

// r54 (first video 8-13 s after native_admitted): any host browser op that takes over a second is named in the timing log.
test('MP-08/MP-10 opt-in timing names a slow host browser op and only a slow one', () => using(async ({ host }, root) => {
  const before = process.env.CHARIOX_BROWSER_DISPLAY_TIMING; process.env.CHARIOX_BROWSER_DISPLAY_TIMING = '1';
  try {
    const request = host.request.bind(host);
    host.request = async params => params.op === 'display_credit' ? new Promise(resolve => setTimeout(resolve, 1100, {})) : request(params);
    await host.handle({ id: 1, method: 'host.browser', params: { op: 'display_credit' } });
    await host.handle({ id: 2, method: 'host.browser', params: { op: 'screenshot', format: 'png' } }).catch(() => {});
    host.timing.flush();
    const stages = (await readFile(path.join(root, 'display-timing.jsonl'), 'utf8')).trim().split('\n').map(line => JSON.parse(line).stage);
    assert.deepEqual(stages.filter(stage => stage.startsWith('host_op_slow')), ['host_op_slow display_credit']);
  } finally { if (before === undefined) delete process.env.CHARIOX_BROWSER_DISPLAY_TIMING; else process.env.CHARIOX_BROWSER_DISPLAY_TIMING = before; }
}));

// MP-08/MP-10: opt-in timing names Tab lifecycle ops with fixed labels (New tab / typed URL attribution).
test('MP-08/MP-10 opt-in timing labels Tab lifecycle ops', () => using(async ({ host }, root) => {
  const before = process.env.CHARIOX_BROWSER_DISPLAY_TIMING; process.env.CHARIOX_BROWSER_DISPLAY_TIMING = '1';
  try {
    const opened = (await host.handle({ id: 1, method: 'host.browser', params: { op: 'open', url: 'about:blank' } })).result;
    await host.handle({ id: 2, method: 'host.browser', params: { op: 'state', tab_id: opened.tab_id, generation: opened.generation } });
    host.timing.flush();
    const stages = (await readFile(path.join(root, 'display-timing.jsonl'), 'utf8')).trim().split('\n').map(line => JSON.parse(line).stage);
    assert.ok(stages.includes('host_op open')); assert.ok(stages.includes('host_op state tab'));
  } finally { if (before === undefined) delete process.env.CHARIOX_BROWSER_DISPLAY_TIMING; else process.env.CHARIOX_BROWSER_DISPLAY_TIMING = before; }
}));

// MP-08/MP-10 (real site html.spec.whatwg.org): a viewer's mirror credit on a huge page was cancelled by its
// caller and the host stopped the whole browser (every Tab) as if input had been cut mid-dispatch. A cancelled
// read-only observation leaves the browser running; cancelled input still stops it (b220 tests above).
test('MP-08/MP-10 a cancelled read-only observation does not stop the browser', () => using(async ({ host, chromium }) => {
  const mirror = process.env.CHARIOX_KERNEL_BROWSER_MIRROR; process.env.CHARIOX_KERNEL_BROWSER_MIRROR = '1';
  try {
  const opened = await host.request({ op: 'open', url: 'about:blank' });
  for (const params of [{ op: 'state' }, { op: 'mirror_next', subscription_id: 'gone', generation: opened.generation, after_sequence: 0, wait_ms: 2000 }, { op: 'snapshot', tab_id: opened.tab_id, generation: opened.generation }]) {
    const cancel = new AbortController();
    const pending = host.handle({ id: 7, method: 'host.browser', params }, { signal: cancel.signal });
    cancel.abort(); // the caller goes away while the read is in flight
    const reply = await pending;
    assert.equal(reply.error?.code, 'browser_action_cancelled', params.op);
    assert.ok(chromium.child && host.browser, `${params.op}: the browser keeps running`);
  }
  assert.equal((await host.request({ op: 'state' })).tabs.length, 1);
  } finally { if (mirror === undefined) delete process.env.CHARIOX_KERNEL_BROWSER_MIRROR; else process.env.CHARIOX_KERNEL_BROWSER_MIRROR = mirror; }
}));

// MP-08/MP-10 (live overflow drill): Chrome lists targets most-recently-active first, so the strip reordered
// on every activation and a New tab could land before existing Tabs. Tabs keep their order; new ones append.
test('MP-08/MP-10 Tabs keep their order and new Tabs are appended', () => using(async ({ host }) => {
  const a = await host.request({ op: 'open', url: 'https://a.example/' });
  const b = await host.request({ op: 'open', url: 'https://b.example/' });
  const browser = host.browser, reconcile = browser.reconcile;
  browser.reconcile = async (...args) => { const state = await reconcile(...args); return { ...state, tabs: [...state.tabs].reverse() }; };
  const c = await host.request({ op: 'open', url: 'https://c.example/' });
  const order = (await host.request({ op: 'state' })).tabs.map(tab => tab.tab_id);
  assert.deepEqual(order, [a.tab_id, b.tab_id, c.tab_id]);
}));

// MP-11 (BROWSER-UX-14 lockdown): behind the kernel's managed policies, the host closes a Tab
// that reaches a blocked scheme without the policy's refusal page.
test('MP-11 the host allows downloads and closes Tabs on blocked schemes only', () => using(async ({ host, handlers, sent, connection, pages }) => {
  await host.request({ op: 'open', url: 'about:blank' });
  // Miguel 2026-10-15: no CDP download deny (r8 live drill: it also cancelled the Web Store package
  // of an extension the person adds); Chrome's settings/internal pages and DevTools stay open.
  assert(!sent.some(call => call.method === 'Browser.setDownloadBehavior'), 'no CDP download deny');
  const send = connection.send;
  connection.send = async (method, params, session) => method === 'DOM.getDocument'
    ? { root: { nodeId: 1, documentURL: session === 'refused' ? 'chrome-error://chromewebdata/' : params.url } }
    : method === 'Target.attachToTarget' ? { sessionId: params.targetId === 'policy-refused' ? 'refused' : 'live' } : send(method, params, session);
  const emit = async (method, targetInfo) => { for (const handler of [...handlers]) handler({ method, params: { targetInfo } }); await new Promise(resolve => setTimeout(resolve, 10)); };
  const before = sent.length, closed = () => sent.slice(before).filter(call => call.method === 'Target.closeTarget').map(call => call.params.targetId);
  await emit('Target.targetInfoChanged', { targetId: 'settings', type: 'page', url: 'chrome://settings/' });
  await emit('Target.targetCreated', { targetId: 'devtools', type: 'page', url: 'devtools://devtools/bundled/devtools_app.html' });
  await emit('Target.targetInfoChanged', { targetId: 'file', type: 'page', url: 'file:///etc/hostname' });
  await emit('Target.targetInfoChanged', { targetId: 'policy-refused', type: 'page', url: 'chrome://extensions/' });
  await emit('Target.targetCreated', { targetId: 'ntp', type: 'page', url: 'chrome://newtab/' });
  await emit('Target.targetCreated', { targetId: 'omnibox', type: 'browser_ui', url: 'chrome://omnibox-popup.top-chrome/' });
  await emit('Target.targetInfoChanged', { targetId: 'site', type: 'page', url: 'https://example.com/' });
  assert.deepEqual(closed(), ['file']);
  assert(pages.size > 0);
}));

// MP-08/MP-11 (Miguel 19:54Z): the kernel's opener ledger names agent Tabs; the host groups them by target.
test('MP-08 agent Tabs named by the kernel are grouped by target; the rest stay ungrouped', () => using(async ({ host }) => {
  const a = await host.request({ op: 'open', url: 'https://example.com/' });
  const b = await host.request({ op: 'open', url: 'https://example.org/' });
  const synced = [];
  host.tabGroups = { sync: async groups => synced.push(groups) };
  const reply = await host.handle({ id: 1, method: 'host.browser', params: { op: 'state', _tab_groups: [{ tab_id: b.tab_id, label: 'Mara' }, { tab_id: 'gone', label: 'Ghost' }] } });
  assert.equal(reply.ok, true);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(synced, [[{ label: 'Mara', target_ids: [host.tabs.get(b.tab_id).target_id] }]]);
  assert.notEqual(host.tabs.get(a.tab_id).target_id, host.tabs.get(b.tab_id).target_id);
}));

// MP-08/MP-10/MP-11 (BROWSER-UX-14): a viewer learns the band's window epoch and the
// natively active Tab from state; it never names a window itself.
test('MP-08 state projects the bound Chrome window epoch and its natively active Tab', () => using(async ({ host }) => {
  const display = process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;
  process.env.CHARIOX_KERNEL_BROWSER_DISPLAY = '1';
  try {
  await host.request({ op: 'open', url: 'https://example.com/' });
  const b = await host.request({ op: 'open', url: 'https://example.org/' });
  host.chromeWindows.bound = null;
  assert.equal((await host.request({ op: 'state' })).chrome_window, null, 'no binding, no band');
  let current = true;
  host.chromeWindows.bound = { tabbed: { window_generation: 7, isCurrent: () => current }, auxiliary: [] };
  host.tabGroups = { active: async () => host.tabs.get(b.tab_id).target_id };
  // MP-11: without the managed lockdown a page can enter same-size fullscreen and its pixels reach the band crop.
  host.chromium.policy = 'unavailable';
  assert.equal((await host.request({ op: 'state' })).chrome_window, null, 'no band without the managed lockdown');
  host.chromium.policy = 'managed';
  assert.deepEqual((await host.request({ op: 'state' })).chrome_window, { window_generation: 7, active_tab_id: b.tab_id, auxiliary: [] });
  host.chromeWindows.auxiliaryState = () => [{ auxiliary_id: 4, x: 1000, y: 70, width: 248, height: 400 }];
  assert.deepEqual((await host.request({ op: 'state' })).chrome_window.auxiliary, [{ auxiliary_id: 4, x: 1000, y: 70, width: 248, height: 400 }], 'protocol 530: the band window\'s popups');
  delete host.chromeWindows.auxiliaryState;
  process.env.CHARIOX_KERNEL_BROWSER_DISPLAY = '0';
  assert.equal((await host.request({ op: 'state' })).chrome_window, null, 'no band is offered when display is disabled');
  process.env.CHARIOX_KERNEL_BROWSER_DISPLAY = '1';
  current = false;
  assert.equal((await host.request({ op: 'state' })).chrome_window, null, 'a retired window is not offered');
  } finally { if (display === undefined) delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY; else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY = display; }
}));

// MP-08/MP-10/MP-11 (review #1016 P1): Chrome's own strip, wheel or shortcuts (or any actor) can show another
// existing Tab in the one window. A whole-page native source of the previous Tab must stop publishing and
// accepting input before then, or Tab B's pixels (e.g. a Vault-filled field outside A's masks) go out as A's.
test('MP-11 an existing-Tab switch retires the previous Tab\'s whole-page native capture', () => using(async ({ host }) => {
  const { LinuxCapture } = await import('./kernel-browser-native.mjs');
  const a = await host.request({ op: 'open', url: 'https://example.com/' });
  const b = await host.request({ op: 'open', url: 'https://example.org/' });
  const target = id => host.tabs.get(id).target_id;
  let active = target(a.tab_id);
  host.tabGroups = { active: async () => active };
  await host.request({ op: 'state' });
  const stream = () => {
    const claim = host.foreground.claim(a.tab_id), events = [];
    const source = Object.assign(Object.create(LinuxCapture.prototype), { closed: false, allowed: () => host.foreground.holds(a.tab_id, claim), close: async () => { events.push('retired'); } });
    host.compositors.set(a.tab_id, { ready: Promise.resolve(source), source, document: host.tabs.get(a.tab_id).document_id });
    return { source, events };
  };
  // 1. The person clicks Tab B in the band: retired before the click reaches Chrome.
  let { source, events } = stream();
  const band = { surface: 'browser_chrome', isCurrent: () => true, click: async () => { events.push(source.allowed() ? 'click while A publishes' : 'click'); active = target(b.tab_id); return true; } };
  await host.arbiter.input({ stream: { regionSource: band }, command: { tab_id: a.tab_id, document_id: host.tabs.get(a.tab_id).document_id, generation: host.generation, input: { kind: 'click', x: 300, y: 20 } } });
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(events, ['click', 'retired']);
  assert.equal(host.compositors.has(a.tab_id), false);
  // 2. Another actor activates B (Ctrl+Tab in Chrome, a page's window.focus): the next observation retires A's source.
  active = target(a.tab_id); await host.request({ op: 'state' });
  ({ source, events } = stream());
  active = target(b.tab_id);
  await host.request({ op: 'state' });
  assert.equal(source.allowed(), false, 'A\'s claim is gone once B is the active Tab');
  assert.deepEqual(events, ['retired']);
}));

// Review #1016 @f0ccdb5c2 P1: a switch outside the browser's own paths (the Computer surface's Ctrl+Tab)
// does not reconcile. Before any state poll, A's whole-page readback and native input must already stop.
test('MP-11 an out-of-band Tab switch stops whole-page native readback and input before any state poll', () => using(async ({ host }) => {
  const { LinuxCapture } = await import('./kernel-browser-native.mjs');
  const a = await host.request({ op: 'open', url: 'https://example.com/' });
  const b = await host.request({ op: 'open', url: 'https://example.org/' });
  const tabA = host.tabs.get(a.tab_id);
  let active = tabA.target_id;
  host.tabGroups = { active: async () => active };
  await host.request({ op: 'state' });
  assert.equal(await host.nativeFront(tabA), true);
  active = host.tabs.get(b.tab_id).target_id; // Ctrl+Tab through the Computer surface: no reconcile
  assert.equal(await host.nativeFront(tabA), false, 'live authority, not the last observation');
  host.tabGroups = null;
  assert.equal(await host.nativeFront(tabA), false, 'no live answer: fail closed');
  host.tabGroups = { active: async () => active };
  // The real publish loop refuses the readback before masks, encoder offers or delivery.
  const published = [];
  const source = Object.assign(Object.create(LinuxCapture.prototype), { closed: false, attested: true, listeners: new Set([frame => published.push(frame)]),
    connection:{},timing:()=>{},regionRevision:0,regions:{guard:{},beforeAt:0,regions:async()=>[],retire(){}},
    pending: { captured_ms:1,get pixels(){assert.fail('inactive Tab pixels must not be read');},release() {} }, valid: () => !source.closed, validateCurrent: () => host.nativeFront(tabA), fence() { this.closed = true; this.pending = null; } });
  await source.publish();
  assert.equal(source.closed, true); assert.deepEqual(published, []);
  // Native input of the retired foreground falls back instead of clicking into Tab B.
  const clicks = [];
  const owned = Object.assign(Object.create(LinuxCapture.prototype), { attested: true, closed: false, valid: () => true, click: (x, y) => (clicks.push([x, y]), true) });
  host.compositors.set(a.tab_id, { ready: Promise.resolve(owned), source: owned, document: tabA.document_id });
  host.displays.set('whole-a', { subscription_id: 'whole-a', tab_id: a.tab_id, observed_by: 'terminal:test', expires: Date.now() + 60_000, close: async () => {} });
  await host.handle({ id: 9, method: 'host.browser', params: { op: 'input', tab_id: a.tab_id, generation: a.generation, document_id: tabA.document_id, input: { kind: 'click', x: 5, y: 5 }, _display_input: true, observed_by: 'terminal:test' } });
  assert.deepEqual(clicks, [], 'no native click while Tab B is in front');
  host.displays.delete('whole-a'); host.compositors.delete(a.tab_id);
}));

// MP-08/MP-10/MP-11: authorised input is not model-visible data. A DOM mirror does not need a video lease to drag.
test('MP-08/MP-10/MP-11 authorised page drags work for agents and mirror-only viewers', () => using(async ({ host, sent }) => {
  const opened = await host.request({ op: 'open', url: 'https://example.com/' }), tab = host.tabs.get(opened.tab_id);
  const drag = extra => host.request({ op: 'input', tab_id: opened.tab_id, generation: host.generation, document_id: tab.document_id, input: { kind: 'drag', x: 10, y: 10, to_x: 200, to_y: 12 }, ...extra });
  const presses = () => sent.filter(x => x.method === 'Input.dispatchMouseEvent' && x.params?.type === 'mousePressed').length;
  await drag({ _agent_input: true });
  await drag({ focused_agent: 'agent-a', _display_input: true, observed_by: 'terminal:test' });
  await drag({ _display_input: true, observed_by: 'terminal:test' });
  assert.equal(presses(), 3, 'the existing input grant and document fence admit every authorised caller');
  await assert.rejects(host.request({op:'input',tab_id:opened.tab_id,generation:host.generation,document_id:'retired',input:{kind:'drag',x:10,y:10,to_x:200,to_y:12}}), error=>error.code==='user_domain_stale_reference');
  assert.equal(presses(), 3, 'retired document coordinates are still refused before dispatch');
}));

// MP-11 (Miguel 2026-10-15): the person manages extensions as in any Chrome; an agent cannot open the
// Web Store or chrome://extensions, nor act on such a Tab (installs come later through /sudo).
test('MP-11 agents cannot reach extension management; the person can', () => using(async ({ host }) => {
  const refused = { message: /extension_management_not_granted/ };
  for (const flags of [{ _agent_input: true }, { focused_agent: 'agent-a' }]) {
    await assert.rejects(host.handle({ id: 1, method: 'host.browser', params: { op: 'open', url: 'https://chromewebstore.google.com/detail/x/abc', ...flags } }).then(r => { if (!r.ok) throw new Error(r.error?.message ?? r.error); return r; }), refused);
    await assert.rejects(host.handle({ id: 2, method: 'host.browser', params: { op: 'open', url: 'chrome://extensions/', ...flags } }).then(r => { if (!r.ok) throw new Error(r.error?.message ?? r.error); return r; }), refused);
  }
  const mine = await host.request({ op: 'open', url: 'https://example.com/' });
  const agent = { _agent_input: true };
  await assert.rejects(host.handle({ id: 3, method: 'host.browser', params: { op: 'navigate', tab_id: mine.tab_id, generation: mine.generation, url: 'chromewebstore.google.com', ...agent } }).then(r => { if (!r.ok) throw new Error(r.error?.message ?? r.error); return r; }), refused);
  await host.request({ op: 'navigate', tab_id: mine.tab_id, generation: mine.generation, url: 'https://chromewebstore.google.com/' });
  const tab = host.tabs.get(mine.tab_id);
  await assert.rejects(host.handle({ id: 4, method: 'host.browser', params: { op: 'input', tab_id: mine.tab_id, generation: host.generation, document_id: tab.document_id, input: { kind: 'click', x: 10, y: 10 }, ...agent } }).then(r => { if (!r.ok) throw new Error(r.error?.message ?? r.error); return r; }), refused, 'an agent cannot click Add to Chrome');
}));

// MP-08/MP-10/MP-11 (protocol 530/138): the public Browser state keys are the hashed
// Rust fixture's; a new key needs a protocol bump and a new fixture.
test('MP-08 public Browser state keys match the protocol 630 fixture', () => using(async ({ host }) => {
  const fixture = JSON.parse(await readFile(new URL('../../src/local/api/tests/protocol_shapes/browser-chrome-ui-530.json', import.meta.url), 'utf8'));
  const display = process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;
  process.env.CHARIOX_KERNEL_BROWSER_DISPLAY = '1';
  try {
    const tab = await host.request({ op: 'open', url: 'https://example.com/' });
    host.chromeWindows.bound = { tabbed: { window_generation: 3, isCurrent: () => true }, auxiliary: [] };
    host.tabGroups = { active: async () => host.tabs.get(tab.tab_id).target_id };
    host.chromium.policy = 'managed';
    const state = await host.request({ op: 'state' });
    const dialogs = JSON.parse(await readFile(new URL('../../src/local/api/tests/protocol_shapes/browser-dialog-630.json', import.meta.url), 'utf8'));
    assert.deepEqual(Object.keys(state).filter(key => !key.startsWith('_')).sort(), dialogs.state_keys);
    assert.deepEqual(Object.keys(state.chrome_window).sort(), fixture.chrome_window_keys);
    host.chromeWindows.auxiliaryState = () => [{ auxiliary_id: 1, x: 0, y: 0, width: 2, height: 2 }];
    assert.deepEqual(Object.keys((await host.request({ op: 'state' })).chrome_window.auxiliary[0]).sort(), fixture.auxiliary_keys);
    delete host.chromeWindows.auxiliaryState;
    assert(fixture.browser_lockdown.includes(state.browser_lockdown));
  } finally { if (display === undefined) delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY; else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY = display; }
}));

// MP-11 (Ubuntu >= 23.10 without the Setup's AppArmor profile): a browser without its
// managed policies says so to every viewer, not only in the kernel log.
test('MP-11 state reports whether the managed browser policies apply', () => using(async ({ host }) => {
  await host.request({ op: 'open', url: 'https://example.com/' });
  host.chromium.policy = 'managed';
  assert.equal((await host.request({ op: 'state' })).browser_lockdown, 'managed');
  host.chromium.policy = 'unavailable';
  assert.equal((await host.request({ op: 'state' })).browser_lockdown, 'unavailable');
}));

// MP-08/MP-10/MP-11: native display density never applies to the owned 1x Computer desktop.
test('MP-08 Browser display keeps the owned Computer desktop page at desktop density', () => {
  const before = process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;
  process.env.CHARIOX_KERNEL_BROWSER_DISPLAY = '1';
  try {
    const host = chromium => new KernelBrowserHost('/nonexistent', { chromium, browserFactory: () => null });
    assert.equal(host({}).viewport().device_scale_factor, 2);
    assert.equal(host({ desktop: { binding() {} } }).viewport().device_scale_factor, 1);
  } finally {
    if (before === undefined) delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY; else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY = before;
  }
});

// MP-08/MP-10/MP-11: projected state and capture use one canonical desktop.
test('MP-08 owned desktop state reports the viewport used by the actual controller',()=>{
 const before=process.env.CHARIOX_KERNEL_BROWSER_MIRROR;process.env.CHARIOX_KERNEL_BROWSER_MIRROR='1';
 const host=new KernelBrowserHost('/nonexistent',{chromium:{desktop:{binding(){}}},browserFactory:()=>null});
 try{assert.deepEqual(host.tabState().viewport,host.viewport())}
 finally{host.mirror.clear();if(before===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_MIRROR;else process.env.CHARIOX_KERNEL_BROWSER_MIRROR=before}
});

// MP-08/MP-10: a mirror-only private renderer keeps its native density.
test('MP-08 private mirrored controller viewport does not depend on video',()=>{
 const mirror=process.env.CHARIOX_KERNEL_BROWSER_MIRROR,video=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;
 process.env.CHARIOX_KERNEL_BROWSER_MIRROR='1';process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='0';
 const host=new KernelBrowserHost('/nonexistent',{chromium:{},browserFactory:()=>null});
 try{
  const viewport=host.viewport();assert.equal(viewport.device_scale_factor,2);
  process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';assert.deepEqual(host.viewport(),viewport);
 }finally{host.mirror.clear();if(mirror===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_MIRROR;else process.env.CHARIOX_KERNEL_BROWSER_MIRROR=mirror;if(video===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=video}
});

test('MP-08/MP-11 native band lifetime survives closing its original page Tab',()=>using(async({host})=>{
 const a=await host.request({op:'open',url:'about:blank'});await host.request({op:'open',url:'about:blank'});
 let closed=0;
 host.displays.set('window-band',{subscription_id:'window-band',tab_id:a.tab_id,expires:Date.now()+60_000,regionTarget:{kind:'browser_chrome'},invalidate(){},close:async()=>{closed++;}});
 await host.request({op:'close',tab_id:a.tab_id,generation:a.generation});
 assert.equal(closed,0,'only the owner window retirement closes the band');assert(host.displays.has('window-band'));
}));

// MP-08/MP-10/MP-11 step4: real credit/DisplayStream path, not empty-credit shortcut.
test('MP-10 delivered native frames perform zero CDP calls after document admission',()=>using(async({host,sent})=>{
 const {displayCredit}=await import('./kernel-browser-display-credit.mjs');
 const {DisplayStream}=await import('./kernel-browser-display.mjs');
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 await host.displayTarget({tab_id:tab.tab_id,generation:opened.generation},true);
 const source={native:true,attested:true,regionRevision:0,policy:host.protection,closed:false,allowed:()=>true,sample:()=>sample,valid:()=>true,close:async()=>{}};
 let sample={serial:1,document_id:tab.document_id,tab_id:tab.tab_id,raw:{},width:1280,height:800,motion:true,data_base64:'signature',encoded:{key:true,data_base64:'YWJj'}};
 const stream=new DisplayStream({subscription_id:'delivered',tab_id:tab.tab_id,codec:'avc1.420033',device_scale_factor:1,bitrate:8_000_000,dependencies:true},{encoder:{close:async()=>{}},wait:async()=>{}});
 const refiner={request:()=>null,invalidate(){},close:async()=>{}};
 const producer={source,frames:[sample],take:()=>producer.frames.shift(),feedback(){},retireUnsent(){},invalidate(){},close:async()=>{}};
 Object.assign(stream,{compositorRegionRevision:0,refiner,refinerDocument:tab.document_id,producer});
 host.compositors.set(tab.tab_id,{document:tab.document_id,source,ready:Promise.resolve(source)});
 // The first frame has no base: its recovery invalidation must keep this pre-encoded key available.
 stream.invalidate=()=>{stream.previous=null;stream.exact=false;stream.document_id=null;};
 try{
  const before=sent.length;
  for(let serial=1;serial<=3;serial++){
   sample={...sample,serial,data_base64:`signature${serial}`};producer.frames=[sample];
   const reply=await displayCredit(host,stream,{generation:opened.generation,after_sequence:stream.sequence,push:{congested:false}});
   assert.equal(reply.frame_sent,true);assert.equal(reply.display_frame.kind,'video');
  }
  assert.deepEqual(sent.slice(before).map(r=>r.method),[],'MP-10: zero calls per delivered frame');
 }finally{await stream.close();}
}));

for(const change of ['navigation','protection'])test(`MP-11 ${change} between native readback and commit discards the frame`,()=>using(async({host,handlers,pages})=>{
 const {displayCredit}=await import('./kernel-browser-display-credit.mjs');
 const {DisplayStream}=await import('./kernel-browser-display.mjs');
 const opened=await host.request({op:'open',url:'about:blank'}),tab=host.tabs.get(opened.tab_id);
 await host.displayTarget({tab_id:tab.tab_id,generation:opened.generation},true);
 const sample={serial:1,document_id:tab.document_id,tab_id:tab.tab_id,raw:{},width:1280,height:800,motion:true,data_base64:'signature',encoded:{key:true,data_base64:'YWJj'}};
 const source={native:true,attested:true,regionRevision:0,policy:host.protection,closed:false,allowed:()=>true,sample:()=>sample,valid:()=>true,close:async()=>{}};
 const producer={source,frames:[sample],feedback(){},retireUnsent(){},invalidate(){},close:async()=>{},take:()=>{
  if(change==='navigation'){
   pages.get(tab.target_id).document_id='next';
   for(const handler of handlers)handler({method:'Page.frameNavigated',sessionId:`session-${tab.target_id}`,params:{frame:{id:'frame',loaderId:'next'}}});
  }else source.regionRevision++;
  return sample;
 }};
 const stream=new DisplayStream({subscription_id:'retired',tab_id:tab.tab_id,codec:'avc1.420033',device_scale_factor:1,bitrate:8_000_000,dependencies:true},{encoder:{close:async()=>{}},wait:async()=>{}});
 Object.assign(stream,{compositorRegionRevision:0,refinerDocument:tab.document_id,refiner:{request:()=>null,invalidate(){},close:async()=>{}},producer});
 stream.invalidate=()=>{stream.previous=null;stream.exact=false;stream.document_id=null;};
 host.compositors.set(tab.tab_id,{document:tab.document_id,source,ready:Promise.resolve(source)});
 try{const reply=await displayCredit(host,stream,{generation:opened.generation,after_sequence:0,push:{congested:false}});assert.equal(reply.frame_sent,false);assert.equal(stream.sequence,0,'no stale frame committed');}finally{await stream.close();}
}));

for(const completion of ['Page.frameStoppedLoading','Page.frameNavigated'])test(`MP-10 pending native load is transient and resumes after ${completion}`,()=>using(async({host,sent,handlers})=>{
 const {displayCredit}=await import('./kernel-browser-display-credit.mjs');
 const {DisplayStream}=await import('./kernel-browser-display.mjs');
 const opened=await host.request({op:'open',url:'about:blank'}),tab=host.tabs.get(opened.tab_id);
 await host.displayTarget({tab_id:tab.tab_id,generation:opened.generation},true);
 const source={native:true,attested:true,regionRevision:0,policy:host.protection,closed:false,allowed:()=>true,sample:()=>sample,valid:()=>true,close:async()=>{}};
 let sample={serial:1,document_id:tab.document_id,tab_id:tab.tab_id,raw:{},width:1280,height:800,motion:true,data_base64:'signature',encoded:{key:true,data_base64:'YWJj'}};
 const stream=new DisplayStream({subscription_id:'delivered',tab_id:tab.tab_id,codec:'avc1.420033',device_scale_factor:1,bitrate:8_000_000,dependencies:true},{encoder:{close:async()=>{}},wait:async()=>{}});
 const refiner={request:()=>null,invalidate(){},close:async()=>{}};
 const producer={source,frames:[sample],take:()=>producer.frames.shift(),feedback(){},retireUnsent(){},invalidate(){},close:async()=>{}};
 Object.assign(stream,{compositorRegionRevision:0,refiner,refinerDocument:tab.document_id,producer});
 host.compositors.set(tab.tab_id,{document:tab.document_id,source,ready:Promise.resolve(source)});
 // The first frame has no base: its recovery invalidation must keep this pre-encoded key available.
 stream.invalidate=()=>{stream.previous=null;stream.exact=false;stream.document_id=null;};
 try{
  const emit=(method,params)=>{for(const handler of handlers)handler({method,params,sessionId:`session-${tab.target_id}`});};
  emit('Page.frameStartedLoading',{frameId:'frame'});
  for(let credit=0;credit<8;credit++){
   const reply=await displayCredit(host,stream,{generation:opened.generation,after_sequence:stream.sequence,push:{congested:false}});
   assert.equal(reply.frame_sent,false);assert.equal(stream.sequence,0);
  }
  emit(completion,completion==='Page.frameNavigated'?{frame:{id:'frame',loaderId:tab.document_id}}:{frameId:'frame'});
  await new Promise(resolve=>setImmediate(resolve));
  const before=sent.length;
  for(let serial=1;serial<=3;serial++){
   sample={...sample,serial,data_base64:`signature${serial}`};producer.frames=[sample];
   const reply=await displayCredit(host,stream,{generation:opened.generation,after_sequence:stream.sequence,push:{congested:false}});
   assert.equal(reply.frame_sent,true);assert.equal(reply.display_frame.kind,'video');
  }
  assert.deepEqual(sent.slice(before).map(r=>r.method),[],'MP-10: zero calls per delivered frame');
 }finally{await stream.close();}
}));

// MP-11 (BROWSER-UX-14 lockdown): behind the kernel's managed policies, the host closes a Tab
// that reaches a blocked scheme without the policy's refusal page.
test('MP-11 the host allows downloads and closes Tabs on blocked schemes only', () => using(async ({ host, handlers, sent, connection, pages }) => {
  await host.request({ op: 'open', url: 'about:blank' });
  // Miguel 2026-10-15: no CDP download deny (r8 live drill: it also cancelled the Web Store package
  // of an extension the person adds); Chrome's settings/internal pages and DevTools stay open.
  assert(!sent.some(call => call.method === 'Browser.setDownloadBehavior'), 'no CDP download deny');
  const send = connection.send;
  connection.send = async (method, params, session) => method === 'DOM.getDocument'
    ? { root: { nodeId: 1, documentURL: session === 'refused' ? 'chrome-error://chromewebdata/' : params.url } }
    : method === 'Target.attachToTarget' ? { sessionId: params.targetId === 'policy-refused' ? 'refused' : 'live' } : send(method, params, session);
  const emit = async (method, targetInfo) => { for (const handler of [...handlers]) handler({ method, params: { targetInfo } }); await new Promise(resolve => setTimeout(resolve, 10)); };
  const before = sent.length, closed = () => sent.slice(before).filter(call => call.method === 'Target.closeTarget').map(call => call.params.targetId);
  await emit('Target.targetInfoChanged', { targetId: 'settings', type: 'page', url: 'chrome://settings/' });
  await emit('Target.targetCreated', { targetId: 'devtools', type: 'page', url: 'devtools://devtools/bundled/devtools_app.html' });
  await emit('Target.targetInfoChanged', { targetId: 'file', type: 'page', url: 'file:///etc/hostname' });
  await emit('Target.targetInfoChanged', { targetId: 'policy-refused', type: 'page', url: 'chrome://extensions/' });
  await emit('Target.targetCreated', { targetId: 'ntp', type: 'page', url: 'chrome://newtab/' });
  await emit('Target.targetCreated', { targetId: 'omnibox', type: 'browser_ui', url: 'chrome://omnibox-popup.top-chrome/' });
  await emit('Target.targetInfoChanged', { targetId: 'site', type: 'page', url: 'https://example.com/' });
  assert.deepEqual(closed(), ['file']);
  assert(pages.size > 0);
}));

// MP-08/MP-11 (Miguel 19:54Z): the kernel's opener ledger names agent Tabs; the host groups them by target.
test('MP-08 agent Tabs named by the kernel are grouped by target; the rest stay ungrouped', () => using(async ({ host }) => {
  const a = await host.request({ op: 'open', url: 'https://example.com/' });
  const b = await host.request({ op: 'open', url: 'https://example.org/' });
  const synced = [];
  host.tabGroups = { sync: async groups => synced.push(groups) };
  const reply = await host.handle({ id: 1, method: 'host.browser', params: { op: 'state', _tab_groups: [{ tab_id: b.tab_id, label: 'Mara' }, { tab_id: 'gone', label: 'Ghost' }] } });
  assert.equal(reply.ok, true);
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.deepEqual(synced, [[{ label: 'Mara', target_ids: [host.tabs.get(b.tab_id).target_id] }]]);
  assert.notEqual(host.tabs.get(a.tab_id).target_id, host.tabs.get(b.tab_id).target_id);
}));

// MP-08/MP-10/MP-11: error strings must retain a specific safe cause to the Rust/client boundary.
test('Browser replies distinguish observation timeout from a retired subscription without exposing page data', () => using(async ({host}) => {
  const failures=[
    [Object.assign(new Error('browser CDP method Page.captureScreenshot timed out after 5000ms'),{code:'browser_cdp_timeout'}), /browser_cdp_timeout Page.captureScreenshot/],
    [new Error('MP-11: stale or foreign mirror'), /mirror_subscription_retired/],
    [Object.assign(new Error('private-page-canary'),{mirrorReason:'occluded_node'}), /occluded_node/],
  ];
  for(const [error,reason] of failures){
    host.request=async()=>{throw error};
    const reply=await host.handle({id:'cause',method:'host.browser',params:{op:'mirror_next'}});
    assert.equal(reply.error.code,'kernel_browser_failed');assert.match(reply.error.message,reason);
    assert(!reply.error.message.includes('private-page-canary'));
  }
}));

test("MP-08/MP-10/MP-11 native subscription readiness does not hold input", { timeout: 5000 }, () => using(async ({ host, sent }) => {
  const opened = await host.request({ op: "open", url: "about:blank" });
  host.browser.canReconcileConcurrently = () => true;
  const input = new PassThrough(), output = new PassThrough();
  const lines = readline.createInterface({ input: output }), waiters = new Map();
  lines.on("line", line => { const reply = JSON.parse(line); waiters.get(reply.id)?.resolve(reply); });
  const server = new BrowserControllerStdioServer({ input, output, scheduleRequest: request => scheduleHostRequest(request, host.browser),
    handleRequest: (request, options) => host.handle(request, options) });
  const running = server.run();
  const rpc = (id, method, params) => {
    const result = Promise.withResolvers(); waiters.set(id, result);
    input.write(`${JSON.stringify({ id, method, params })}\n`);
    return result.promise;
  };
  const entered = Promise.withResolvers(), release = Promise.withResolvers();
  const handle = host.handle.bind(host);
  host.handle = async (request,options) => {
    if(request.method==='host.browser'&&request.params?.op==='display_subscribe'){
      entered.resolve();await release.promise;return {id:request.id,ok:true,result:{}};
    }
    return handle(request,options);
  };
  try {
    const state = rpc(1, "host.browser", { op: "display_subscribe", target:{kind:"browser_auxiliary"} });
    await entered.promise;
    const typed = await Promise.race([
      rpc(2, "host.browser", { op: "input", tab_id: opened.tab_id, generation: opened.generation,
        document_id: opened.tabs[0].document_id, input: { kind: "text", text: "a" } }),
      new Promise(resolve => setTimeout(() => resolve(null), 1000)),
    ]);
    assert.ok(typed?.ok, "input waited behind native subscription readiness");
    assert.equal(sent.filter(call => call.method === "Input.insertText").length, 1);
    release.resolve();
    const polled = await state;
    assert.equal(polled.ok, true);
  } finally { release.resolve(); input.end(); await running; lines.close(); output.end(); }
}));

// MP-08/MP-10/MP-11 review #1001 P1: no App, Vault or clipboard denial can stand in
// for extension authority on the native Computer path. Inspect live, including unregistered Tabs.
test('MP-11 native Computer mutations cannot operate owner-only extension surfaces', () => using(async ({ host, chromium, pages }) => {
  const opened = await host.request({ op: 'open', url: 'https://example.com/' });
  const tab = host.tabs.get(opened.tab_id);
  const binding = { surface_id: 'desktop', generation: 'native-1', width: 1280, height: 800, environment: {}, ownedProcesses: async () => [] };
  chromium.desktop = { binding: () => binding };
  let native = 0, accessible = 0, launched = 0;
  host.nativeComputer.channel = null; // Exercise owner admission through the same injected physical helper.
  host.nativeComputer.execute = async () => { native++; return { applied: true }; };
  host.nativeAccessibility.action = async () => { accessible++; return { applied: true }; };
  chromium.desktop.launchApplication = async () => { launched++; return {}; };
  const params = { surface_id: binding.surface_id, generation: binding.generation, _agent_input: true, observed_by: 'agent:a' };
  const send = (op, extra = {}) => host.handle({ id: 'native-extension', method: 'host.computer', params: { ...params, op, ...extra } });
  const click = { input: { kind: 'click', x: 600, y: 300 } };
  assert.equal((await send('input', click)).ok, true, 'ordinary native input still works');
  for (const url of ['https://chromewebstore.google.com/detail/x/abc', 'chrome://extensions/', 'chrome-extension://abc/popup.html']) {
    pages.set(tab.target_id, { url }); // Native navigation has not yet reconciled the host's cached URL.
    assert.equal(tab.url, 'https://example.com/');
    for (const [op, extra] of [['input', click], ['input', { input: { kind: 'key', key: 'Enter' } }], ['target_action', {}], ['launch_application', { application_id: 'chrome' }]]) {
      const before = [native, accessible, launched];
      const reply = await send(op, extra);
      assert.equal(reply.ok, false, `${url}: ${op} must refuse Add to Chrome/confirmation`);
      assert.match(reply.error.message, /extension_management_not_granted/);
      assert.deepEqual([native, accessible, launched], before, 'no effect before refusal');
    }
  }
  assert.equal((await send('input', { ...click, _agent_input: false, observed_by: 'terminal:owner' })).ok, true, 'the person still manages extensions');
  pages.set(tab.target_id, { url: 'https://example.com/' });
  pages.set('unreconciled-popup', { url: 'https://chrome.google.com/webstore/detail/x/abc' });
  assert.equal((await send('input', click)).ok, false, 'a newly opened extension surface is already fenced');
  pages.delete('unreconciled-popup');
  pages.set('extension-side-panel', { type: 'other', url: 'chrome-extension://abc/side-panel.html' });
  assert.equal((await send('input', click)).ok, false, 'non-page interactive extension surfaces are also owner-only');
  pages.delete('extension-side-panel');
  pages.set('tab-group-worker', { type: 'service_worker', url: 'chrome-extension://kernel/background.js' });
  assert.equal((await send('input', click)).ok, true, 'closing extension management restores ordinary input');
}));

test('MP-11 native agent mutations fail closed when browser destinations cannot be proved', () => using(async ({ host, chromium, connection }) => {
  await host.request({ op: 'open', url: 'https://example.com/' });
  const binding = { surface_id: 'desktop', generation: 'native-1', width: 1280, height: 800, environment: {}, ownedProcesses: async () => [] };
  chromium.desktop = { binding: () => binding };
  let calls = 0;
  host.nativeComputer.execute = async () => { calls++; return { applied: true }; };
  connection.beforeSend = async method => { if (method === 'Target.getTargets') throw new Error('private CDP detail'); };
  try {
    const reply = await host.handle({ id: 'native-unknown', method: 'host.computer', params: { op: 'input', surface_id: binding.surface_id, generation: binding.generation, _agent_input: true, observed_by: 'agent:a', input: { kind: 'click', x: 600, y: 300 } } });
    assert.equal(reply.ok, false);
    assert.match(reply.error.message, /extension_management_not_granted/);
    assert.equal(calls, 0);
  } finally { connection.beforeSend = null; }
}));

// r13 b3 union (1920x1080@1, browserux-r13union1-mvai225e): a whole-page native start fits Chrome's window
// to the page (1288x891) even when it then refuses (attestation mismatch); the band's capture retired on that
// resize and no live band was left to read the inventory, so the stale window binding refused every band
// resubscribe for 20 min. Every native start attempt re-resolves the window binding from the owner's inventory.
test('MP-08/MP-11 a native page start re-resolves the band window it may have resized', { timeout: 20000 }, (t) => using(async ({ host }) => {
  const { LinuxCapture } = await import('./kernel-browser-native.mjs');
  const { OwnedDisplay } = await import('./kernel-browser-owned-display.mjs');
  const real = new OwnedDisplay(os.tmpdir());
  try { await real.start(); } catch { return t.skip('no Xvfb'); }
  const start = LinuxCapture.prototype.start;
  try {
    const opened = await host.request({ op: 'open', url: 'https://example.com/' });
    const tab = host.tabs.get(opened.tab_id);
    host.chromium.display = real;
    let resolves = 0;
    host.chromeWindows.resolve = async () => { resolves++; };
    LinuxCapture.prototype.start = async function () { throw Error('MD-DISPLAY: native attestation RGB differed'); };
    const source = await host.compositorFor(tab, { codec: 'avc1.420033', device_scale_factor: 1 });
    assert.equal(source instanceof LinuxCapture, false, 'the refused native start falls back');
    assert.equal(resolves, 1, 'the window binding is re-resolved after the start');
  } finally { LinuxCapture.prototype.start = start; await host.closeCompositors(); await real.close(); }
}));


// MP-08/MP-10/MP-11:536's cropped page uses the ordinary admitted input path,
// and the crop's native wheel instead of an instant CDP scroll fallback.
test('MP-08/MP-10/MP-11 visible-page wheel reaches its admitted native source',()=>using(async({host,sent})=>{
 const {inputVisiblePage}=await import('./kernel-browser-page-display.mjs');
 const opened=await host.request({op:'open',url:'https://example.com/'}),tab=host.tabs.get(opened.tab_id),wheels=[];
 const source={attested:true,valid:()=>true,wheel:async(...args)=>{wheels.push(args);return true},wake(){}};
 const stream={close:async()=>{},subscription_id:'page-crop',regionTarget:{kind:'visible_page'},tab_id:tab.tab_id,generation:host.generation,observed_by:'terminal:a',expires:Date.now()+60000,
  regionLease:{document_id:tab.document_id},regionWindow:{isCurrent:()=>true},regionSource:source,regionCurrent:async()=>true,css_width:1280,css_height:712};
 host.displays.set(stream.subscription_id,stream);let fronts=0;host.nativeFront=async()=>{fronts++;return true};
 await inputVisiblePage(host,stream,{op:'input',tab_id:tab.tab_id,generation:host.generation,document_id:tab.document_id,_display_input:true,input:{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120}},'terminal:a');
 assert.deepEqual(wheels,[[10,20,0,1]]);assert(fronts>0,'fresh active Tab remains required');
 assert.equal(sent.filter(c=>c.method==='Input.dispatchMouseEvent'&&c.params.type==='mouseWheel').length,0);
 host.displays.delete(stream.subscription_id);
}));

// MP-08/MP-10/MP-11: the real host wiring reuses event-bound loader state.
test('MP-08/MP-10/MP-11 warm visible-page keys avoid duplicate loader queries in host and input helper',()=>using(async({host,sent,connection})=>{
 const {inputVisiblePage}=await import('./kernel-browser-page-display.mjs');
 const opened=await host.request({op:'open',url:'https://example.com/'}),tab=host.tabs.get(opened.tab_id),keys=[];
 const source={native:true,attested:true,valid:()=>true,key:(...args)=>{keys.push(args);return true},wake(){}};
 const stream={close:async()=>{},subscription_id:'key-crop',regionTarget:{kind:'visible_page'},tab_id:tab.tab_id,generation:host.generation,observed_by:'terminal:a',expires:Date.now()+60000,
  regionLease:{document_id:tab.document_id},regionWindow:{isCurrent:()=>true},regionSource:source,regionCurrent:async()=>true,css_width:1280,css_height:712};
 host.displays.set(stream.subscription_id,stream);host.nativeFront=async()=>true;
 const original=connection.send;connection.send=async(method,params,session)=>{
  if(method==='Runtime.evaluate'&&params.expression==='document.hasFocus()'){sent.push({method,params,session});return {result:{value:true}}}
  return original(method,params,session);
 };
 const command={op:'input',tab_id:tab.tab_id,generation:host.generation,document_id:tab.document_id,_display_input:true,input:{kind:'key',key:'a'}};
 try{
  await inputVisiblePage(host,stream,command,'terminal:a');sent.length=0;keys.length=0;
  await inputVisiblePage(host,stream,command,'terminal:a');
  assert.deepEqual(keys,[[97,false]]);assert.equal(sent.filter(call=>call.method==='Page.getFrameTree').length,0);
 }finally{host.displays.delete(stream.subscription_id);connection.send=original;}
}));

// vidlat r34: a warm native key admits from the event-bound owner snapshot; coordinate input still reads live.
function keyAdmission(host,tab,connection,{cachedActive}={}){
 const keys=[],reads={fresh:0,key:0,active:0};
 const source={native:true,attested:true,valid:()=>true,key:(...args)=>{keys.push(args);return true},click:()=>true,wheel:(...args)=>{keys.push(['wheel',...args]);return true},wake(){}};
 const stream={close:async()=>{},subscription_id:'key-admit',regionTarget:{kind:'visible_page'},tab_id:tab.tab_id,generation:host.generation,observed_by:'terminal:a',expires:Date.now()+60000,
  regionLease:{document_id:tab.document_id},regionWindow:{isCurrent:()=>true},regionSource:source,css_width:1280,css_height:712,
  regionCurrent:async()=>{reads.fresh++;return true},regionInputCurrent:async input=>{if(['key','scroll'].includes(input?.kind)){reads.key++;return true}reads.fresh++;return true}};
 host.displays.set(stream.subscription_id,stream);
 host.tabGroups={active:async()=>{reads.active++;return tab.target_id},cachedActive:()=>cachedActive===undefined?tab.target_id:cachedActive};
 const original=connection.send;connection.send=async(method,params,session)=>method==='Runtime.evaluate'&&params.expression==='document.hasFocus()'?{result:{value:true}}:original(method,params,session);
 return {keys,reads,stream,restore:()=>{host.displays.delete(stream.subscription_id);connection.send=original;}};
}
const keyCommand=(host,tab,kind='key')=>({op:'input',tab_id:tab.tab_id,generation:host.generation,document_id:tab.document_id,_display_input:true,input:kind==='key'?{kind:'key',key:'a'}:kind==='scroll'?{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120}:{kind:'click',x:10,y:20}});
test('MP-08/MP-10/MP-11 a warm native key makes no live active-Tab or geometry read',()=>using(async({host,connection})=>{
 const {inputVisiblePage}=await import('./kernel-browser-page-display.mjs');
 const opened=await host.request({op:'open',url:'https://example.com/'}),tab=host.tabs.get(opened.tab_id),f=keyAdmission(host,tab,connection);
 try{
  await inputVisiblePage(host,f.stream,keyCommand(host,tab),'terminal:a');
  assert.deepEqual(f.keys,[[97,false]]);assert.equal(f.reads.fresh,0,'no fresh geometry/layout read');assert.equal(f.reads.active,0,'no live active-Tab read');assert(f.reads.key>0);
 }finally{f.restore();}
}));
test('MP-08/MP-10/MP-11 a warm native wheel notch makes no live active-Tab, geometry or loader read',()=>using(async({host,connection,sent})=>{
 const {inputVisiblePage}=await import('./kernel-browser-page-display.mjs');
 const opened=await host.request({op:'open',url:'https://example.com/'}),tab=host.tabs.get(opened.tab_id),f=keyAdmission(host,tab,connection);
 try{
  await inputVisiblePage(host,f.stream,keyCommand(host,tab,'scroll'),'terminal:a');Object.assign(f.reads,{fresh:0,key:0,active:0});sent.length=0;f.keys.length=0;
  await inputVisiblePage(host,f.stream,keyCommand(host,tab,'scroll'),'terminal:a');
  assert.deepEqual(f.keys,[['wheel',10,20,0,1]]);assert.equal(f.reads.fresh,0,'no fresh geometry/layout read');assert.equal(f.reads.active,0,'no live active-Tab read');
  assert.equal(sent.filter(call=>call.method==='Page.getFrameTree').length,0,'no live loader read');assert(f.reads.key>0);
 }finally{f.restore();}
}));
test('MP-08/MP-10/MP-11 a native key still refuses when the snapshot says another Tab is in front or the admission fails',()=>using(async({host,connection})=>{
 const {inputVisiblePage}=await import('./kernel-browser-page-display.mjs');
 const opened=await host.request({op:'open',url:'https://example.com/'}),tab=host.tabs.get(opened.tab_id),f=keyAdmission(host,tab,connection,{cachedActive:'another-target'});
 try{
  await inputVisiblePage(host,f.stream,keyCommand(host,tab),'terminal:a').catch(()=>{});
  assert.deepEqual(f.keys,[],'no native key for a Tab that is not in front');assert.equal(f.reads.active,0);
  f.stream.regionInputCurrent=async()=>false;host.tabGroups.cachedActive=()=>tab.target_id;
  await assert.rejects(inputVisiblePage(host,f.stream,keyCommand(host,tab),'terminal:a'),{code:'user_domain_stale_reference'});
  assert.deepEqual(f.keys,[]);
 }finally{f.restore();}
}));
test('MP-08/MP-10/MP-11 a native wheel notch is not native when the snapshot says another Tab is in front or its admission fails; clicks still read live',()=>using(async({host,connection})=>{
 const {inputVisiblePage}=await import('./kernel-browser-page-display.mjs');
 const opened=await host.request({op:'open',url:'https://example.com/'}),tab=host.tabs.get(opened.tab_id),f=keyAdmission(host,tab,connection,{cachedActive:'another-target'});
 try{
  await inputVisiblePage(host,f.stream,keyCommand(host,tab,'scroll'),'terminal:a').catch(()=>{});
  assert.deepEqual(f.keys,[],'no native wheel for a Tab that is not in front');assert.equal(f.reads.active,0);
  f.stream.regionInputCurrent=async()=>false;host.tabGroups.cachedActive=()=>tab.target_id;
  await assert.rejects(inputVisiblePage(host,f.stream,keyCommand(host,tab,'scroll'),'terminal:a'),{code:'user_domain_stale_reference'});
  assert.deepEqual(f.keys,[]);
 }finally{f.restore();}
}));
test('MP-08/MP-10/MP-11 a native key reads the active Tab live on a snapshot miss; coordinate input always does',()=>using(async({host,connection})=>{
 const {inputVisiblePage}=await import('./kernel-browser-page-display.mjs');
 const opened=await host.request({op:'open',url:'https://example.com/'}),tab=host.tabs.get(opened.tab_id),f=keyAdmission(host,tab,connection,{cachedActive:null});
 try{
  await inputVisiblePage(host,f.stream,keyCommand(host,tab),'terminal:a');
  assert.equal(f.reads.active,1,'a miss reads live');assert.equal(f.keys.length,1);
  f.reads.active=0;host.tabGroups.cachedActive=()=>tab.target_id;
  await inputVisiblePage(host,f.stream,keyCommand(host,tab,'click'),'terminal:a');
  assert(f.reads.active>=1,'a click ignores the snapshot');assert(f.reads.fresh>0,'a click reads geometry live');
 }finally{f.restore();}
}));

// MP-08/MP-10/MP-11: failed extension startup and browser recovery release observers.
test('MP-08/MP-10/MP-11 native owner observers retire on recovery and shutdown',()=>using(async({host,chromium,handlers,connection})=>{
 const {ChromeTabGroups}=await import('./kernel-browser-tab-groups.mjs');
 await host.request({op:'open',url:'about:blank'});assert.equal(host.tabGroups,null);assert.equal(handlers.size,2,'failed extension load leaves lockdown and recovery');
 const groups=new ChromeTabGroups(connection);await groups.nativeState.attach('worker-session');host.tabGroups=groups;assert.equal(handlers.size,3);
 chromium.child.exitCode=1;await host.request({op:'start'});
 assert.equal(groups.nativeState.closed,true);assert.equal(groups.nativeState.off,null);assert.equal(host.tabGroups,null);
 assert.equal(handlers.size,2,'lockdown and recovery remain; the retired native observer is gone');
 await host.request({op:'stop'});assert.equal(handlers.size,0);
}));

// PERF_AUDIT 4.2.3 (protocol 540): a viewer that holds frames pushed before its registration response
// asks for its first frame at registration (the relay pump then starts without waiting for its first ACK).
test('MP-08/MP-10 a display attach reports the viewer\'s initial push request',()=>using(async({host})=>{
 const original=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try{const tab=await host.request({op:'open',url:'about:blank'});
  const subscribe=extra=>host.request({op:'display_subscribe',tab_id:tab.tab_id,generation:tab.generation,codecs:['png'],bitrate:8000000,device_scale_factor:1,...extra});
  const pushed=await subscribe({initial_push:true}),plain=await subscribe({});
  assert.deepEqual(await host.request({op:'display_attach',subscription_id:pushed.subscription_id,generation:tab.generation}),{attached:true,generation:tab.generation,initial_push:true});
  assert.deepEqual(await host.request({op:'display_attach',subscription_id:plain.subscription_id,generation:tab.generation}),{attached:true,generation:tab.generation});
 }finally{if(original===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=original;}
}));

// PERF_AUDIT 4.2.2/4.2.6: the window a Chrome UI input left is read concurrently with later input
// (never in the host's serial barrier lane, where it queued every following key) and never starts the browser.
test('MP-08/MP-10 the Chrome window follow-up is a concurrent observation',()=>using(async({host})=>{
 assert.equal(scheduleHostRequest({method:'host.browser',params:{op:'chrome_window_follow',tab_id:'t',switches:false}}).kind,'bridge');
 assert.equal(scheduleHostRequest({method:'host.browser',params:{op:'input'}}).kind,'barrier');
 // r54 RED 2: a selection note waits for the page renderer (8 s on a parsing page); it must not hold every key behind it.
 for(const op of ['note_selection'])assert.equal(scheduleHostRequest({method:'host.browser',params:{op,tab_id:'t'}}).kind,'bridge');
 await assert.rejects(host.request({op:'chrome_window_follow',tab_id:'t',switches:false}),/MP-11: user browser is stopped/);
}));

// MP-08/MP-10 (BROWSER-UX-05): a Tab that just opened has a busy renderer; its own-Tab read (the subscribe
// preflight) is answered from the page session's navigation events, and asks the renderer only when none named it.
test("MP-08/MP-10 a Tab's own read uses the document the page session named and asks the renderer otherwise", async () => {
  const calls = [], saved = [];
  const connection = { send: async method => { calls.push(method);
    if (method === "Page.getFrameTree") return { frameTree: { frame: { loaderId: "LIVE" } } };
    if (method === "Target.getTargetInfo") return { targetInfo: { url: "https://home.example/", title: "Home" } };
    throw new Error("unexpected " + method); } };
  const read = cached => {
    const stored = { tab_id: "t", target_id: "T", document_id: "NTP", url: "chrome://newtab/", title: "" };
    const host = { generation: 1, tabs: new Map([["t", stored]]), save: async () => { saved.push(stored.document_id); },
      browser: { documentIdsByTarget: new Map(cached ? [["T", cached]] : []), resolvePageTarget: async () => ({ connection, sessionId: "S" }) } };
    return KernelBrowserHost.prototype.displayTarget.call(host, { tab_id: "t", generation: 1 });
  };
  calls.length = 0;
  const named = await read("HOME");
  assert.deepEqual([named.document_id, named.url, named.title, calls], ["HOME", "https://home.example/", "Home", ["Target.getTargetInfo"]]);
  calls.length = 0;
  const live = await read(undefined);
  assert.deepEqual([live.document_id, calls], ["LIVE", ["Page.getFrameTree", "Target.getTargetInfo"]]);
  assert.deepEqual(saved, ["HOME", "LIVE"]);
});

test('MP-08/MP-10/MP-11 viewer input waits in order while the busy renderer cannot answer the fences',()=>using(async({host,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=5;
 let busy=true;const before=connection.beforeSend;
 connection.beforeSend=async(method)=>{if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})};
 const text=input=>({op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input});
 const first=await host.request(text({kind:'text',text:'a'}));
 const second=await host.request(text({kind:'text',text:'b'}));
 assert.ok(first&&second,'a busy renderer is not an error for the person');
 assert.equal(sent.filter(x=>x.method==='Input.insertText').length,0,'nothing reaches the page before the fences pass');
 busy=false;
 for(let n=0;n<200&&sent.filter(x=>x.method==='Input.insertText').length<2;n++)await new Promise(r=>setTimeout(r,5));
 assert.deepEqual(sent.filter(x=>x.method==='Input.insertText').map(x=>x.params.text),['a','b']);
 connection.beforeSend=before;
}));

test('MP-08/MP-10/MP-11 viewer input that may have been dispatched is never parked or replayed',()=>using(async({host,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 let attempts=0;connection.beforeSend=async(method)=>{if(method==='Input.insertText'){attempts++;throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})}};
 await assert.rejects(host.request({op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text:'a'}}),error=>error.code==='browser_cdp_timeout');
 await new Promise(r=>setTimeout(r,50));
 assert.equal(attempts,1);
}));

test('MP-08/MP-10/MP-11 a parked wheel burst replays as one scroll',()=>using(async({host,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=5;let busy=true;
 connection.beforeSend=async(method)=>{if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})};
 for(let n=0;n<5;n++)await host.request({op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120}});
 busy=false;
 for(let n=0;n<200&&!sent.some(x=>x.method==='Input.dispatchMouseEvent');n++)await new Promise(r=>setTimeout(r,5));
 await new Promise(r=>setTimeout(r,30));
 const wheels=sent.filter(x=>x.method==='Input.dispatchMouseEvent'&&x.params.type==='mouseWheel');
 assert.equal(wheels.length,1);assert.equal(wheels[0].params.deltaY,600);
}));

test('MP-08/MP-10/MP-11 typing that lost a parked key is dropped whole, never delivered mangled',()=>using(async({host,connection,sent,pages})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0],target=host.tabs.get(opened.tab_id).target_id;
 host.busyInputs.retryMs=5;let busy=true,refuseFirst=true;
 connection.beforeSend=async(method,params)=>{
  if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'});
  if(!busy&&refuseFirst&&method==='Page.getFrameTree'){refuseFirst=false;pages.set(target,{url:'about:blank',document_id:'replacement'})}
 };
 for(const text of ['h','t','t','p'])await host.request({op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text}});
 busy=false;
 for(let n=0;n<100&&!host.busyInputs.queues.size&&n<5;n++)await new Promise(r=>setTimeout(r,5));
 for(let n=0;n<200&&host.busyInputs.queues.size;n++)await new Promise(r=>setTimeout(r,5));
 assert.equal(sent.filter(x=>x.method==='Input.insertText').length,0,'no letter of a word that lost its first letter is typed');
}));

// r55: the page acknowledged the input, so a busy renderer that cannot answer the observation after it must not turn the
// person's accepted input into an error (r54/r55: "input refused" notices); the Tab state is the last observed one.
test('MP-08/MP-10/MP-11 input acknowledged by the page is accepted and never replayed when only the post-input observation times out',()=>using(async({host,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=5;
 const reconcile=host.browser.reconcile.bind(host.browser);
 host.browser.reconcile=async(...args)=>{if(sent.some(x=>x.method==='Input.insertText'))throw Object.assign(Error('browser CDP method Target.getTargets timed out after 5000ms'),{code:'browser_cdp_timeout'});return reconcile(...args)};
 const reply=await host.request({op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text:'a'}});
 assert.ok(Array.isArray(reply.tabs),'the reply carries the last observed Tab state');
 await new Promise(r=>setTimeout(r,50));
 assert.equal(sent.filter(x=>x.method==='Input.insertText').length,1);
 assert.equal(host.busyInputs.queues.size,0);
}));

test('MP-08/MP-10/MP-11 a scroll arriving while a parked scroll is being admitted is not lost',()=>using(async({host,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=5;let busy=true,release,gate=null;
 connection.beforeSend=async(method)=>{
  if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'});
  if(gate&&method==='Page.getFrameTree')await gate;
 };
 const scroll=()=>host.request({op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'scroll',x:10,y:20,delta_x:0,delta_y:120}});
 await scroll();
 gate=new Promise(r=>{release=r});busy=false;
 for(let n=0;n<200&&!host.busyInputs.queues.get('adapter')?.active;n++)await new Promise(r=>setTimeout(r,5));
 await scroll();
 release();
 for(let n=0;n<200&&host.busyInputs.queues.size;n++)await new Promise(r=>setTimeout(r,5));
 const wheels=sent.filter(x=>x.method==='Input.dispatchMouseEvent'&&x.params.type==='mouseWheel');
 assert.equal(wheels.reduce((sum,x)=>sum+x.params.deltaY,0),240);
}));

test('MP-08/MP-10/MP-11 parked input dies with its terminal and with a takeover of its tab',()=>using(async({host,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=5;let busy=true;
 connection.beforeSend=async(method)=>{if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})};
 const type=(observed_by,text)=>host.request({op:'input',observed_by,tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text}});
 await type('terminal:a','a');await type('terminal:b','b');await type('terminal:c','c');
 assert.equal(host.busyInputs.queues.size,3);
 await host.handle({id:1,method:'host.computer.retire',params:{observer:'terminal:a'}});
 await host.handle({id:2,method:'host.browser.retire_input',params:{tab_id:tab.tab_id,except:'terminal:c'}});
 assert.deepEqual([...host.busyInputs.queues.values()].filter(queue=>queue.entries.length).map(queue=>queue.key),['terminal:c']);
 busy=false;
 for(let n=0;n<200&&host.busyInputs.queues.size;n++)await new Promise(r=>setTimeout(r,5));
 assert.deepEqual(sent.filter(x=>x.method==='Input.insertText').map(x=>x.params.text),['c']);
}));

test('MP-08/MP-10/MP-11 stopping the browser ends a parked input and a later retry cannot relaunch it',()=>using(async({host,chromium,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=30;let busy=true;
 connection.beforeSend=async(method)=>{if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})};
 await host.request({op:'input',observed_by:'terminal:a',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text:'a'}});
 assert.equal(host.busyInputs.queues.size,1);
 let starts=0;const start=chromium.start;chromium.start=async(...args)=>{starts++;return start(...args)};
 await host.request({op:'stop'});busy=false;
 await new Promise(r=>setTimeout(r,120));
 assert.equal(starts,0,'a stopped browser is not relaunched by a parked input');
 assert.equal(chromium.child,null);assert.equal(host.busyInputs.queues.size,0);
 assert.equal(sent.filter(x=>x.method==='Input.insertText').length,0);
}));

test('MP-08/MP-10/MP-11 the parking reply and the observer\'s state report its parked input (protocol 552), nobody else\'s',()=>using(async({host,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=5;let busy=true;
 connection.beforeSend=async(method)=>{if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})};
 const type=(observed_by,text)=>host.request({op:'input',observed_by,tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text}});
 assert.equal((await type('terminal:a','a')).parked_input,1);
 assert.equal((await type('terminal:a','b')).parked_input,2);
 assert.equal((await host.request({op:'state',observed_by:'terminal:a'})).parked_input,2,'a state read answers without waiting for the busy renderer');
 assert.equal((await host.request({op:'state',observed_by:'terminal:b'})).parked_input,undefined);
 busy=false;
 for(let n=0;n<200&&host.busyInputs.queues.size;n++)await new Promise(r=>setTimeout(r,5));
 assert.equal((await host.request({op:'state',observed_by:'terminal:a'})).parked_input,undefined,'cleared once the input is admitted');
 assert.deepEqual(sent.filter(x=>x.method==='Input.insertText').map(x=>x.params.text),['a','b']);
}));

test('MP-08/MP-10/MP-11 a retirement from an older ownership change cannot retire the newer owner\'s parked input',()=>using(async({host,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=5;let busy=true;
 connection.beforeSend=async(method)=>{if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})};
 const type=(observed_by,text)=>host.request({op:'input',observed_by,tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text}});
 await type('terminal:b','b');await type('terminal:c','c');
 await host.handle({id:1,method:'host.browser.retire_input',params:{tab_id:tab.tab_id,except:'terminal:c',epoch:2}});
 await host.handle({id:2,method:'host.browser.retire_input',params:{tab_id:tab.tab_id,except:'terminal:b',epoch:1}});
 busy=false;
 for(let n=0;n<200&&host.busyInputs.queues.size;n++)await new Promise(r=>setTimeout(r,5));
 assert.deepEqual(sent.filter(x=>x.method==='Input.insertText').map(x=>x.params.text),['c']);
}));

test('MP-08/MP-10/MP-11 a retirement for one tab is not suppressed by a newer ownership change of another tab',()=>using(async({host,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 const other=await host.request({op:'open',url:'about:blank'});
 host.busyInputs.retryMs=5;let busy=true;
 connection.beforeSend=async(method)=>{if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})};
 await host.request({op:'input',observed_by:'terminal:a',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text:'a'}});
 await host.handle({id:1,method:'host.browser.retire_input',params:{tab_id:other.tab_id,except:'terminal:c',epoch:2}});
 await host.handle({id:2,method:'host.browser.retire_input',params:{tab_id:tab.tab_id,except:'terminal:b',epoch:1}});
 busy=false;
 for(let n=0;n<200&&host.busyInputs.queues.size;n++)await new Promise(r=>setTimeout(r,5));
 assert.deepEqual(sent.filter(x=>x.method==='Input.insertText'),[],'the older takeover of tab X still retires the previous owner\'s input');
}));

test('MP-08/MP-10/MP-11 a parked input refused as stale on an unchanged document is admitted again; a changed document drops it',()=>using(async({host,connection,sent})=>{
 const {UserDomainRefusal}=await import('./kernel-browser-refusal.mjs');
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=5;let busy=true,refusals=2;
 connection.beforeSend=async(method)=>{if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})};
 const inputGuard=async()=>{if(refusals-->0)throw new UserDomainRefusal('stale_reference')};
 const type=text=>host.request({op:'input',observed_by:'terminal:a',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text}},{inputGuard});
 await type('a');busy=false;
 for(let n=0;n<200&&host.busyInputs.queues.size;n++)await new Promise(r=>setTimeout(r,5));
 assert.deepEqual(sent.filter(x=>x.method==='Input.insertText').map(x=>x.params.text),['a'],'the geometry moved under the busy renderer, the document did not');
 busy=true;refusals=1000;await type('b');host.tabs.get(tab.tab_id).document_id='another';busy=false;
 for(let n=0;n<200&&host.busyInputs.queues.size;n++)await new Promise(r=>setTimeout(r,5));
 assert.deepEqual(sent.filter(x=>x.method==='Input.insertText').map(x=>x.params.text),['a'],'a changed document drops the input');
}));

test('MP-08/MP-10/MP-11 a disconnected terminal\'s observer retirement drops its parked input and keeps the others\'',()=>using(async({host,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=5;let busy=true;
 connection.beforeSend=async(method)=>{if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})};
 const type=(observed_by,text)=>host.request({op:'input',observed_by,tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text}});
 await type('terminal:a','a');await type('terminal:b','b');
 await host.handle({id:1,method:'host.browser.retire_observer',params:{observer:'terminal:a'}});
 busy=false;
 for(let n=0;n<200&&host.busyInputs.queues.size;n++)await new Promise(r=>setTimeout(r,5));
 assert.deepEqual(sent.filter(x=>x.method==='Input.insertText').map(x=>x.params.text),['b']);
}));

test('MP-08/MP-10/MP-11 a parked retry retired inside its final input guard dispatches nothing',()=>using(async({host,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=5;let busy=true,reached,release;
 const inGuard=new Promise(r=>{reached=r}),gate=new Promise(r=>{release=r});
 connection.beforeSend=async(method)=>{if(busy&&method==='Page.getFrameTree')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})};
 const inputGuard=async()=>{reached();await gate};
 await host.request({op:'input',observed_by:'terminal:a',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text:'a'}},{inputGuard});
 busy=false;await inGuard;
 await host.handle({id:1,method:'host.browser.retire_input',params:{tab_id:tab.tab_id,except:'terminal:b'}});
 release();
 for(let n=0;n<200&&host.busyInputs.queues.size;n++)await new Promise(r=>setTimeout(r,5));
 assert.deepEqual(sent.filter(x=>x.method==='Input.insertText'),[]);
}));

test('MP-08/MP-10/MP-11 viewer input parks on the first fence read that times out, not after the last',()=>using(async({host,connection})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.busyInputs.retryMs=60_000;let timedOut=0;
 connection.beforeSend=async(method)=>{if(!method.startsWith('Input.')){timedOut++;throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})}};
 await host.request({op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text:'a'}});
 assert.equal(timedOut,1,'every further read would cost another 5 s of the 30 s the viewer waits');
 assert.equal(host.busyInputs.queues.size,1);
}));

test('MP-08/MP-10/MP-11 a busy renderer that is late to acknowledge a click never stops the browser',()=>using(async({host,connection,sent})=>{
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 let stops=0;const stop=host.stop.bind(host);host.stop=async()=>{stops++;return stop()};
 connection.beforeSend=async(method,params)=>{if(method==='Input.dispatchMouseEvent'&&params.type==='mousePressed')throw Object.assign(Error(`browser CDP method ${method} timed out after 5000ms`),{code:'browser_cdp_timeout'})};
 await assert.rejects(host.request({op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'click',x:10,y:10}}),error=>error.code==='browser_cdp_timeout'&&error.afterDispatch===true);
 assert.equal(stops,0,'the press and its release were both sent in order: nothing is stuck, and the page is only busy');
 assert.ok(sent.some(x=>x.method==='Input.dispatchMouseEvent'&&x.params.type==='mouseReleased'),'the release was queued with the press');
 connection.beforeSend=null;
 assert.ok((await host.request({op:'state'})).tabs.length,'the browser is still there');
}));

// r55: Chrome's band input has its own busy lane; page input and unbound input share the page lane.
test('MP-08/MP-10/MP-11 inputLane separates Chrome band input from page input', () => using(async ({ host }) => {
  host.displays.set('band', { binding_id: 'b1', regionTarget: { kind: 'browser_chrome' } });
  host.displays.set('popup', { binding_id: 'b2', regionTarget: { kind: 'browser_auxiliary' } });
  host.displays.set('page', { binding_id: 'b3', regionTarget: { kind: 'visible_page' } });
  assert.deepEqual(['b1', 'b2', 'b3', 'unknown', undefined].map(binding_id => host.inputLane({ op: 'input', binding_id })), ['band', 'band', 'page', 'page', 'page']);
  assert.equal(host.inputLane({ op: 'input', binding_id: 'b3', input: { kind: 'scroll' } }), 'wheel', 'r67: a parked wheel does not hold the keys behind it');
  assert.equal(host.inputLane({ op: 'input', binding_id: 'b3', input: { kind: 'key' } }), 'page');
  assert.equal(host.inputLane({ op: 'input', binding_id: 'b1', input: { kind: 'scroll' } }), 'band');
  host.displays.clear();
}));

// r55: a person's first wheel on a busy page waited 5 s for the popup-evidence observer's renderer round trips (a wheel
// opens no window) and held the input lane; now it installs no observer.
test('MP-08/MP-10/MP-11 a scroll installs no popup-evidence observer in the page, a click does', () => using(async ({ host, sent }) => {
  const opened = await host.request({ op: 'open', url: 'about:blank' }), tab = opened.tabs[0];
  const input = (kind, extra) => host.request({ op: 'input', tab_id: tab.tab_id, generation: opened.generation, document_id: tab.document_id, _action_id: `action-${kind}`, input: { kind, x: 10, y: 20, ...extra } });
  sent.length = 0; await input('scroll', { delta_x: 0, delta_y: 120 });
  assert.equal(sent.some(x => x.method === 'Page.addScriptToEvaluateOnNewDocument'), false, 'no observer for a wheel');
  sent.length = 0; await input('click');
  assert.equal(sent.some(x => x.method === 'Page.addScriptToEvaluateOnNewDocument'), true, 'a click keeps its popup evidence');
}));

// r55: during a navigation preview a person's wheel goes by CDP (the handover flush must follow CDP acks). A busy
// renderer never acks it: awaiting the ack held the input lane 5-9 s and ended in an error. The dispatch order is
// CDP's own, so the wheel is sent without awaiting the ack, as for the native path.
test('MP-08/MP-10/MP-11 a person\'s wheel during a navigation preview does not wait for the renderer\'s ack', () => using(async ({ host, connection }) => {
  const opened = await host.request({ op: 'open', url: 'about:blank' }), tab = opened.tabs[0];
  host.displays.set('viewer', { tab_id: tab.tab_id, observed_by: 'terminal:a', expires: Date.now() + 60_000, close: async () => {} });
  host.mirror.streams.set('m', { tab_id: tab.tab_id, scope: 'terminal:a', navigationPreview: true });
  let released; const neverAcked = new Promise(resolve => { released = resolve; });
  connection.beforeSend = async method => { if (method === 'Input.dispatchMouseEvent') await neverAcked; };
  const started = performance.now();
  try { await Promise.race([
    host.request({ op: 'input', observed_by: 'terminal:a', _display_input: true, tab_id: tab.tab_id, generation: opened.generation, document_id: tab.document_id, input: { kind: 'scroll', x: 10, y: 20, delta_x: 0, delta_y: 120 } }),
    new Promise((_, reject) => setTimeout(() => reject(new Error('the wheel waited for the renderer ack')), 1500)),
  ]); assert.ok(performance.now() - started < 1500); }
  finally { released(); host.displays.clear(); host.mirror.streams.clear(); }
}));

// r55: the observation after a page-acknowledged input does not hold the person's reply (and with it the whole input
// lane) while a busy renderer takes its 5 s CDP timeouts; the reply carries the last observed Tab state.
test('MP-08/MP-10/MP-11 the post-input observation is bounded, the input reply does not wait for a busy page', () => using(async ({ host, sent }) => {
  const opened = await host.request({ op: 'open', url: 'about:blank' }), tab = opened.tabs[0];
  const reconcile = host.browser.reconcile.bind(host.browser);
  host.browser.reconcile = (...args) => sent.some(x => x.method === 'Input.insertText') ? new Promise(() => {}) : reconcile(...args);
  const started = performance.now();
  const reply = await host.request({ op: 'input', tab_id: tab.tab_id, generation: opened.generation, document_id: tab.document_id, input: { kind: 'text', text: 'a' } });
  assert.ok(performance.now() - started < 1500, `the reply waited for the observation: ${Math.round(performance.now() - started)} ms`);
  assert.ok(Array.isArray(reply.tabs));
}));

// Protocol 556: the viewer's 250 ms state poll becomes one parked wait per change plus one read.
const parkSignal=(host,after,wait_ms)=>host.handle({id:'p',method:'host.browser',params:{op:'state_signal',after,wait_ms}}).then(reply=>reply.result.signal);
test('MP-08/MP-10 a state read reports the signal taken before it and a parked wait ends on a Tab change',()=>using(async({host,handlers})=>{
 await host.request({op:'open',url:'https://example.com/a'});
 const first=await host.request({op:'state',state_wait:true});
 assert.equal(typeof first._state_signal,'number');
 assert.equal('_state_signal' in await host.request({op:'state'}),false,'a plain state read is unchanged');
 let answered;const parked=parkSignal(host,first._state_signal,5000).then(signal=>answered=signal);
 await new Promise(r=>setTimeout(r,60));assert.equal(answered,undefined,'parked');
 for(const method of ['Network.requestWillBeSent','Page.screencastFrame','DOM.childNodeInserted'])for(const handler of handlers)handler({method,params:{}});
 await new Promise(r=>setTimeout(r,30));assert.equal(answered,undefined,'unrelated events leave it parked');
 for(const handler of handlers)handler({method:'Target.targetInfoChanged',params:{targetInfo:{targetId:'target-1',type:'page'}}});
 assert.equal(await parked,first._state_signal+1);
 assert.equal(await parkSignal(host,first._state_signal,5000),first._state_signal+1,'a change before the wait arrived is not lost');
}));
test('MP-08/MP-10 a parked wait is answered at its deadline and by Tab or agent requests, not by a person\'s input',()=>using(async({host})=>{
 const opened=await host.request({op:'open',url:'https://example.com/a'});
 const signal=(await host.request({op:'state',state_wait:true}))._state_signal;
 const started=performance.now();assert.equal(await parkSignal(host,signal,80),signal);assert(performance.now()-started>=70,'an idle wait lasts its deadline');
 assert.equal(await parkSignal(host,null,5000),signal,'no baseline: answers at once');
 const tab=opened.tabs[0];
 const input=(extra={})=>host.handle({id:'i',method:'host.browser',params:{op:'input',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'text',text:'x'},...extra}});
 let woke=false;const parked=parkSignal(host,signal,400).then(()=>woke=true);
 await input();await new Promise(r=>setTimeout(r,60));assert.equal(woke,false,'a person\'s input leaves it parked');
 await input({_agent_input:true,observed_by:'agent:a'}).catch(()=>{});await parked;assert.equal(woke,true,'an agent acting wakes it');
 const second=(await host.request({op:'state',state_wait:true}))._state_signal;
 const waiting=parkSignal(host,second,5000);
 await host.handle({id:'n',method:'host.browser',params:{op:'navigate',tab_id:tab.tab_id,generation:opened.generation,document_id:tab.document_id,url:'https://example.com/b'}});
 assert((await waiting)>second,'a navigation wakes it');
}));
test('MP-08/MP-10 a parked wait starts no browser and never queues behind the host barrier lane',()=>using(async({host,chromium})=>{
 assert.equal(chromium.child,null);
 assert.equal(await parkSignal(host,0,20),0);assert.equal(chromium.child,null,'waiting never launches Chromium');
 const parked={method:'host.browser',params:{op:'state_signal',after:3,wait_ms:2000}};
 for(const reconcilable of [true,false,undefined])assert.deepEqual(scheduleHostRequest(parked,{canReconcileConcurrently:()=>reconcilable}),{kind:'bridge'});
 assert.deepEqual(scheduleHostRequest({method:'host.browser',params:{op:'state'}},{canReconcileConcurrently:()=>false}),{kind:'barrier'},'a plain state read keeps its viewport barrier');
}));


test('MP-08/MP-10 browser death automatically restores stable tabs, active tab and groups', {timeout: 10000}, () => using(async ({host,chromium,handlers,sent,connection}) => {
  const a=await host.request({op:'open',url:'https://www.wikipedia.org/'});
  const b=await host.request({op:'open',url:'https://github.com/charioxai/chariox'});
  host.activeTarget=host.tabs.get(a.tab_id).target_id;
  await host.syncTabGroups([{tab_id:b.tab_id,label:'Mara'}]);
  await host.save();
  const generation=host.generation;
  chromium.child.signalCode='SIGSEGV';
  for(const handler of [...handlers])handler({method:'Chariox.browserDisconnected',params:{code:'browser_cdp_disconnected'}});
  for(let n=0;n<150&&host.generation===generation;n++)await new Promise(r=>setTimeout(r,10));
  await host.starting;
  assert.equal(host.generation,generation+1,'the loss signal restarts without a user request');
  assert.deepEqual([...host.tabs.keys()],[a.tab_id,b.tab_id]);
  assert.equal(host.activeTarget,host.tabs.get(a.tab_id).target_id);
  assert.deepEqual(host.savedGroups,[{tab_id:b.tab_id,label:'Mara'}]);
  assert(sent.some(call=>call.method==='Target.activateTarget'&&call.params.targetId===host.activeTarget));
  const saved=JSON.parse(await readFile(path.join(host.root,'tabs.json'),'utf8'));
  assert.equal(saved.active_tab_id,a.tab_id);
}));

test('MP-08/MP-10 a retired display is a navigation race; a foreign display remains denied', () => using(async ({host}) => {
  const opened=await host.request({op:'open',url:'about:blank'});
  await assert.rejects(host.request({op:'screenshot',display_subscription_id:'retired',generation:opened.generation}), error=>error.code==='user_domain_stale_reference'&&error.race==='display_retired');
  host.displays.set('foreign',{observed_by:'terminal:other',expires:Date.now()+60000,close:async()=>{}});
  await assert.rejects(host.request({op:'screenshot',display_subscription_id:'foreign',generation:opened.generation}),{code:'user_domain_not_granted'});
}));

test('MP-08/MP-10 interrupted old-browser actions settle without replay or stopping the recovered browser', {timeout:10000}, () => using(async ({host,chromium,handlers}) => {
  const opened=await host.request({op:'open',url:'https://www.wikipedia.org/'});
  const began=Promise.withResolvers(),ended=Promise.withResolvers();
  host.browser.navigate=async()=>{began.resolve();await ended.promise;throw Object.assign(Error('cancelled'),{code:'browser_action_cancelled'});};
  const action=host.handle({id:'interrupted',method:'host.browser',params:{op:'navigate',tab_id:opened.tab_id,generation:opened.generation,url:'https://github.com/charioxai/chariox'}});
  await began.promise;
  chromium.child.signalCode='SIGSEGV';
  for(const handler of [...handlers])handler({method:'Chariox.browserDisconnected',params:{code:'browser_cdp_disconnected'}});
  for(let n=0;n<150&&host.generation===opened.generation;n++)await new Promise(r=>setTimeout(r,10));
  await host.starting;ended.resolve();
  const reply=await action;
  assert.equal(reply.ok,false);assert.equal(reply.error.code,'browser_unavailable');assert.match(reply.error.message,/disconnected.*not replayed/);
  assert.equal(host.generation,opened.generation+1);assert(host.browser);assert.equal(chromium.child.signalCode,null);
  assert.equal(host.tabs.get(opened.tab_id).url,'https://www.wikipedia.org/');
}));

test('MP-08/MP-10 concurrent recovery reads launch one browser and explicit stop stays stopped', () => using(async ({host,chromium}) => {
  let starts=0;const start=chromium.start;chromium.start=async()=>{starts++;return start();};
  const opened=await host.request({op:'open',url:'about:blank'});chromium.child.exitCode=139;
  const states=await Promise.all(Array.from({length:8},()=>host.request({op:'state'})));
  assert.equal(starts,2);assert(states.every(state=>state.generation===opened.generation+1));
  assert(states.every(state=>state.tabs.length===1&&state.tabs[0].tab_id===opened.tab_id));
  await host.stop();await new Promise(r=>setTimeout(r,300));assert.equal(starts,2);assert.equal(host.recovery.enabled,false);
  await assert.rejects(host.request({op:'state'}),{code:'browser_unavailable'});
}));

test('MP-08/MP-10 a cancelled viewer does not cancel shared browser recovery', () => using(async ({host,chromium}) => {
  const opened=await host.request({op:'open',url:'about:blank'});chromium.child.exitCode=139;
  const start=chromium.start,began=Promise.withResolvers(),gate=Promise.withResolvers();
  chromium.start=async()=>{began.resolve();await gate.promise;return start();};
  const controller=new AbortController();
  const cancelled=host.request({op:'state'},{signal:controller.signal});
  const rejected=assert.rejects(cancelled,{code:'browser_action_cancelled'});
  await began.promise;const other=host.request({op:'state'});controller.abort();gate.resolve();
  await rejected;const state=await other;
  assert.equal(state.generation,opened.generation+1);assert.equal(state.tabs[0].tab_id,opened.tab_id);
  assert(host.browser);assert.equal(host.recovery.enabled,true);
}));

test('MP-08/MP-10 recovery restores every tab before one inventory read', () => using(async ({host},root) => {
  const tabs=Array.from({length:8},(_,i)=>({tab_id:`host-tab-restored-${i}`,url:`https://en.wikipedia.org/wiki/Web_browser#${i}`}));
  await writeFile(path.join(root,'tabs.json'),JSON.stringify({generation:7,tabs,active_tab_id:tabs[2].tab_id}));
  let reads=0;const factory=host.browserFactory;
  host.browserFactory=connection=>{const browser=factory(connection),read=browser.reconcile;browser.reconcile=async(...args)=>{reads++;return read(...args)};return browser;};
  await host.start();
  assert.equal(reads,1,'restoring N tabs must not wait for N all-page renderer inventories');
  assert.deepEqual([...host.tabs.keys()],tabs.map(tab=>tab.tab_id));
  assert.equal(host.activeTarget,host.tabs.get(tabs[2].tab_id).target_id);
}));

for(const late of [false,true])test(`MP-08/MP-10 a disconnected page input never cancels browser recovery (late=${late})`,{timeout:10000},()=>using(async({host,chromium,connection,handlers})=>{
 let live=true;chromium.connection=connection;connection.isOpen=()=>live;
 const start=chromium.start;chromium.start=async()=>{live=true;return start();};
 const opened=await host.request({op:'open',url:'https://www.wikipedia.org/'}),tab=opened.tabs[0];
 const began=Promise.withResolvers(),gate=Promise.withResolvers();let presses=0;
 connection.beforeSend=async method=>{if(method==='Input.dispatchMouseEvent'){presses++;began.resolve();await gate.promise;throw Object.assign(Error('pipe closed'),{code:'browser_cdp_disconnected'});}};
 const action=host.handle({id:'page-input',method:'host.browser',params:{op:'input',_agent_input:true,tab_id:opened.tab_id,generation:opened.generation,document_id:tab.document_id,input:{kind:'scroll',x:20,y:20,delta_x:0,delta_y:120}}});
 await began.promise;chromium.child.signalCode='SIGSEGV';live=false;
 for(const handler of [...handlers])handler({method:'Chariox.browserDisconnected',params:{code:'browser_cdp_disconnected'}});
 if(late){for(let n=0;n<150&&host.generation===opened.generation;n++)await new Promise(r=>setTimeout(r,10));await host.starting;}
 gate.resolve();const reply=await action;
 assert.equal(reply.ok,false);assert.equal(reply.error.code,'browser_unavailable');assert.match(reply.error.message,/not replayed/);
 for(let n=0;n<150&&host.generation===opened.generation;n++)await new Promise(r=>setTimeout(r,10));await host.starting;
 assert.equal(host.generation,opened.generation+1);assert(host.browser);assert.equal(host.recovery.enabled,true);assert(host.tabs.has(opened.tab_id));assert.equal(presses,1,'the old input is never replayed');
}));

// MP-08/MP-10: an opaque absent Tab is obsolete, not evidence of a lost grant.
test('retired tab reads do not revoke an owner grant',async()=>{
 const {resolveDisplayTarget}=await import('./kernel-browser-display-document.mjs');
 const host=Object.assign(Object.create(KernelBrowserHost.prototype),{generation:1,tabs:new Map(),reconcile:async()=>{},timing:()=>{}});
 const command={generation:1,tab_id:'retired-tab'};
 for(const read of [()=>host.target(command),()=>host.mirrorTarget(command),()=>resolveDisplayTarget(host,command)])await assert.rejects(read(),e=>e.code==='user_domain_stale_reference'&&e.race==='display_retired');
});

// MP-08/MP-10: a state request carries agent groups while teardown has no live
// targets. That attribution update must not replace the last durable inventory.
test('group synchronization during recovery cannot save an empty registry',()=>using(async({host,chromium})=>{
 const first=await host.request({op:'open',url:'https://www.wikipedia.org/'});
 const second=await host.request({op:'open',url:'https://github.com/charioxai/chariox'});
 const groups=[{tab_id:second.tab_id,label:'Agent'}];await host.syncTabGroups(groups);
 const ids=[...host.tabs.keys()],old=host.browser;
 old.close=async()=>{host.tabs.clear();await host.syncTabGroups(groups)};
 chromium.child.signalCode='SIGSEGV';await host.request({op:'start'});
 assert.deepEqual([...host.tabs.keys()],ids);
 assert.deepEqual(host.savedGroups,groups);assert.equal(host.restoring,false);
}));

test('group updates during restoration do not apply a partial native inventory',()=>using(async({host})=>{
 const opened=await host.request({op:'open',url:'https://www.wikipedia.org/'});let applied=0;
 host.tabGroups={sync:async()=>{applied++}};host.restoring=true;
 const groups=[{tab_id:opened.tab_id,label:'Agent'}];await host.syncTabGroups(groups);
 assert.equal(applied,0);assert.deepEqual(host.savedGroups,groups);
 await host.syncTabGroups(groups,{restore:true});assert.equal(applied,1);host.restoring=false;
}));

// MP-08/MP-10: restoring the registry must retain selection, including a pending App.
test('restart retains the active regular Tab and active App origin', () => using(async ({host}, root) => {
  const first = await host.request({op:'open',url:'https://example.com/first'});
  await host.request({op:'open',url:'https://example.com/second'});
  host.activeTarget = host.tabs.get(first.tab_id).target_id;
  await host.save();
  let saved = JSON.parse(await readFile(path.join(root,'tabs.json'),'utf8'));
  assert.equal(saved.active_tab_id, first.tab_id);
  const opened = await host.request({op:'open',url:'https://app.todo.invalid/'});
  const targetId = host.tabs.get(opened.tab_id).target_id;
  host.browser.appTabs = {apps:new Map([['app',{targetId,origin:'https://app.todo.invalid'}]])};
  host.activeTarget = targetId;
  await host.save();
  saved = JSON.parse(await readFile(path.join(root,'tabs.json'),'utf8'));
  assert.equal(saved.active_app_origin,'https://app.todo.invalid');
  assert.equal(saved.tabs.some(tab=>tab.url.includes('app.todo.invalid')),false);
}));

test('MP-08/MP-10: active first Tab is selected after every saved Tab reopens', () => using(async ({host}, root) => {
  const first = await host.request({op:'open',url:'https://example.com/first'});
  await host.request({op:'open',url:'https://example.com/second'});
  host.activeTarget = host.tabs.get(first.tab_id).target_id;
  await host.save();await host.stop();await host.start();
  assert.equal(host.activeTarget, host.tabs.get(first.tab_id).target_id);
}));

test('MP-08/MP-10/MP-11: selection waits for all verified App rebindings', () => using(async ({host}, root) => {
  const todo = await host.request({op:'open',url:'https://app.todo.invalid/'});
  const documents = await host.request({op:'open',url:'https://app.documents.invalid/'});
  const app = (tab, origin) => ({targetId:host.tabs.get(tab.tab_id).target_id,origin,ready:true});
  host.browser.appTabs = {apps:new Map([['todo',app(todo,'https://app.todo.invalid')],['documents',app(documents,'https://app.documents.invalid')]])};
  host.activeTarget = host.tabs.get(todo.tab_id).target_id;await host.save();
  await host.stop();await host.start();
  const reboundTodo = await host.request({op:'open',url:'https://app.todo.invalid/'});
  host.browser.appTabs = {apps:new Map([['todo',app(reboundTodo,'https://app.todo.invalid')]])};
  await host.reconcile();assert(host.pendingActive);
  const reboundDocuments = await host.request({op:'open',url:'https://app.documents.invalid/'});
  host.browser.appTabs.apps.set('documents',{...app(reboundDocuments,'https://app.documents.invalid'),ready:false});
  await host.reconcile();assert(host.pendingActive,'an opening App may still take native focus');
  host.browser.appTabs.apps.get('documents').ready=true;
  await host.reconcile();
  assert.equal(host.activeTarget,host.tabs.get(reboundTodo.tab_id).target_id);
  assert.equal(host.pendingActive,null);
}));

// MP-08/MP-10/MP-11: mirrored input can target a page in a background Chrome window.
// Its admitted human intent must supersede a pending native-window restoration.
for (const actor of ['human','mirror-human','agent','denied']) test(`MP-08/MP-10/MP-11: ${actor} input preserves the correct pending selection`, () => using(async ({host,connection},root) => {
  const todo=await host.request({op:'open',url:'https://example.com/todo'});
  const docs=await host.request({op:'open',url:'https://example.com/documents'});
  const target=id=>host.tabs.get(id).target_id;let active=target(docs.tab_id);
  host.tabGroups={active:async()=>active};host.activeTarget=active;
  const send=connection.send.bind(connection);connection.send=async(method,params,session)=>{if(method==='Target.activateTarget')active=params.targetId;return send(method,params,session)};
  const pending={tab_id:docs.tab_id,app_origin:null,apps:[],until:Date.now()+30000};host.pendingActive=pending;
  fixture.secretFocused=false;
  if(actor==='mirror-human')host.mirror.resolveInput=async()=>({perform:async()=>{},fenced:false});
  const input=host.request({op:'input',tab_id:todo.tab_id,generation:todo.generation,document_id:host.tabs.get(todo.tab_id).document_id,...(actor==='agent'?{_agent_input:true}:{}),input:actor==='denied'?{kind:'click',x:-1,y:0}:actor==='mirror-human'?{kind:'mirror',action:{kind:'set',node_id:'n1',value:'unsent draft'}}:{kind:'text',text:'unsent draft'}});
  if(actor==='denied'){await assert.rejects(input,PointerOutsideViewport);assert.equal(host.pendingActive,pending)}
  else {await input;const saved=JSON.parse(await readFile(path.join(root,'tabs.json'),'utf8'));assert.equal(saved.active_tab_id,actor==='human'||actor==='mirror-human'?todo.tab_id:docs.tab_id)}
}));

// MP-08/MP-10/MP-11: a saved selection that can never return must not hide the live native selection.
test('MP-08/MP-10: an expired selection intent for an App that never binds yields to the native selection', () => using(async ({host,connection},root) => {
 const plain=await host.request({op:'open',url:'https://example.com/plain'});
 const target=host.tabs.get(plain.tab_id).target_id;
 host.tabGroups={active:async()=>target};host.activeTarget=target;
 host.pendingActive={tab_id:null,app_origin:'https://app.gone.invalid',apps:['https://app.gone.invalid'],until:Date.now()-1};
 await host.reconcile();
 assert.equal(host.pendingActive,null);
 assert.equal(host.activeTarget,target);
 assert.equal(JSON.parse(await readFile(path.join(root,'tabs.json'),'utf8')).active_tab_id,plain.tab_id);
}));

test('MP-08/MP-10: closing the selected Tab cancels its pending selection intent', () => using(async ({host}) => {
 const first=await host.request({op:'open',url:'https://example.com/first'});
 const second=await host.request({op:'open',url:'https://example.com/second'});
 // An origin that has not rebound keeps the restore unsettled, so only Close can cancel the intent.
 host.pendingActive={tab_id:first.tab_id,app_origin:null,apps:['https://app.unready.invalid'],until:Date.now()+30000};
 await host.request({op:'close',tab_id:first.tab_id,generation:first.generation,document_id:host.tabs.get(first.tab_id).document_id});
 assert.equal(host.pendingActive,null);
 assert(host.tabs.has(second.tab_id));
}));

test('MP-08/MP-11: restoring a selection retires native capture before activating its window', () => using(async ({host,connection}) => {
 const first=await host.request({op:'open',url:'https://example.com/first'});
 await host.request({op:'open',url:'https://example.com/second'});
 host.pendingActive={tab_id:first.tab_id,app_origin:null,apps:[],until:Date.now()};
 let retired=false;host.retireNativeForeground=async()=>{retired=true};
 const send=connection.send.bind(connection);connection.send=async(method,params,session)=>{if(method==='Target.activateTarget')assert(retired,'old native capture must retire before focus moves');return send(method,params,session)};
 await host.reconcile();
}));

for (const mirrored of [false,true]) test(`MP-08/MP-10/MP-11: ${mirrored?'DOM':'physical'} human input rechecks a stale matching selection`, () => using(async ({host,connection},root) => {
 const todo=await host.request({op:'open',url:'https://example.com/todo'});
 const docs=await host.request({op:'open',url:'https://example.com/documents'});
 const target=id=>host.tabs.get(id).target_id;let active=target(docs.tab_id),activations=0;
 host.tabGroups={active:async()=>active};host.activeTarget=target(todo.tab_id);
 const send=connection.send.bind(connection);connection.send=async(method,params,session)=>{if(method==='Target.activateTarget'){active=params.targetId;activations++}return send(method,params,session)};
 if(mirrored)host.mirror.resolveInput=async()=>({perform:async()=>{},fenced:false});
 const input={op:'input',tab_id:todo.tab_id,generation:todo.generation,document_id:host.tabs.get(todo.tab_id).document_id,input:mirrored?{kind:'mirror',action:{kind:'set',node_id:'n1',value:'draft'}}:{kind:'text',text:'draft'}};
 await host.request(input);
 assert.equal(host.activeTarget,target(todo.tab_id),'a delayed native change cannot replace the admitted human selection');
 assert.equal(active,target(todo.tab_id));assert.equal(activations,1);
 await host.request(input);assert.equal(activations,1,'already selected native tab requires no activation');
 const saved=JSON.parse(await readFile(path.join(root,'tabs.json'),'utf8'));assert.equal(saved.active_tab_id,todo.tab_id);
}));

// MP-08/MP-10/MP-11: an input can publish before its focus reconciliation settles.
test('pending human selection preserves every verified App restore barrier', () => using(async ({host},root) => {
 const todo=await host.request({op:'open',url:'https://app.todo.invalid/'});
 const docs=await host.request({op:'open',url:'https://app.documents.invalid/'});
 host.browser.appTabs={apps:new Map([['todo',{targetId:host.tabs.get(todo.tab_id).target_id,origin:'https://app.todo.invalid',ready:true}],['docs',{targetId:host.tabs.get(docs.tab_id).target_id,origin:'https://app.documents.invalid',ready:false}]])};
 host.pendingActive={tab_id:todo.tab_id,app_origin:'https://app.todo.invalid',apps:[],until:Date.now()};
 await host.save();let saved=JSON.parse(await readFile(path.join(root,'tabs.json'),'utf8'));
 assert.deepEqual(new Set(saved.app_origins),new Set(['https://app.todo.invalid','https://app.documents.invalid']));
 host.pendingActive.apps=['https://app.todo.invalid','https://app.pending.invalid'];await host.save();saved=JSON.parse(await readFile(path.join(root,'tabs.json'),'utf8'));
 assert.deepEqual(new Set(saved.app_origins),new Set(['https://app.todo.invalid','https://app.documents.invalid','https://app.pending.invalid']));
 assert.equal(saved.active_app_origin,'https://app.todo.invalid');
}));

// MP-08/MP-10/MP-11: App rebinding can focus a transient window before restore settles.
test('native selection stays unknown while the saved App selection is pending', () => using(async ({host}) => {
 const previous=process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;process.env.CHARIOX_KERNEL_BROWSER_DISPLAY='1';
 try {
  const todo=await host.request({op:'open',url:'https://app.todo.invalid/'});
  const docs=await host.request({op:'open',url:'https://app.documents.invalid/'});
  host.chromium.policy='managed';
  host.chromeWindows={bound:{tabbed:{isCurrent:()=>true,window_generation:1}},auxiliaryState:()=>[]};
  host.activeTarget=host.tabs.get(docs.tab_id).target_id;
  host.pendingActive={tab_id:todo.tab_id,app_origin:'https://app.todo.invalid',apps:['https://app.todo.invalid','https://app.documents.invalid'],until:Date.now()+30000};
  assert.equal(host.chromeWindow().active_tab_id,null,'a rebinding window is not a settled native selection');
  assert.equal(host.activeTarget,host.tabs.get(docs.tab_id).target_id,'physical input retains the actual native target');
  host.activeTarget=host.tabs.get(todo.tab_id).target_id;host.pendingActive=null;
  assert.equal(host.chromeWindow().active_tab_id,todo.tab_id);
  host.activeTarget=host.tabs.get(docs.tab_id).target_id;
  assert.equal(host.chromeWindow().active_tab_id,docs.tab_id,'later native changes remain visible');
 } finally {host.chromeWindows={retire(){}};if(previous===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_DISPLAY;else process.env.CHARIOX_KERNEL_BROWSER_DISPLAY=previous}
}));

// Review #1073 P1: an ordinary state read reconciles every Tab; a surviving Tab keeps its committed resize.
test('MP-08/MP-10 tab reconciliation keeps a surviving Tab layout and drops a retired one', () => using(async ({host}) => {
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 host.browser.setLayout=async()=>{};
 await host.setLayout(host.tabs.get(tab.tab_id),'resize',{width:900,height:600});
 const revision=host.layoutRevision(tab.tab_id);
 await host.request({op:'state'});
 assert.deepEqual(host.pageSize(tab.tab_id),{width:900,height:600},'a state read must not restore the canonical size under a resized page');
 assert.equal(host.layoutRevision(tab.tab_id),revision);
 host.layouts.set('host-tab-retired',{width:700,height:500,subscriptionId:'resize'});host.layoutRevisions.set('host-tab-retired',3);
 await host.request({op:'state'});
 assert.equal(host.layouts.has('host-tab-retired'),false);assert.equal(host.layoutRevisions.has('host-tab-retired'),false);
}));

// #1073 P2: a Tab left at an older size by a resize that failed part-way is not skipped because its owner is already 'resize'.
test('MP-08/MP-10 reconciliation re-lays out a Tab whose resize layout is older than the window', () => using(async ({host}) => {
 const opened=await host.request({op:'open',url:'about:blank'}),tab=opened.tabs[0];
 const applied=[];host.browser.setLayout=async(target,layout)=>{applied.push(layout)};
 await host.setLayout(host.tabs.get(tab.tab_id),'resize',{width:900,height:600});
 host.windowLayout={width:1000,height:700};applied.length=0;
 await host.request({op:'state'});
 assert.deepEqual(host.pageSize(tab.tab_id),{width:1000,height:700});assert.deepEqual(applied,[{width:1000,height:700}]);
 applied.length=0;await host.request({op:'state'});assert.deepEqual(applied,[],'an up-to-date Tab is not laid out again');
}));

// browserperf r2: the URL typed right after opening a Tab was lost because the new Tab reported the canonical size and the
// view resized the shared window again, retiring the band under the person's click.
test('MP-08/MP-10 a Tab opened in a resized window inherits the window layout', () => using(async ({host}) => {
 const opened=await host.request({op:'open',url:'about:blank'});
 const applied=[];host.browser.setLayout=async(target,layout)=>{applied.push([target,layout])};
 host.windowLayout={width:900,height:600};
 const next=await host.request({op:'open',url:'about:blank'});
 for(const tab of next.tabs)assert.deepEqual(host.pageSize(tab.tab_id),{width:900,height:600});
 assert(applied.length>=1&&applied.every(([,layout])=>layout.width===900&&layout.height===600));
 assert.equal(opened.tabs.length+1,next.tabs.length);
}));


test('MP-08/MP-10/MP-11: crashed tabs remain in host state and close under sustained memory pressure', () => using(async ({ host, connection, handlers, sent }) => {
  const opened = await host.request({ op: 'open', url: 'https://example.test/crashed' });
  await host.request({ op: 'open', url: 'https://example.test/healthy' });
  const send = connection.send.bind(connection);
  connection.close = async () => {};
  connection.send = async (method, params, session) => {
    const result = await send(method, params, session);
    if (method === 'Target.attachToTarget') return { sessionId: `session-${params.targetId}` };
    if (method === 'Target.closeTarget') return { success: true };
    return result;
  };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection,
    memory: new BrowserMemory({ availableMemory: () => 128 * 1024 * 1024 }) });
  host.browser = browser;
  const before = await host.request({ op: 'state' });
  const crashed = before.tabs.find(tab => tab.url.includes('/crashed'));
  const healthy = before.tabs.find(tab => tab.url.includes('/healthy'));
  const crashedTarget = host.tabs.get(crashed.tab_id).target_id;
  for (const handler of handlers) handler({ method: 'Inspector.targetCrashed', sessionId: `session-${crashedTarget}`, params: {} });
  assert.equal(browser.rendererRecovery.crashed.has(crashedTarget), true);
  await assert.rejects(browser.memory.requireHeadroom(), error => error.code === 'browser_memory_pressure');
  const start = sent.length;
  const state = await host.request({ op: 'state' });
  assert.equal(state.tabs.length, before.tabs.length);
  assert.equal(state.tabs.find(tab => tab.tab_id === crashed.tab_id).document_id, crashed.document_id);
  assert.ok(!sent.slice(start).some(call => call.session === `session-${crashedTarget}`), 'inventory must not query a dead renderer');
  const closedHealthy = await host.request({ op: 'close', generation: state.generation, tab_id: healthy.tab_id });
  assert.ok(!closedHealthy.tabs.some(tab => tab.tab_id === healthy.tab_id));
  const closedCrashed = await host.request({ op: 'close', generation: closedHealthy.generation, tab_id: crashed.tab_id });
  assert.ok(!closedCrashed.tabs.some(tab => tab.tab_id === crashed.tab_id));
  assert.equal(browser.rendererRecovery.crashed.size, 0);
  assert.equal(sent.filter(call => call.method === 'Page.reload').length, 0, 'pressure must hold recovery while close stays usable');
}));

test('MP-08/MP-10/MP-11: a crash arriving during inventory does not cancel recovery', () => using(async ({ host, connection, handlers, sent }) => {
  const opened = await host.request({ op: 'open', url: 'https://example.test/crashed' });
  await host.request({ op: 'open', url: 'https://example.test/healthy' });
  const send = connection.send.bind(connection);
  connection.close = async () => {};
  connection.send = async (method, params, session) => {
    const result = await send(method, params, session);
    if (method === 'Target.attachToTarget') return { sessionId: `session-${params.targetId}` };
    if (method === 'Target.closeTarget') return { success: true };
    return result;
  };
  const browser = new BrowserCdpClient({ connectionFactory: async () => connection,
    memory: new BrowserMemory({ availableMemory: () => 128 * 1024 * 1024 }) });
  host.browser = browser;
  const before = await host.request({ op: 'state' });
  const crashed = before.tabs.find(tab => tab.url.includes('/crashed'));
  const healthy = before.tabs.find(tab => tab.url.includes('/healthy'));
  const crashedTarget = host.tabs.get(crashed.tab_id).target_id;
  const inspect = browser.inspectPage.bind(browser); let injected = false;
  browser.inspectPage = async (c, target, ...args) => {
    if (target.targetId === crashedTarget && !injected) {
      injected = true;
      for (const handler of handlers) handler({ method: 'Inspector.targetCrashed', sessionId: `session-${crashedTarget}`, params: {} });
      const error = new Error('Page.getFrameTree: Target closed'); error.code = 'browser_cdp_command_failed'; throw error;
    }
    return inspect(c, target, ...args);
  };
  const start = sent.length;
  const state = await host.request({ op: 'state' });
  assert.equal(state.tabs.length, before.tabs.length);
  assert.equal(state.tabs.find(tab => tab.tab_id === crashed.tab_id).document_id, crashed.document_id);
  assert.equal(browser.rendererRecovery.crashed.has(crashedTarget), true, 'in-flight renderer failure must retain the scheduled recovery');
  const closedHealthy = await host.request({ op: 'close', generation: state.generation, tab_id: healthy.tab_id });
  assert.ok(!closedHealthy.tabs.some(tab => tab.tab_id === healthy.tab_id));
  const closedCrashed = await host.request({ op: 'close', generation: closedHealthy.generation, tab_id: crashed.tab_id });
  assert.ok(!closedCrashed.tabs.some(tab => tab.tab_id === crashed.tab_id));
  assert.equal(browser.rendererRecovery.crashed.size, 0);
  assert.equal(sent.filter(call => call.method === 'Page.reload').length, 0, 'pressure must hold recovery while close stays usable');
}));

test('MP-08 / MP-10 user-domain App open and bridge polling keep the native Chrome window', () => using(async ({host, sent}) => {
  const params = {origin_label:'native-app',installation_id:'installation-1',assets:[{path:'index.html',content_type:'text/html',body_base64:Buffer.from('<canvas/>').toString('base64')}]};
  const opened = await host.handle({id:1,method:'browser.app.open',params});
  assert.equal(opened.ok,true,JSON.stringify(opened));
  assert.equal(sent.find(call=>call.method==='Target.createTarget' && call.params.newWindow!==undefined)?.params.newWindow,false);
  await host.handle({id:2,method:'browser.app.open',params});
  await host.handle({id:3,method:'browser.app.calls',params:{wait_ms:0}});
  assert(!sent.some(call=>call.method==='Browser.setWindowBounds' && call.params.bounds.windowState==='fullscreen'));
}));

test('MP-08 / MP-10 opening and reopening an App refresh its native window before exposure', () => using(async ({host}) => {
  host.chromium.display = {};
  let nativeTarget = null;
  host.chromeWindows.resolve = async () => { nativeTarget = [...host.tabs.values()].at(-1)?.target_id ?? null };
  const params = {origin_label:'native-app',installation_id:'installation-1',assets:[{path:'index.html',content_type:'text/html',body_base64:Buffer.from('<canvas/>').toString('base64')}]};
  const first = await host.handle({id:1,method:'browser.app.open',params});
  assert.equal(first.ok,true,JSON.stringify(first));
  assert.equal(nativeTarget,first.result.target_id);
  nativeTarget = 'retired-window';
  const reopened = await host.handle({id:2,method:'browser.app.open',params});
  assert.equal(reopened.ok,true,JSON.stringify(reopened));
  assert.equal(nativeTarget,reopened.result.target_id);
}));

test('MP-08 / MP-10 a mirror stays inside the owned native page viewport', () => using(async ({host,chromium}) => {
  const previous=process.env.CHARIOX_KERNEL_BROWSER_MIRROR;
  process.env.CHARIOX_KERNEL_BROWSER_MIRROR='1';
  try {
    const opened=await host.request({op:'open',url:'about:blank'}),tab=host.tabs.get(opened.tab_id),applied=[];
    chromium.desktop={};
    host.browser.nativeViewport=async target=>({target_id:target,width:1272,height:709});
    host.browser.setLayout=async(target,layout)=>{applied.push({target,layout})};
    host.mirror.v2.begin=()=>{};
    const subscribe=options=>host.request({op:'mirror_subscribe',tab_id:tab.tab_id,generation:host.generation,device_scale_factor:1,wire:2,...options});
    await subscribe({layout_width:1920,layout_height:1080});
    assert.deepEqual(host.pageSize(tab.tab_id),{width:1272,height:708},'the native viewport, rounded inward for the codec, bounds the canonical page');
    assert.deepEqual(applied.at(-1),{target:tab.target_id,layout:{width:1272,height:708}});
    await subscribe({layout_width:900,layout_height:500});
    assert.deepEqual(host.pageSize(tab.tab_id),{width:900,height:500},'a smaller viewer keeps its requested layout');
    host.browser.nativeViewport=async()=>null;
    await assert.rejects(subscribe({}),{code:'user_domain_stale_reference'});
    assert.deepEqual(host.pageSize(tab.tab_id),{width:900,height:500},'an unavailable native query does not publish a partial layout');
  } finally {
    delete chromium.desktop;
    if(previous===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_MIRROR;else process.env.CHARIOX_KERNEL_BROWSER_MIRROR=previous;
  }
}));

test('MP-08 / MP-10 an App mirror applies its native cap through CDP before publication', () => using(async ({host,chromium,sent}) => {
  const previous=process.env.CHARIOX_KERNEL_BROWSER_MIRROR;
  process.env.CHARIOX_KERNEL_BROWSER_MIRROR='1';
  try {
    const opened=await host.handle({id:1,method:'browser.app.open',params:{origin_label:'layout-app',installation_id:'layout-app',assets:[{path:'index.html',content_type:'text/html',body_base64:Buffer.from('<button style="position:fixed;right:0;bottom:0">Save</button>').toString('base64')}]}});
    assert.equal(opened.ok,true,JSON.stringify(opened));
    const tab=[...host.tabs.values()].find(tab=>tab.target_id===opened.result.target_id);
    assert([...host.browser.appTabs.apps.values()].some(app=>app.targetId===tab.target_id));
    chromium.desktop={};
    host.browser.nativeViewport=async target=>({target_id:target,width:1272,height:709});
    const applied=()=>sent.filter(call=>call.method==='Emulation.setDeviceMetricsOverride'&&call.session===`session-${tab.target_id}`).at(-1)?.params;
    let publications=0;
    host.mirror.v2.begin=()=>{
      const size=host.pageSize(tab.tab_id),metrics=applied();
      assert.deepEqual({width:metrics.width,height:metrics.height},size,'the App must be resized before the mirror starts');
      publications++;
    };
    await host.request({op:'mirror_subscribe',tab_id:tab.tab_id,generation:host.generation,device_scale_factor:1,wire:2,layout_width:1920,layout_height:1080});
    assert.equal(publications,1);
    assert.deepEqual(host.pageSize(tab.tab_id),{width:1272,height:708});
    await host.browser.setLayout(tab.target_id,null);
    assert.deepEqual({width:applied().width,height:applied().height},{width:1280,height:800});
  } finally {
    delete chromium.desktop;
    if(previous===undefined)delete process.env.CHARIOX_KERNEL_BROWSER_MIRROR;else process.env.CHARIOX_KERNEL_BROWSER_MIRROR=previous;
  }
}, {realCdp:true}));
