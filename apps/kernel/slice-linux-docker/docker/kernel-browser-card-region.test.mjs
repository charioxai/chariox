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
import { UserDomainRefusal } from './kernel-browser-refusal.mjs';
import { BrowserControllerStdioServer } from './browser-controller.mjs';

async function withHost(run) {
  const root = await mkdtemp(path.join(tmpdir(), 'card-region-'));
  const host = new KernelBrowserHost(root), calls = [], document = {};
  const state = { document_id: 'doc', width: 800, height: 600, box: { left: 80, top: 90, right: 180, bottom: 140 } };
  const node = { isConnected: true, ownerDocument: document, innerText: '', getBoundingClientRect: () => state.box };
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
      const result = await runInNewContext(`(${params.functionDeclaration})`, context).call(state.replacement ?? node, params.arguments[0].value, params.arguments[1]?.value ?? null, params.arguments[2]?.objectId === 'resolver' ? () => state.replacement ?? (node.isConnected ? node : null) : undefined);
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
  // The real host resolves a Tab only under its current generation, as its display target does.
  host.displayTarget = async command => {
    if (command.generation !== host.generation) throw new UserDomainRefusal('stale_epoch');
    return { tab_id: command.tab_id, target_id: 'target', document_id: 'doc', url: 'https://example.com/' };
  };
  await assert.rejects(region({ card_id: 'app-confirm-0', app_confirmation: true, ref: undefined }), { code: 'user_domain_stale_epoch' });
  const found = await region({ card_id: 'app-confirm-1', app_confirmation: true, ref: undefined, generation: 1 });
  assert.equal(found.state, 'present');
  assert.equal(found.region.ref, 'backend:42');
  assert.deepEqual(sent.find(call => call.method === 'DOM.querySelectorAll').params, { nodeId: 1, selector: '[data-chariox-confirmation]' });
  // The follow-up poll names the reference the host returned and needs no flag.
  const followed = await region({ card_id: 'app-confirm-1', ref: 'backend:42', generation: 1, document_id: 'doc' });
  assert.equal(followed.state, 'present');
  // Two matching elements mean the page tampered with the preview: nothing is shown.
  matches = [7, 8];
  await assert.rejects(region({ card_id: 'app-confirm-2', app_confirmation: true, ref: undefined, generation: 1 }), { code: 'user_domain_stale_reference' });
  assert.equal(calls.includes('DOM.getDocument'), false, 'the base fake never saw the confirmation query');
}));

test('after a browser restart a person card is bound again to the whole page (a plain card by the restart notice, a card that waits for text or a proof by unavailable "Browser restarted"); a rebind never carries a proof', () => withHost(async ({ host, snapshot, region, state }) => {
  await snapshot();
  const first = await region({ card_id: 'person-card' });
  assert.equal(first.state, 'present');
  // The kernel restarted: the Tab came back with a new document, and the controller a new generation.
  host.generation = 2; state.document_id = 'doc-2';
  // The restarted controller remembers nothing: no snapshot references, no anchors.
  host.cardReferences.clear(); host.cardAnchors.clear();
  const lost = await region({ card_id: 'person-card', generation: 1, document_id: 'doc' });
  assert.equal(lost.state, 'condition_met');
  assert.equal(lost.reason, 'Browser restarted');
  const connection = await host.browser.ensureConnection(), base = connection.send, sent = [];
  connection.send = async (method, params, session) => {
    sent.push({ method, params });
    if (method === 'DOM.getDocument') return { root: { nodeId: 1 } };
    if (method === 'DOM.querySelector') return { nodeId: 5 };
    if (method === 'DOM.describeNode') return { node: { backendNodeId: 77 } };
    return base(method, params, session);
  };
  const again = await region({ card_id: 'person-card', ref: undefined, rebind: true });
  assert.equal(again.state, 'present');
  assert.equal(again.region.ref, 'backend:77');
  assert.equal(again.region.generation, 2);
  assert.equal(again.region.document_id, 'doc-2');
  assert.equal(again.region.tab_id, 'one');
  assert.deepEqual(sent.find(call => call.method === 'DOM.querySelector').params, { nodeId: 1, selector: 'body' });
  // The poll that follows names the new binding and keeps following the page.
  const followed = await region({ card_id: 'person-card', ref: 'backend:77', generation: 2, document_id: 'doc-2' });
  assert.equal(followed.state, 'present');
  for (const refused of [{ app_confirmation: true }, { release: true }, { verification: { challenge_id: 'x', url: 'https://example.com/verify', accepted_json_pointer: '/accepted' } }]) {
    await assert.rejects(region({ card_id: 'person-card', ref: undefined, rebind: true, ...refused }), { code: 'user_domain_stale_reference' });
  }
  await assert.rejects(region({ card_id: undefined, ref: undefined, rebind: true }), { code: 'user_domain_stale_reference' });
}));
test('546 a done_text card is bound again to the page its Tab navigated to, keeps its text, and settles when the text shows there', () => withHost(async ({ host, snapshot, region, state, node }) => {
  await snapshot();
  const command = { card_id: 'person-nav', done_text: 'Welcome home' };
  const first = await region(command);
  assert.equal(first.state, 'present');
  // The page navigates inside the person's step (a sign-in redirect): the observation reports the change, the kernel rebinds.
  state.document_id = 'doc-2';
  const moved = await region({ ...command, generation: 1, document_id: 'doc' });
  assert.deepEqual(moved, { state: 'unavailable', reason: 'Browser document changed' });
  const connection = await host.browser.ensureConnection(), base = connection.send;
  connection.send = async (method, params, session) => {
    if (method === 'DOM.getDocument') return { root: { nodeId: 1 } };
    if (method === 'DOM.querySelector') return { nodeId: 5 };
    if (method === 'DOM.describeNode') return { node: { backendNodeId: 77 } };
    return base(method, params, session);
  };
  const again = await region({ card_id: 'person-nav', ref: undefined, rebind: true, done_text: 'Welcome home' });
  assert.equal(again.state, 'present');
  assert.equal(again.region.ref, 'backend:77');
  assert.equal(again.region.document_id, 'doc-2');
  assert.equal(again.region.done_text, 'Welcome home', 'the card keeps waiting for its text on the new page');
  const followed = await region({ card_id: 'person-nav', ref: 'backend:77', generation: 1, document_id: 'doc-2', done_text: 'Welcome home' });
  assert.equal(followed.state, 'present');
  node.innerText = 'Welcome Home, Ada';
  assert.deepEqual(await region({ card_id: 'person-nav', ref: 'backend:77', generation: 1, document_id: 'doc-2', done_text: 'Welcome home' }), { state: 'condition_met', reason: 'The page shows what the person was asked to reach' });
}));
test('546 a verification card survives its Tab navigating: it is bound to the new page without its proof, which lapsed with the old document', () => withHost(async ({ host, snapshot, region, state }) => {
  const verification = { challenge_id: 'challenge-nav', url: 'https://example.com/verify', accepted_json_pointer: '/accepted' };
  await snapshot();
  const first = await region({ card_id: 'verification-nav', verification });
  assert.equal(first.state, 'present');
  assert.ok(first.region.verification, 'the card watches for its proof');
  state.document_id = 'doc-2';
  // The observation reports the navigation as unavailable (the proof belongs to one document); the kernel rebinds, never completes.
  assert.deepEqual(await region({ card_id: 'verification-nav', verification, generation: 1, document_id: 'doc' }), { state: 'unavailable', reason: 'Browser document changed' });
  const connection = await host.browser.ensureConnection(), base = connection.send;
  connection.send = async (method, params, session) => {
    if (method === 'DOM.getDocument') return { root: { nodeId: 1 } };
    if (method === 'DOM.querySelector') return { nodeId: 5 };
    if (method === 'DOM.describeNode') return { node: { backendNodeId: 77 } };
    return base(method, params, session);
  };
  const again = await region({ card_id: 'verification-nav', ref: undefined, rebind: true });
  assert.equal(again.state, 'present');
  assert.equal(again.region.document_id, 'doc-2');
  assert.equal(again.region.verification, undefined, 'no proof is carried to a page where its input is gone');
  assert.equal(host.cardAnchors.get('agent:a\0verification-nav')?.verification, undefined);
}));
test('546 a second navigation while the card is being rebound leaves it rebindable: the next rebind binds the newest page', () => withHost(async ({ host, snapshot, region, state }) => {
  await snapshot();
  assert.equal((await region({ card_id: 'multi-step' })).state, 'present');
  state.document_id = 'doc-2';
  assert.equal((await region({ card_id: 'multi-step', generation: 1, document_id: 'doc' })).reason, 'Browser document changed');
  const connection = await host.browser.ensureConnection(), base = connection.send;
  let redirected = false;
  connection.send = async (method, params, session) => {
    if (method === 'DOM.getDocument') return { root: { nodeId: 1 } };
    if (method === 'DOM.querySelector') return { nodeId: 5 };
    if (method === 'DOM.describeNode') { if (!redirected) { redirected = true; state.document_id = 'doc-3'; } return { node: { backendNodeId: 77 } }; }
    return base(method, params, session);
  };
  // The page redirects again after the new anchor was installed: the rebind reports the change (never throws, never completes).
  const interrupted = await region({ card_id: 'multi-step', ref: undefined, rebind: true });
  assert.notEqual(interrupted.state, 'present');
  assert.equal(interrupted.reason, 'Browser document changed');
  // The kernel retries the rebind itself (the old reference is stale by now); it binds the newest page.
  const again = await region({ card_id: 'multi-step', ref: undefined, rebind: true });
  assert.equal(again.state, 'present');
  assert.equal(again.region.document_id, 'doc-3');
}));
test('a verification card whose proof was lost with the restart reports unavailable "Browser restarted" (the kernel keeps the card and rebinds it without the proof), never a completed card', () => withHost(async ({ host, snapshot, region }) => {
  const verification = { challenge_id: 'challenge-one', url: 'https://example.com/verify', accepted_json_pointer: '/accepted' };
  await snapshot();
  const first = await region({ card_id: 'verification', verification });
  assert.equal(first.state, 'present');
  assert.ok(host.cardAnchors.get('agent:a\0verification').verification, 'the card is watching for its proof');
  // The kernel restarted: a new controller generation that remembers no anchor and no snapshot reference.
  host.generation = 2;
  host.cardReferences.clear(); host.cardAnchors.clear();
  // The recovered card polls with the old generation and its verification condition.
  const lost = await region({ card_id: 'verification', verification, generation: 1, document_id: 'doc' });
  assert.equal(lost.state, 'unavailable', 'a lost proof is reported as unavailable, never as success; the kernel keeps the card pending and rebinds it');
  assert.equal(lost.reason, 'Browser restarted');
  // The same poll for a plain card is still the restart notice the kernel rebinds from.
  const plain = await region({ card_id: 'plain', generation: 1, document_id: 'doc' });
  assert.equal(plain.state, 'condition_met');
  assert.equal(plain.reason, 'Browser restarted');
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

test('546 an element that went away with a navigation is a changed document, not a removed element', () => withHost(async ({ snapshot, region, node, state }) => {
  await snapshot();
  node.isConnected = false; state.document_id = 'replacement';
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

test('AU-03 a done_text card follows a replaced element, reports unavailable when it is lost, and settles when the text shows', () => withHost(async ({ snapshot, region, state, node }) => {
  await snapshot();
  const command = { card_id: 'done', done_text: 'Verification accepted' };
  const first = await region(command);
  assert.equal(first.state, 'present');
  assert.equal(first.region.done_text, 'Verification accepted');
  const { rect, viewport_width, viewport_height } = first.region;
  const pending = region({ ...command, after: { rect, viewport_width, viewport_height } });
  await delay(30);
  // A rejected code makes the page rerender its form under the same id: not the requested text, so not done.
  node.isConnected = false;
  state.replacement = { ...node, isConnected: true, getBoundingClientRect: () => ({ left: 80, top: 190, right: 180, bottom: 240 }) };
  const replaced = await pending;
  assert.equal(replaced.state, 'present');
  assert.equal(replaced.region.rect.y, 174);
  state.replacement.innerText = 'Try again. Verification Accepted';
  assert.deepEqual(await region(command), { state: 'condition_met', reason: 'The page shows what the person was asked to reach' });
  state.replacement.innerText = '';
  state.replacement.isConnected = false;
  assert.deepEqual(await region(command), { state: 'unavailable', reason: 'Browser element disappeared' }, 'a lost target before the text is never completion');
}));

test('AU-03 a done_text card without a card identity reports unavailable when its element disappears, a plain card still settles', () => withHost(async ({ snapshot, region, node }) => {
  await snapshot();
  node.isConnected = false;
  assert.deepEqual(await region({ done_text: 'Verification accepted' }), { state: 'unavailable', reason: 'Browser element disappeared' });
  assert.deepEqual(await region({}), { state: 'condition_met', reason: 'Browser element disappeared' });
}));

test('AU-03 a done_text card cannot be rebound to another done text', () => withHost(async ({ snapshot, region }) => {
  await snapshot();
  await region({ card_id: 'done', done_text: 'Verification accepted' });
  await assert.rejects(region({ card_id: 'done', done_text: 'Something else' }), { code: 'user_domain_stale_reference' });
  await assert.rejects(region({ card_id: 'done' }), { code: 'user_domain_stale_reference' });
}));

// Row 655: a hidden hCaptcha challenge frame is parked far above the page (top:-10000px). Its buttons map to a
// box clipped to nothing; the card must be the empty crop, never a sized box at the page's top edge.
function childFrameConnection(host, state, frame) {
  return host.browser.ensureConnection().then(connection => {
    const original = connection.send.bind(connection);
    const childFrame = { id: 'child', parentId: 'frame', loaderId: 'child-doc' };
    connection.send = async (method, params, session) => {
      if (method === 'Page.getFrameTree') return { frameTree: session === 'child-session'
        ? { frame: childFrame } : { frame: { id: 'frame', loaderId: state.document_id }, childFrames: [{ frame: childFrame }] } };
      if (method === 'Target.getTargets') return { targetInfos: [{ type: 'iframe', targetId: 'child', parentFrameId: 'frame' }] };
      if (method === 'Target.attachToTarget') return { sessionId: 'child-session' };
      if (method === 'DOM.getFrameOwner') return { backendNodeId: 99 };
      if (method === 'DOM.getBoxModel' && params.backendNodeId !== 99 && frame.unrendered) throw new Error('Could not compute box model.');
      if (method === 'DOM.getBoxModel') return params.backendNodeId === 99
        ? { model: { content: frame.owner } } : { model: { border: [5, 10, 55, 10, 55, 30, 5, 30] } };
      if (method === 'DOM.resolveNode' && params.backendNodeId === 99) return { object: { objectId: 'owner' } };
      if (method === 'Runtime.callFunctionOn' && params.objectId === 'owner') return { result: { value: !frame.ownerHidden } };
      if (method === 'Page.createIsolatedWorld') return { executionContextId: 8 };
      if (method === 'Runtime.evaluate') return { result: { value: session === 'child-session' ? [200, 100] : [800, 600] } };
      return original(method, params, session);
    };
  });
}

test('655 a card on an element inside a hidden challenge frame is the empty crop, then follows the frame when it opens', () => withHost(async ({ host, snapshot, region, state }) => {
  const ref = 'frame:child:child-doc:backend:43';
  host.browser.snapshot = async () => ({ document_id: 'doc', dom_nodes: [], accessibility_nodes: [{ node_ref: ref }] });
  const frame = { owner: [0, -10000, 400, -10000, 400, -9500, 0, -9500] };
  await childFrameConnection(host, state, frame);
  await snapshot();
  const parked = await region({ ref, card_id: 'card' });
  assert.equal(parked.state, 'present');
  assert.deepEqual(parked.region.rect, { x: 0, y: 0, width: 0, height: 0 }, 'a box clipped to nothing is the empty crop');
  frame.owner = [100, 120, 300, 120, 300, 220, 100, 220];
  const opened = await region({ ref, card_id: 'card', generation: 1, document_id: 'doc', after: { rect: parked.region.rect, viewport_width: 800, viewport_height: 600 } });
  assert.deepEqual(opened.region.rect, { x: 89, y: 114, width: 82, height: 52 }, 'the card follows the frame when it opens');
  frame.ownerHidden = true;
  const closed = await region({ ref, card_id: 'card', generation: 1, document_id: 'doc', after: { rect: opened.region.rect, viewport_width: 800, viewport_height: 600 } });
  assert.deepEqual(closed.region.rect, { x: 0, y: 0, width: 0, height: 0 }, 'a frame hidden by its owner hides its elements');
}));

test('655 an element that is not rendered has no box: the empty crop, not a 0x0 box read as a position', () => withHost(async ({ host, snapshot, region, state }) => {
  const ref = 'frame:child:child-doc:backend:43';
  host.browser.snapshot = async () => ({ document_id: 'doc', dom_nodes: [], accessibility_nodes: [{ node_ref: ref }] });
  const frame = { owner: [100, 120, 300, 120, 300, 220, 100, 220], unrendered: true };
  await childFrameConnection(host, state, frame);
  await snapshot();
  const started = Date.now();
  const unrendered = await region({ ref });
  assert.deepEqual(unrendered.region.rect, { x: 0, y: 0, width: 0, height: 0 });
  assert.ok(Date.now() - started >= 900, 'registration waited the geometry window for the element to appear');
}));

test('655 registration waits the geometry window for an element that appears (an opening challenge), then binds its box', () => withHost(async ({ snapshot, region, state }) => {
  await snapshot();
  state.box = { left: 80, top: -10000, right: 180, bottom: -9950 };
  const waiting = region({});
  await delay(50);
  state.box = { left: 80, top: 90, right: 180, bottom: 140 };
  const shown = await waiting;
  assert.equal(shown.state, 'present');
  assert.deepEqual(shown.region.rect, { x: 64, y: 74, width: 132, height: 82 });
  state.box = { left: 80, top: -10000, right: 180, bottom: -9950 };
  const parked = await region({});
  assert.deepEqual(parked.region.rect, { x: 0, y: 0, width: 0, height: 0 }, 'an element above the viewport is the empty crop, not a strip at the top edge');
}));

test('666 a card on a challenge widget grows to the widget\'s visible frames of its own origin, never the page\'s other frames', () => withHost(async ({ host, snapshot, region, state }) => {
  const ref = 'frame:child:child-doc:backend:43';
  host.browser.snapshot = async () => ({ document_id: 'doc', dom_nodes: [], accessibility_nodes: [{ node_ref: ref }] });
  const connection = await host.browser.ensureConnection(), original = connection.send.bind(connection);
  const widget = 'https://widget.example', frames = {
    child: { frame: { id: 'child', parentId: 'frame', loaderId: 'child-doc', securityOrigin: widget, url: widget + '/checkbox' }, owner: 99, box: [20, 80, 320, 80, 320, 154, 20, 154] },
    challenge: { frame: { id: 'challenge', parentId: 'frame', loaderId: 'c-doc', securityOrigin: widget, url: widget + '/challenge' }, owner: 98, box: [330, 60, 730, 60, 730, 660, 330, 660] },
    ad: { frame: { id: 'ad', parentId: 'frame', loaderId: 'a-doc', securityOrigin: 'https://ads.example', url: 'https://ads.example/' }, owner: 97, box: [0, 700, 1280, 700, 1280, 790, 0, 790] },
  };
  const hidden = new Set(['challenge']);
  connection.send = async (method, params, session) => {
    if (method === 'Page.getFrameTree') {
      const own = Object.values(frames).find(f => session === f.frame.id + '-session');
      return { frameTree: own ? { frame: own.frame } : { frame: { id: 'frame', loaderId: state.document_id, securityOrigin: 'https://page.example', url: 'https://page.example/' }, childFrames: Object.values(frames).map(f => ({ frame: f.frame })) } };
    }
    if (method === 'Target.getTargets') return { targetInfos: Object.keys(frames).map(id => ({ type: 'iframe', targetId: id, parentFrameId: 'frame' })) };
    if (method === 'Target.attachToTarget') return { sessionId: params.targetId + '-session' };
    if (method === 'DOM.getFrameOwner') return { backendNodeId: frames[params.frameId].owner };
    if (method === 'DOM.resolveNode' && params.backendNodeId >= 97 && params.backendNodeId <= 99) return { object: { objectId: 'owner-' + params.backendNodeId } };
    if (method === 'Runtime.callFunctionOn' && String(params.objectId).startsWith('owner-')) {
      const id = Object.keys(frames).find(k => 'owner-' + frames[k].owner === params.objectId);
      return { result: { value: !hidden.has(id) } };
    }
    if (method === 'DOM.getBoxModel') {
      const f = Object.values(frames).find(v => v.owner === params.backendNodeId);
      return f ? { model: { content: f.box, border: f.box } } : { model: { border: [5, 10, 55, 10, 55, 30, 5, 30] } };
    }
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 8 };
    if (method === 'Runtime.evaluate') return { result: { value: String(session).endsWith('-session') ? [300, 74] : [1280, 800] } };
    return original(method, params, session);
  };
  await snapshot();
  const closed = await region({ ref, card_id: 'w' });
  const r = closed.region.rect;
  assert.ok(r.x <= 20 && r.x + r.width >= 320 && r.height < 200, `only the checkbox frame while the challenge is hidden: ${JSON.stringify(r)}`);
  hidden.delete('challenge');
  const open = (await region({ ref, card_id: 'w', generation: 1, document_id: 'doc', after: { rect: r, viewport_width: 1280, viewport_height: 800 } })).region.rect;
  assert.ok(open.x <= 20 && open.x + open.width >= 730 && open.y <= 60 && open.y + open.height >= 660, `the challenge with its bottom bar is in: ${JSON.stringify(open)}`);
  assert.ok(open.y + open.height < 700, 'the page\'s ad frame of another origin is not');
}));

test('666 a reCAPTCHA-style widget: a page element holding the anchor frame grows to the picture frame (bframe) each time it opens, and back', () => withHost(async ({ host, snapshot, region, state, node }) => {
  // The agent anchors on the page's own container of the anchor iframe; the bframe is a sibling frame of the same origin.
  const google = 'https://www.google.com', frames = {
    anchor: { frame: { id: 'anchor', parentId: 'frame', loaderId: 'a-doc', securityOrigin: google, url: google + '/recaptcha/api2/anchor' }, owner: 99, box: [40, 100, 344, 100, 344, 178, 40, 178] },
    bframe: { frame: { id: 'bframe', parentId: 'frame', loaderId: 'b-doc', securityOrigin: google, url: google + '/recaptcha/api2/bframe' }, owner: 98, box: [360, 20, 760, 20, 760, 600, 360, 600] },
  };
  let open = false;
  node.querySelector = selector => selector === 'iframe' ? {} : null;
  const connection = await host.browser.ensureConnection(), original = connection.send.bind(connection);
  connection.send = async (method, params, session) => {
    if (method === 'Page.getFrameTree') {
      const own = Object.values(frames).find(f => session === f.frame.id + '-session');
      return { frameTree: own ? { frame: own.frame } : { frame: { id: 'frame', loaderId: state.document_id, securityOrigin: 'https://shop.example', url: 'https://shop.example/' }, childFrames: Object.values(frames).map(f => ({ frame: f.frame })) } };
    }
    if (method === 'Target.getTargets') return { targetInfos: Object.keys(frames).map(id => ({ type: 'iframe', targetId: id, parentFrameId: 'frame' })) };
    if (method === 'Target.attachToTarget') return { sessionId: params.targetId + '-session' };
    if (method === 'DOM.getFrameOwner') return { backendNodeId: frames[params.frameId].owner };
    if (method === 'DOM.resolveNode' && (params.backendNodeId === 98 || params.backendNodeId === 99)) return { object: { objectId: 'owner-' + params.backendNodeId } };
    if (method === 'Runtime.callFunctionOn' && params.functionDeclaration.includes('this.contains(node)')) return { result: { value: params.arguments[0].objectId === 'owner-99' } };
    if (method === 'Runtime.callFunctionOn' && String(params.objectId).startsWith('owner-')) return { result: { value: params.objectId === 'owner-99' || open } };
    if (method === 'DOM.getBoxModel') { const f = Object.values(frames).find(v => v.owner === params.backendNodeId); if (f) return { model: { content: f.box, border: f.box } }; }
    return original(method, params, session);
  };
  state.box = { left: 0, top: 0, right: 1280, bottom: 790 };
  await snapshot();
  let last = (await region({ card_id: 'r' })).region;
  assert.ok(last.rect.width < 400 && last.rect.height < 200, `a page-wide element holding the widget is framed on the widget: ${JSON.stringify(last.rect)}`);
  const after = r => ({ generation: 1, document_id: 'doc', card_id: 'r', after: { rect: r.rect, viewport_width: r.viewport_width, viewport_height: r.viewport_height } });
  for (let round = 0; round < 3; round++) {
    open = true;
    const grown = (await region(after(last))).region;
    assert.ok(grown.rect.x + grown.rect.width >= 760 && grown.rect.y <= 20 && grown.rect.y + grown.rect.height >= 600, `round ${round}: the picture frame is in: ${JSON.stringify(grown.rect)}`);
    if (round === 0) {
      // Review P2: a card that grew waits on its anchor's own geometry: a stationary open challenge is a long read, not a spin.
      const started = Date.now();
      const still = (await region(after(grown))).region;
      assert.ok(Date.now() - started >= 900, `a stationary open challenge waits the geometry window (${Date.now() - started} ms)`);
      assert.deepEqual(still.rect, grown.rect);
    }
    open = false;
    last = (await region(after(grown))).region;
    assert.ok(last.rect.x + last.rect.width < 400, `round ${round}: closed again, only the anchor: ${JSON.stringify(last.rect)}`);
  }
}));

test('666 review: a widget whose frames render in the page\'s own process (no iframe targets) also grows', () => withHost(async ({ host, snapshot, region, state, node }) => {
  // The agent anchors on the page's own container of the anchor iframe; the bframe is a sibling frame of the same origin.
  const google = 'https://www.google.com', frames = {
    anchor: { frame: { id: 'anchor', parentId: 'frame', loaderId: 'a-doc', securityOrigin: google, url: google + '/recaptcha/api2/anchor' }, owner: 99, box: [40, 100, 344, 100, 344, 178, 40, 178] },
    bframe: { frame: { id: 'bframe', parentId: 'frame', loaderId: 'b-doc', securityOrigin: google, url: google + '/recaptcha/api2/bframe' }, owner: 98, box: [360, 20, 760, 20, 760, 600, 360, 600] },
  };
  let open = false;
  node.querySelector = selector => selector === 'iframe' ? {} : null;
  const connection = await host.browser.ensureConnection(), original = connection.send.bind(connection);
  connection.send = async (method, params, session) => {
    if (method === 'Page.getFrameTree') {
      const own = Object.values(frames).find(f => session === f.frame.id + '-session');
      return { frameTree: own ? { frame: own.frame } : { frame: { id: 'frame', loaderId: state.document_id, securityOrigin: 'https://shop.example', url: 'https://shop.example/' }, childFrames: Object.values(frames).map(f => ({ frame: f.frame })) } };
    }
    if (method === 'Target.getTargets') return { targetInfos: [] };
    if (method === 'Target.attachToTarget') return { sessionId: params.targetId + '-session' };
    if (method === 'DOM.getFrameOwner') return { backendNodeId: frames[params.frameId].owner };
    if (method === 'DOM.resolveNode' && (params.backendNodeId === 98 || params.backendNodeId === 99)) return { object: { objectId: 'owner-' + params.backendNodeId } };
    if (method === 'Runtime.callFunctionOn' && params.functionDeclaration.includes('this.contains(node)')) return { result: { value: params.arguments[0].objectId === 'owner-99' } };
    if (method === 'Runtime.callFunctionOn' && String(params.objectId).startsWith('owner-')) return { result: { value: params.objectId === 'owner-99' || open } };
    if (method === 'DOM.getBoxModel') { const f = Object.values(frames).find(v => v.owner === params.backendNodeId); if (f) return { model: { content: f.box, border: f.box } }; }
    return original(method, params, session);
  };
  state.box = { left: 30, top: 90, right: 354, bottom: 188 };
  await snapshot();
  let last = (await region({ card_id: 'r' })).region;
  const after = r => ({ generation: 1, document_id: 'doc', card_id: 'r', after: { rect: r.rect, viewport_width: r.viewport_width, viewport_height: r.viewport_height } });
  for (let round = 0; round < 1; round++) {
    open = true;
    const grown = (await region(after(last))).region;
    assert.ok(grown.rect.x + grown.rect.width >= 760 && grown.rect.y <= 20 && grown.rect.y + grown.rect.height >= 600, `round ${round}: the picture frame is in: ${JSON.stringify(grown.rect)}`);
    if (round === 0) {
      // Review P2: a card that grew waits on its anchor's own geometry: a stationary open challenge is a long read, not a spin.
      const started = Date.now();
      const still = (await region(after(grown))).region;
      assert.ok(Date.now() - started >= 900, `a stationary open challenge waits the geometry window (${Date.now() - started} ms)`);
      assert.deepEqual(still.rect, grown.rect);
    }
    open = false;
    last = (await region(after(grown))).region;
    assert.ok(last.rect.x + last.rect.width < 400, `round ${round}: closed again, only the anchor: ${JSON.stringify(last.rect)}`);
  }
}));

test('666 final rerun: cross-process widget frames missing from the page\'s own frame tree (real Chrome) still grow the card', () => withHost(async ({ host, snapshot, region, state }) => {
  const ref = 'frame:child:child-doc:backend:43';
  host.browser.snapshot = async () => ({ document_id: 'doc', dom_nodes: [], accessibility_nodes: [{ node_ref: ref }] });
  const connection = await host.browser.ensureConnection(), original = connection.send.bind(connection);
  const widget = 'https://widget.example', frames = {
    child: { frame: { id: 'child', parentId: 'frame', loaderId: 'child-doc', securityOrigin: widget, url: widget + '/checkbox' }, owner: 99, box: [20, 80, 320, 80, 320, 154, 20, 154] },
    challenge: { frame: { id: 'challenge', parentId: 'frame', loaderId: 'c-doc', securityOrigin: widget, url: widget + '/challenge' }, owner: 98, box: [330, 60, 730, 60, 730, 660, 330, 660] },
    ad: { frame: { id: 'ad', parentId: 'frame', loaderId: 'a-doc', securityOrigin: 'https://ads.example', url: 'https://ads.example/' }, owner: 97, box: [0, 700, 1280, 700, 1280, 790, 0, 790] },
  };
  const hidden = new Set(['challenge']);
  connection.send = async (method, params, session) => {
    if (method === 'Page.getFrameTree') {
      const own = Object.values(frames).find(f => session === f.frame.id + '-session');
      return { frameTree: own ? { frame: own.frame } : { frame: { id: 'frame', loaderId: state.document_id, securityOrigin: 'https://page.example', url: 'https://page.example/' }, childFrames: [] } };
    }
    if (method === 'Target.getTargets') return { targetInfos: Object.keys(frames).map(id => ({ type: 'iframe', targetId: id, parentFrameId: 'frame' })) };
    if (method === 'Target.attachToTarget') return { sessionId: params.targetId + '-session' };
    if (method === 'DOM.getFrameOwner') return { backendNodeId: frames[params.frameId].owner };
    if (method === 'DOM.resolveNode' && params.backendNodeId >= 97 && params.backendNodeId <= 99) return { object: { objectId: 'owner-' + params.backendNodeId } };
    if (method === 'Runtime.callFunctionOn' && String(params.objectId).startsWith('owner-')) {
      const id = Object.keys(frames).find(k => 'owner-' + frames[k].owner === params.objectId);
      return { result: { value: !hidden.has(id) } };
    }
    if (method === 'DOM.getBoxModel') {
      const f = Object.values(frames).find(v => v.owner === params.backendNodeId);
      return f ? { model: { content: f.box, border: f.box } } : { model: { border: [5, 10, 55, 10, 55, 30, 5, 30] } };
    }
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 8 };
    if (method === 'Runtime.evaluate') return { result: { value: String(session).endsWith('-session') ? [300, 74] : [1280, 800] } };
    return original(method, params, session);
  };
  await snapshot();
  const closed = await region({ ref, card_id: 'w' });
  const r = closed.region.rect;
  assert.ok(r.x <= 20 && r.x + r.width >= 320 && r.height < 200, `only the checkbox frame while the challenge is hidden: ${JSON.stringify(r)}`);
  hidden.delete('challenge');
  const open = (await region({ ref, card_id: 'w', generation: 1, document_id: 'doc', after: { rect: r, viewport_width: 1280, viewport_height: 800 } })).region.rect;
  assert.ok(open.x <= 20 && open.x + open.width >= 730 && open.y <= 60 && open.y + open.height >= 660, `the challenge with its bottom bar is in: ${JSON.stringify(open)}`);
  assert.ok(open.y + open.height < 700, 'the page\'s ad frame of another origin is not');
}));

test('655 one card per Tab: a plain card follows the newer element the agent named on its Tab; a page-text card never switches', () => withHost(async ({ host, snapshot, region }) => {
  host.browser.snapshot = async () => ({ document_id: 'doc', dom_nodes: [{ node_ref: 'backend:42', document_index: 0 }, { node_ref: 'backend:43', document_index: 0 }], dom_documents: [] });
  await snapshot();
  assert.equal((await region({ card_id: 'one' })).state, 'present');
  const moved = await region({ card_id: 'one', ref: 'backend:43' });
  assert.equal(moved.state, 'present', 'the updated card follows the new element');
  assert.equal(moved.region.ref, 'backend:43');
  await assert.rejects(region({ card_id: 'one', ref: 'backend:42', done_text: 'Signed in' }), /refused|stale_reference/, 'a card cannot change into a page-text card on another element');
}));

test('655 a hidden anchor inside a widget (the closed challenge\'s Verify) shows the widget\'s visible checkbox, not the whole page, and says the anchor is hidden', () => withHost(async ({ host, snapshot, region, state }) => {
  const ref = 'frame:challenge:c-doc:backend:43';
  host.browser.snapshot = async () => ({ document_id: 'doc', dom_nodes: [], accessibility_nodes: [{ node_ref: ref }] });
  const connection = await host.browser.ensureConnection(), original = connection.send.bind(connection);
  const widget = 'https://widget.example', frames = {
    child: { frame: { id: 'child', parentId: 'frame', loaderId: 'child-doc', securityOrigin: widget, url: widget + '/checkbox' }, owner: 99, box: [20, 80, 320, 80, 320, 154, 20, 154] },
    challenge: { frame: { id: 'challenge', parentId: 'frame', loaderId: 'c-doc', securityOrigin: widget, url: widget + '/challenge' }, owner: 98, box: [330, 60, 730, 60, 730, 660, 330, 660] },
    ad: { frame: { id: 'ad', parentId: 'frame', loaderId: 'a-doc', securityOrigin: 'https://ads.example', url: 'https://ads.example/' }, owner: 97, box: [0, 700, 1280, 700, 1280, 790, 0, 790] },
  };
  const hidden = new Set(['challenge']);
  connection.send = async (method, params, session) => {
    if (method === 'Page.getFrameTree') {
      const own = Object.values(frames).find(f => session === f.frame.id + '-session');
      return { frameTree: own ? { frame: own.frame } : { frame: { id: 'frame', loaderId: state.document_id, securityOrigin: 'https://page.example', url: 'https://page.example/' }, childFrames: Object.values(frames).map(f => ({ frame: f.frame })) } };
    }
    if (method === 'Target.getTargets') return { targetInfos: Object.keys(frames).map(id => ({ type: 'iframe', targetId: id, parentFrameId: 'frame' })) };
    if (method === 'Target.attachToTarget') return { sessionId: params.targetId + '-session' };
    if (method === 'DOM.getFrameOwner') return { backendNodeId: frames[params.frameId].owner };
    if (method === 'DOM.resolveNode' && params.backendNodeId >= 97 && params.backendNodeId <= 99) return { object: { objectId: 'owner-' + params.backendNodeId } };
    if (method === 'Runtime.callFunctionOn' && String(params.objectId).startsWith('owner-')) {
      const id = Object.keys(frames).find(k => 'owner-' + frames[k].owner === params.objectId);
      return { result: { value: !hidden.has(id) } };
    }
    if (method === 'DOM.getBoxModel') {
      const f = Object.values(frames).find(v => v.owner === params.backendNodeId);
      return f ? { model: { content: f.box, border: f.box } } : { model: { border: [5, 10, 55, 10, 55, 30, 5, 30] } };
    }
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 8 };
    if (method === 'Runtime.evaluate') return { result: { value: String(session).endsWith('-session') ? [300, 74] : [1280, 800] } };
    return original(method, params, session);
  };
  await snapshot();
  const parked = await region({ ref, card_id: 'h' });
  assert.equal(parked.state, 'present');
  assert.equal(parked.anchor_hidden, true, 'the host says the named element itself is hidden');
  assert.ok(parked.region.rect.x <= 20 && parked.region.rect.x + parked.region.rect.width >= 320 && parked.region.rect.width < 400, `the visible checkbox frame, not the whole page: ${JSON.stringify(parked.region.rect)}`);
}));
