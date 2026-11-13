// MP-08 / MP-09 / MP-10 / MP-11: production host entry and renderer geometry,
// with deterministic CDP transport. Real-browser/provider drills remain separate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { runInNewContext } from 'node:vm';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { KernelBrowserHost, scheduleHostRequest } from './kernel-browser-host.mjs';
import { PassThrough } from 'node:stream';
import { BrowserControllerStdioServer } from './browser-controller.mjs';

async function withHost(run) {
  const root = await mkdtemp(path.join(tmpdir(), 'card-region-'));
  const host = new KernelBrowserHost(root), calls = [], document = {};
  const state = { document_id: 'doc', width: 800, height: 600, box: { left: 80, top: 90, right: 180, bottom: 140 } };
  const node = { isConnected: true, ownerDocument: document, getBoundingClientRect: () => state.box };
  const tab = { tab_id: 'one', target_id: 'target', document_id: 'doc', url: 'https://user:pass@example.com/login?token=private#fragment' };
  const connection = { subscribe: () => () => {}, async send(method, params) {
    calls.push(method);
    if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'frame', loaderId: state.document_id } } };
    if (method === 'DOM.resolveNode') return { object: { objectId: 'element' } };
    if (method === 'Runtime.callFunctionOn' && params.functionDeclaration.includes('function trackCardElement')) return {result:{objectId:'resolver'}};
    if (method === 'Runtime.callFunctionOn' && params.objectId === 'resolver') return {result:(state.replacement ?? node).isConnected ? {objectId:'element'} : {subtype:'null',value:null}};
    if (method === 'Runtime.callFunctionOn' && params.functionDeclaration.includes('function bindVerificationInput')) return {result:{objectId:'cleanup'}};
    if (method === 'Runtime.callFunctionOn') {
      const context = { document, innerWidth: state.width, innerHeight: state.height, setTimeout, clearTimeout,
        requestAnimationFrame: callback => setTimeout(callback, 5), cancelAnimationFrame: clearTimeout };
      const result = await runInNewContext(`(${params.functionDeclaration})`, context).call(state.replacement ?? node, params.arguments[0].value, params.arguments[1]?.objectId === 'resolver' ? () => state.replacement ?? (node.isConnected ? node : null) : undefined);
      return { result: { value: JSON.parse(JSON.stringify(result)) } };
    }
    return {};
  } };
  host.generation = 1; host.start = async () => {};
  host.tabs.set(tab.tab_id, tab);
  host.target = async command => ({ ...tab, tab_id: command.tab_id });
  host.displayTarget = async command => ({ ...tab, tab_id: command.tab_id, document_id: state.document_id });
  host.browser = { ensureConnection: async () => connection, ensureTargetSession: async () => 'session', ensureFocusWorld: async () => ({ contextId: 7 }),
    snapshot: async () => ({ document_id: state.document_id, dom_nodes: [{ node_ref: 'backend:42', document_index: 0 }], dom_documents: [] }) };
  const snapshot = (scope = 'agent:a', tab_id = 'one') => host.request({ op: 'snapshot', tab_id, generation: 1, observed_by: scope });
  const region = extra => host.request({ op: 'popup_region', tab_id: 'one', ref: 'backend:42', observed_by: 'agent:a', ...extra });
  try { await run({ host, state, node, calls, snapshot, region }); }
  finally { await rm(root, { recursive: true, force: true }); }
}

test('MP-11 card region requires a snapshot ref from the same observer and Tab', () => withHost(async ({ snapshot, region, calls }) => {
  await assert.rejects(region({}), { code: 'user_domain_stale_reference' });
  await snapshot();
  for (const invalid of [{ tab_id: 'two' }, { observed_by: 'agent:b' }, { ref: 'backend:999' }, { ref: 'backend:42;private' }]) {
    await assert.rejects(region(invalid), { code: 'user_domain_stale_reference' });
  }
  assert.equal(calls.includes('DOM.resolveNode'), false, 'foreign references never reach CDP');
  const result = await region({});
  assert.equal(result.state, 'present');
  assert.deepEqual(result.region.rect, { x: 64, y: 74, width: 132, height: 82 });
  assert.equal(result.region.origin, 'https://example.com', 'userinfo, query and fragment stay out of the card');
  assert.equal(calls.includes('Page.captureScreenshot'), false, 'card geometry does not create another display');
}));

test('an App confirmation region is found by the host in the Tab document, never from a reference the App supplies', () => withHost(async ({ host, region, calls }) => {
  const connection = await host.browser.ensureConnection(), base = connection.send, sent = [];
  let matches = [7];
  connection.send = async (method, params, session) => {
    sent.push({ method, params });
    if (method === 'DOM.getDocument') return { root: { nodeId: 1 } };
    if (method === 'DOM.querySelectorAll') return { nodeIds: matches };
    if (method === 'DOM.describeNode') return { node: { backendNodeId: 42 } };
    return base(method, params, session);
  };
  const found = await region({ card_id: 'app-confirm-1', app_confirmation: true, ref: undefined });
  assert.equal(found.state, 'present');
  assert.equal(found.region.ref, 'backend:42');
  assert.deepEqual(sent.find(call => call.method === 'DOM.querySelectorAll').params, { nodeId: 1, selector: '[data-chariox-confirmation]' });
  // The follow-up poll names the reference the host returned and needs no flag.
  const followed = await region({ card_id: 'app-confirm-1', ref: 'backend:42', generation: 1, document_id: 'doc' });
  assert.equal(followed.state, 'present');
  // Two matching elements mean the page tampered with the preview: nothing is shown.
  matches = [7, 8];
  await assert.rejects(region({ card_id: 'app-confirm-2', app_confirmation: true, ref: undefined }), { code: 'user_domain_stale_reference' });
  assert.equal(calls.includes('DOM.getDocument'), false, 'the base fake never saw the confirmation query');
}));

test('MP-08/MP-10 card geometry follows scrolling, movement and viewport resize without another snapshot', () => withHost(async ({ snapshot, region, state }) => {
  await snapshot();
  const initial = (await region({})).region;
  const after = ({ rect, viewport_width, viewport_height }) => ({ rect, viewport_width, viewport_height });
  const waiting = region({ generation: 1, document_id: 'doc', after: after(initial) });
  await delay(20);
  state.box = { left: 20, top: -10, right: 120, bottom: 40 };
  const moved = await waiting;
  assert.deepEqual(moved.region.rect, { x: 4, y: 0, width: 132, height: 56 });
  state.width = 100; state.height = 40;
  const resized = await region({ generation: 1, document_id: 'doc', after: after(moved.region) });
  assert.deepEqual(resized.region.rect, { x: 4, y: 0, width: 96, height: 40 });
  assert.equal(resized.region.viewport_width, 100);
  assert.equal(resized.region.viewport_height, 40);
  state.box = { left: 0, top: 0, right: 0, bottom: 0 };
  const hidden = await region({ generation: 1, document_id: 'doc', after: after(resized.region) });
  assert.equal(hidden.state, 'present', 'a connected element without layout remains live');
  assert.deepEqual(hidden.region.rect, { x: 0, y: 0, width: 0, height: 0 });
}));

test('MP-08/MP-11 removing the element or replacing its document settles condition_met', () => withHost(async ({ snapshot, region, node, state }) => {
  await snapshot();
  node.isConnected = false;
  assert.deepEqual(await region({ generation: 1, document_id: 'doc' }), { state: 'condition_met', reason: 'Browser element disappeared' });
  node.isConnected = true; state.document_id = 'replacement';
  assert.deepEqual(await region({ generation: 1, document_id: 'doc' }), { state: 'condition_met', reason: 'Browser document changed' });
  // A later snapshot must not rebind an already-open card to the replacement.
  await snapshot();
  assert.deepEqual(await region({ generation: 1, document_id: 'doc' }), { state: 'condition_met', reason: 'Browser document changed' });
}));

test('MP-08/MP-11 an isolated-frame reference maps its owned quad into the Tab viewport', () => withHost(async ({ host, snapshot, region, state, node }) => {
  const ref = 'frame:child:child-doc:backend:43';
  host.browser.snapshot = async () => ({ document_id: 'doc', dom_nodes: [], accessibility_nodes: [{ node_ref: ref }] });
  const connection = await host.browser.ensureConnection(), original = connection.send.bind(connection);
  const childFrame = { id: 'child', parentId: 'frame', loaderId: 'child-doc' };
  connection.send = async (method, params, session) => {
    if (method === 'Page.getFrameTree') return { frameTree: session === 'child-session'
      ? { frame: childFrame } : { frame: { id: 'frame', loaderId: state.document_id }, childFrames: [{ frame: childFrame }] } };
    if (method === 'Target.getTargets') return { targetInfos: [{ type: 'iframe', targetId: 'child', parentFrameId: 'frame' }] };
    if (method === 'Target.attachToTarget') return { sessionId: 'child-session' };
    if (method === 'DOM.getFrameOwner') return { backendNodeId: 99 };
    if (method === 'DOM.getBoxModel') return params.backendNodeId === 99
      ? { model: { content: [100, 120, 300, 120, 300, 220, 100, 220] } }
      : { model: { border: [5, 10, 55, 10, 55, 30, 5, 30] } };
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 8 };
    if (method === 'Runtime.evaluate') return { result: { value: session === 'child-session' ? [200, 100] : [800, 600] } };
    return original(method, params, session);
  };
  await snapshot();
  const mapped = await region({ ref });
  assert.equal(mapped.state, 'present');
  assert.deepEqual(mapped.region.rect, { x: 89, y: 114, width: 82, height: 52 });
  assert.equal(mapped.region.document_id, 'doc', 'the card stays bound to the containing Tab document');
  node.isConnected = false;
  assert.equal((await region({ ref })).state, 'condition_met', 'a detached child-frame element also closes its card');
}));

test('MP-08/MP-10 pending card geometry does not hold the browser input lane', async () => {
  const input = new PassThrough(), output = new PassThrough();
  let release, started, completed;
  const began = new Promise(resolve => started = resolve), held = new Promise(resolve => release = resolve), inputDone = new Promise(resolve => completed = resolve);
  output.on('data', bytes => { for (const line of bytes.toString().trim().split('\n')) if (line && JSON.parse(line).id === 2) completed(); });
  const server = new BrowserControllerStdioServer({ input, output, scheduleRequest: scheduleHostRequest, handleRequest: async request => {
    if (request.id === 1) { started(); await held; }
    return { id: request.id, ok: true, result: {} };
  } });
  const running = server.run();
  input.write(JSON.stringify({ id: 1, method: 'host.browser', params: { op: 'popup_region', tab_id: 'one', ref: 'backend:42' } }) + '\n');
  await began;
  input.write(JSON.stringify({ id: 2, method: 'host.browser', params: { op: 'input', tab_id: 'one' } }) + '\n');
  const progressed = await Promise.race([inputDone.then(() => true), delay(100).then(() => false)]);
  input.end(); release(); await running;
  assert(progressed, 'ordinary input must proceed while the card awaits its next geometry change');
});

test('MP-08/MP-11 active card anchors survive ref and observer-cache eviction, then release', () => withHost(async ({ host, snapshot, region }) => {
  await snapshot();
  const first = await region({ card_id: 'kept-card' });
  assert.equal(first.state, 'present');
  host.browser.snapshot = async () => ({ document_id: 'doc', dom_nodes: Array.from({ length: 10_001 }, (_, i) => ({ node_ref: `backend:${i + 100}`, document_index: 0 })) });
  await snapshot();
  assert.equal(host.cardReferences.get('agent:a').get('one').refs.has('backend:42'), false);
  assert.equal((await region({ card_id: 'kept-card', generation: 1, document_id: 'doc' })).state, 'present', 'evicting issued refs must preserve the live card anchor');
  for (let i = 0; i < 65; i++) await snapshot(`observer:${i}`);
  assert.equal(host.cardReferences.has('agent:a'), false);
  assert.equal((await region({ card_id: 'kept-card', generation: 1, document_id: 'doc' })).state, 'present', 'evicting the snapshot observer must preserve the live card anchor');
  assert.deepEqual(await region({ card_id: 'kept-card', release: true }), { released: true });
  assert.equal(host.cardAnchors.size, 0);
  const expired = await region({ card_id: 'kept-card', generation: 1, document_id: 'doc' });
  assert.equal(expired.state, 'unavailable', 'missing registry state cannot claim the element disappeared');
}));

test('MP-08/MP-11 abandoned card anchors expire and capacity refuses without evicting live anchors', () => withHost(async ({ host, snapshot, region }) => {
  await snapshot();
  await region({ card_id: 'abandoned' });
  host.cardAnchors.get('agent:a\0abandoned').expires = Date.now() - 1;
  host.cardReferences.clear();
  assert.equal((await region({ card_id: 'abandoned', generation: 1, document_id: 'doc' })).state, 'unavailable');
  assert.equal(host.cardAnchors.size, 0);
  await snapshot();
  for (let i = 0; i < 1_024; i++) host.cardAnchors.set(`other\0${i}`, { expires: Date.now() + 60_000 });
  await assert.rejects(region({ card_id: 'one-too-many' }), /Browser card limit reached/);
  assert.equal(host.cardAnchors.size, 1_024, 'existing active anchors are never evicted to admit another');
}));

test('MP-08/MP-09/MP-10/MP-11 verification requires a positive response and retains rejection after a site rerender', () => withHost(async ({host, snapshot, region, node}) => {
  const connection=await host.browser.ensureConnection(), listeners=new Set();
  connection.subscribe=listener=>{listeners.add(listener);return()=>listeners.delete(listener)};
  const original=connection.send.bind(connection);
  let accepted=false;
  connection.send=async(method,params,session)=>method==='Network.getResponseBody'?{body:JSON.stringify({accepted,token:'must-never-appear-in-receipt'})}:original(method,params,session);
  const verification={challenge_id:'challenge-one',url:'https://example.com/verify',accepted_json_pointer:'/accepted'};
  await snapshot();
  const initial=await region({card_id:'verification',verification});
  const emit=(method,params)=>{for(const listener of listeners)listener({sessionId:'session',method,params})};
  let inputSequence=0;
  const attempt=async(id,status)=>{
    const watch=host.cardAnchors.get('agent:a\0verification').verification;
    emit('Runtime.bindingCalled',{name:watch.bindingName,executionContextId:7,payload:JSON.stringify({sequence:++inputSequence,value:'never-record-this-code'})});
    emit('Network.requestWillBeSent',{requestId:id,frameId:'frame',loaderId:'doc',hasUserGesture:true,request:{method:'POST',url:verification.url,postData:'{"code":"never-record-this-code"}'}});
    emit('Network.responseReceived',{requestId:id,response:{status,url:verification.url}});
    emit('Network.loadingFinished',{requestId:id,encodedDataLength:128});
    await delay(0);
  };
  await attempt('rejected',422);
  node.isConnected=false;
  const rejected=await region({card_id:'verification',verification,generation:1,document_id:'doc',after:initial.region});
  assert.deepEqual(rejected,{state:'unavailable',reason:'Browser element disappeared'},'an unidentifiable replacement cannot retain a stale crop');
  node.isConnected=true;
  accepted=true;await attempt('accepted',200);
  const result=await region({card_id:'verification',verification,generation:1,document_id:'doc'});
  assert.equal(result.state,'condition_met');
  assert.equal(JSON.parse(result.reason).verification_result.result,'accepted');
  assert.ok(!JSON.stringify(result).includes('must-never-appear'));
  assert.ok(!JSON.stringify(result).includes('never-record-this-code'));
}));

test('MP-08/MP-11 verification cannot be rebound or completed by another document, target or endpoint', () => withHost(async({host,snapshot,region})=>{
  const connection=await host.browser.ensureConnection(),listeners=new Set();
  connection.subscribe=listener=>{listeners.add(listener);return()=>listeners.delete(listener)};
  await snapshot();
  const verification={challenge_id:'challenge-one',url:'https://example.com/verify',accepted_json_pointer:'/accepted'};
  await assert.rejects(region({card_id:'foreign-origin',verification:{...verification,url:'https://other.example/verify'}}));
  await region({card_id:'bound',verification});
  await assert.rejects(region({card_id:'bound',verification:{...verification,challenge_id:'different'}}));
  for(const [sessionId,frameId,loaderId,url] of [['foreign','frame','doc',verification.url],['session','other','doc',verification.url],['session','frame','old-doc',verification.url],['session','frame','doc',verification.url+'?code=private']]) {
    for(const listener of listeners)listener({sessionId,method:'Network.requestWillBeSent',params:{requestId:'foreign',frameId,loaderId,request:{method:'POST',url}}});
  }
  const result=await region({card_id:'bound',verification});
  assert.equal(result.state,'present');assert.equal(result.region.verification_result,undefined);
  await region({card_id:'bound',release:true});assert.equal(listeners.size,0);
}));

test('MP-08/MP-10/MP-11 a pending geometry poll follows the same challenge replacement', () => withHost(async ({ snapshot, region, state, node }) => {
  await snapshot();
  const command = { card_id: 'replacement', verification: { challenge_id: 'replacement', url: 'https://example.com/verify', accepted_json_pointer: '/accepted' } };
  const first = await region(command);
  const { rect, viewport_width, viewport_height } = first.region;
  const pending = region({ ...command, after: { rect, viewport_width, viewport_height } });
  await delay(30);
  node.isConnected = false;
  state.replacement = { ...node, isConnected: true, getBoundingClientRect: () => ({ left: 80, top: 190, right: 180, bottom: 240 }) };
  const replaced = await pending;
  assert.equal(replaced.state, 'present');
  assert.equal(replaced.region.rect.y, 174);
}));
