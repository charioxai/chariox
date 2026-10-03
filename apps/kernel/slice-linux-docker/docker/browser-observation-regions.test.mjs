import assert from 'node:assert/strict';
import test from 'node:test';
import { locateBrowserRegions } from './browser-observation-regions.mjs';
import { handleBrowserControllerRequest } from './browser-controller.mjs';

const target = { kind: 'browser', target_id: 'target', document_id: 'document', node_ref: 'backend:42' };
function fixture({ stale = false, hidden = false, replaced = false } = {}) {
  const methods = [];
  return { methods, async resolvePageTarget() { return { sessionId: 'session', connection: { async send(method) {
    methods.push(method);
    if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'root', loaderId: stale ? 'other' : 'document' } } };
    if (method === 'Target.getTargets') return { targetInfos: [] };
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 7 };
    if (method === 'Runtime.evaluate') return { result: { value: [hidden ? 'hidden' : 'visible', 1] } };
    if (method === 'Browser.getWindowForTarget') return { bounds: { left: 0, top: 0, width: 800, height: 800, windowState: 'normal' } };
    if (method === 'Page.getLayoutMetrics') return { cssLayoutViewport: { clientHeight: 700, pageX: 0, pageY: 0 }, cssVisualViewport: { scale: 1 } };
    if (method === 'DOM.getBoxModel' && replaced) throw new Error('detached field');
    if (method === 'DOM.getBoxModel') return { model: { border: [10, 20, 110, 20, 110, 50, 10, 50] } };
    if (method === 'DOMSnapshot.captureSnapshot') return { strings: ['#text', 'synthetic-only', 'CANVAS'], documents: [{ nodes: { nodeName: [0, 2], nodeValue: [1], inputValue: { index: [0], value: [1] } }, layout: { nodeIndex: [0, 1], bounds: [[200, 10, 200, 20], [0, 300, 300, 100]] } }] };
    throw new Error(method);
  } } }; } };
}

test('MP-08/MP-10/MP-11 known field uses trusted layout and desktop offset', async () => {
  const browser = fixture();
  assert.deepEqual((await locateBrowserRegions([target], browser))[0], [10, 120, 100, 30]);
  assert.equal(browser.methods.includes('Runtime.callFunctionOn'), false, 'page overrides cannot choose the masked region');
});
test('MP-08/MP-10/MP-11 stale document drops only this observation', async () => {
  await assert.rejects(locateBrowserRegions([target], fixture({ stale: true })), /document changed/);
});
test('MP-08/MP-10/MP-11 copied raw text and opaque canvas are also masked', async () => {
  assert.deepEqual(await locateBrowserRegions([target], fixture(), ['synthetic-only']), [[10, 120, 100, 30], [200, 110, 200, 20], [0, 400, 300, 100], [0, 0, 800, 100], [0, 776, 800, 24]]);
});
test('MP-08/MP-10/MP-11 replacement controller is seeded before observation with no human prompt', async () => {
  const browser = { protectedValues: new Set(), async snapshot() {
    assert.equal(this.protectedValues.has('synthetic-only'), true);
    return { text: 'synthetic-only', benign: 'synthetic-onl' };
  } };
  const response = await handleBrowserControllerRequest({ id: 1, method: 'browser.snapshot', protected_values: ['synthetic-only'] }, { browser });
  assert.deepEqual(response.result, { text: '[redacted]', benign: 'synthetic-onl' });
});

test('MP-08/MP-10/MP-11 hidden protected tabs do not block a benign desktop observation', async () => {
  assert.deepEqual(await locateBrowserRegions([target], fixture({ hidden: true }), ['synthetic-only']), []);
});

test('MP-08/MP-10/MP-11 replaced field is re-located autonomously from trusted raw input layout', async () => {
  const regions = await locateBrowserRegions([target], fixture({ replaced: true }), ['synthetic-only']);
  assert.deepEqual(regions[0], [200, 110, 200, 20]);
});
