import assert from 'node:assert/strict';
import test from 'node:test';
import { performBrowserAction } from './browser-controller-actions.mjs';
import { inspectInteractionTarget } from './browser-controller-interactions.mjs';

// MP-08/MP-10/MP-11: first-party observed-target interaction, no site knowledge.
function fixture() {
  const calls = [];
  const connection = { calls, async send(method, params = {}) {
    calls.push({method, params});
    if (method === 'Page.getFrameTree') return {frameTree: {frame: {loaderId: 'doc'}}};
    if (method === 'DOM.resolveNode') return {object: {objectId: 'field'}};
    if (method === 'Runtime.callFunctionOn') {
      if (params.functionDeclaration.includes('scrollIntoView')) return {result: {value: {
        state: 'ready', x: 100, y: 80, width: 40, height: 20,
      }}};
      return {result: {value: {ok: true}}};
    }
    if (method === 'Page.getLayoutMetrics') return {cssVisualViewport: {clientWidth: 800, clientHeight: 600}};
    return {};
  }};
  return connection;
}
const perform = (connection, action, extra = {}) => performBrowserAction({connection, action,
  sessionId: 'session', targetId: 'target', documentId: 'doc', nodeRef: 'backend:1', sleep: async () => {}, ...extra});

test('MP-08/MP-10/MP-11 missing ARIA value cannot verify zero and readonly drag rejects before input', async () => {
  const target = {isConnected:true, tagName:'DIV', getAttribute:name => name === 'role' ? 'slider' : null};
  assert.equal(inspectInteractionTarget.call(target, '0', false).ok, false);
  target.getAttribute = name => ({role:'slider','aria-readonly':'true','aria-valuenow':'5'}[name] ?? null);
  const c = fixture(), send = c.send.bind(c);
  c.send = async (method, params) => {
    if (method === 'Runtime.callFunctionOn' && !params.functionDeclaration.includes('scrollIntoView'))
      return {result:{value:inspectInteractionTarget.call(target, '6', false, false)}};
    return send(method, params);
  };
  await assert.rejects(perform(c, {kind:'drag',delta_x:60,delta_y:0,expected_value:'6'}));
  assert.equal(c.calls.some(c => c.method.startsWith('Input.')), false);
});

test('MP-08/MP-10/MP-11 observed key delivers one native key pair and verifies value', async () => {
  const c = fixture();
  const result = await perform(c, {kind: 'press', key: 'ArrowRight', expected_value: '13'});
  assert.equal(result.action_kind, 'press');
  assert.deepEqual(c.calls.filter(c => c.method === 'Input.dispatchKeyEvent').map(c => c.params.type), ['keyDown', 'keyUp']);
  assert.ok(c.calls.some(c => c.method === 'Runtime.callFunctionOn' && c.params.arguments?.[0]?.value === '13'));
});

test('MP-08/MP-10/MP-11 observed drag follows bounded geometry with one press/release', async () => {
  const c = fixture();
  assert.equal((await perform(c, {kind: 'drag', delta_x: 60, delta_y: -20})).action_kind, 'drag');
  const events = c.calls.filter(c => c.method === 'Input.dispatchMouseEvent').map(c => c.params);
  assert.equal(events.filter(e => e.type === 'mousePressed').length, 1);
  assert.equal(events.filter(e => e.type === 'mouseReleased').length, 1);
  assert.deepEqual([events.at(-1).x, events.at(-1).y], [160, 60]);
});

test('MP-08/MP-10/MP-11 malformed interaction and off-viewport drag reject before input', async () => {
  for (const action of [{kind:'press', key:'Control+L'}, {kind:'press',key:'ArrowRight',expected_value:'NaN'},
    {kind:'drag',delta_x:NaN,delta_y:0}, {kind:'drag',delta_x:2049,delta_y:0},
    {kind:'drag',delta_x:0,delta_y:0}, {kind:'drag',delta_x:-101,delta_y:0}]) {
    const c = fixture();
    await assert.rejects(perform(c, action), e => e.code === 'browser_action_invalid');
    assert.equal(c.calls.some(c => c.method.startsWith('Input.')), false);
  }
});

test('MP-08/MP-10/MP-11 lost drag acknowledgement releases input without replay', async () => {
  const c = fixture(), send = c.send.bind(c);
  c.send = async (method, params) => {
    const result = await send(method, params);
    if (method === 'Input.dispatchMouseEvent' && params.type === 'mouseMoved' && params.buttons === 1)
      throw Object.assign(new Error('synthetic lost acknowledgement'), {code:'browser_cdp_timeout'});
    return result;
  };
  await assert.rejects(perform(c, {kind:'drag',delta_x:60,delta_y:0}), {code:'browser_cdp_timeout'});
  const types = c.calls.filter(c => c.method === 'Input.dispatchMouseEvent').map(c => c.params.type);
  assert.equal(types.filter(t => t === 'mousePressed').length, 1);
  assert.equal(types.filter(t => t === 'mouseReleased').length, 1);
});
