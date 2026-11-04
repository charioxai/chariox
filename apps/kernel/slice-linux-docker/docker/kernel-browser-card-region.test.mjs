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
  const connection = { async send(method, params) {
    calls.push(method);
    if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'frame', loaderId: state.document_id } } };
    if (method === 'DOM.resolveNode') return { object: { objectId: 'element' } };
    if (method === 'Runtime.callFunctionOn') {
      const context = { document, innerWidth: state.width, innerHeight: state.height, setTimeout, clearTimeout,
        requestAnimationFrame: callback => setTimeout(callback, 5), cancelAnimationFrame: clearTimeout };
      const result = await runInNewContext(`(${params.functionDeclaration})`, context).call(node, params.arguments[0].value);
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
