import assert from 'node:assert/strict';
import test from 'node:test';
import { locateBrowserRegions } from './browser-observation-regions.mjs';
import { handleBrowserControllerRequest } from './browser-controller.mjs';

const target = { kind: 'browser', target_id: 'target', document_id: 'document', node_ref: 'backend:42' };
function fixture({ stale = false, hidden = false, replaced = false, scrollbar = 0, noEcho = false, windowHeight = 800 } = {}) {
  const methods = [];
  return { methods, async resolvePageTarget() { return { sessionId: 'session', connection: { async send(method) {
    methods.push(method);
    if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'root', loaderId: stale ? 'other' : 'document' } } };
    if (method === 'Target.getTargets') return { targetInfos: [] };
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 7 };
    if (method === 'Runtime.evaluate') return { result: { value: [hidden ? 'hidden' : 'visible', 1, 700] } };
    if (method === 'Browser.getWindowForTarget') return { bounds: { left: 0, top: 0, width: 800, height: windowHeight, windowState: 'normal' } };
    if (method === 'Page.getLayoutMetrics') return { cssLayoutViewport: { clientHeight: 700 - scrollbar, pageX: 0, pageY: 0 }, cssVisualViewport: { scale: 1 } };
    if (method === 'DOM.getBoxModel' && replaced) throw new Error('detached field');
    if (method === 'DOM.getBoxModel') return { model: { border: [10, 20, 110, 20, 110, 50, 10, 50] } };
    if (method === 'DOMSnapshot.captureSnapshot' && noEcho) return { strings: [], documents: [{ nodes: {}, layout: {} }] };
    if (method === 'DOMSnapshot.captureSnapshot') return { strings: ['#text', 'synthetic-only', 'CANVAS'], documents: [{ nodes: { nodeName: [0, 2], nodeValue: [1], inputValue: { index: [0], value: [1] } }, layout: { nodeIndex: [0, 1], bounds: [[200, 10, 200, 20], [0, 300, 300, 100]] } }] };
    throw new Error(method);
  } } }; } };
}

test('MP-08/MP-10/MP-11 known field uses trusted layout and desktop offset', async () => {
  const browser = fixture();
  assert.deepEqual((await locateBrowserRegions([target], browser))[0], [10, 120, 100, 30]);
  assert.equal(browser.methods.includes('Runtime.callFunctionOn'), false, 'page overrides cannot choose the masked region');
});
test('MP-08/MP-10/MP-11 navigation prunes the old document and scans current echoes', async () => {
  const browser = fixture({ stale: true });
  const regions = await locateBrowserRegions([target], browser, ['synthetic-only']);
  assert.equal(browser.methods.includes('DOM.getBoxModel'), false);
  assert.deepEqual(regions[0], [200, 110, 200, 20]);
});
test('MP-08/MP-10/MP-11 horizontal scrollbar does not shift field masks', async () => {
  assert.deepEqual((await locateBrowserRegions([target], fixture({ scrollbar: 15 })))[0], [10, 120, 100, 30]);
});
test('MP-08/MP-10/MP-11 taskbar never renders document titles', async () => {
  const { readFile } = await import('node:fs/promises');
  const config = await readFile(new URL('./tint2rc', import.meta.url), 'utf8');
  assert.match(config, /^task_text = 0$/m);
  assert.match(config, /^tooltip = 0$/m);
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

// MP-08/MP-10/MP-11: absence must be confirmed by the browser, not by a failed RPC.
test('MP-08/MP-10/MP-11 closed page targets are pruned while replacement pages are scanned', async () => {
  const browser = fixture();
  browser.ensureConnection = async () => ({ send: async () => ({ targetInfos: [{ type: 'page', targetId: 'replacement' }] }) });
  const resolve = browser.resolvePageTarget;
  browser.resolvePageTarget = async id => { assert.equal(id, 'replacement'); return resolve(); };
  const regions = await locateBrowserRegions([target], browser, ['synthetic-only']);
  assert.equal(browser.methods.includes('DOM.getBoxModel'), false);
  assert.deepEqual(regions[0], [200, 110, 200, 20]);
});

test('MP-08/MP-10/MP-11 removed field with no remaining echoes does not block captures', async () => {
  assert.deepEqual(await locateBrowserRegions([target], fixture({ replaced: true, noEcho: true }), ['synthetic-only']), [[0, 0, 800, 100], [0, 776, 800, 24]]);
});

// MD-5: background user-domain content still has pixels in CDP captures.
test('MD-5 content capture reuses Room field/echo/opaque masks without native chrome offsets', async () => {
  const browser = fixture({ hidden: true, windowHeight: 600 });
  const regions = await locateBrowserRegions([target], browser, ['synthetic-only'], { contentTarget: 'target' });
  assert.deepEqual(regions, [[10, 20, 100, 30], [200, 10, 200, 20], [0, 300, 300, 100]]);
  assert(!browser.methods.includes('Browser.getWindowForTarget'));
  await assert.rejects(locateBrowserRegions([target], fixture({ windowHeight: 600 }), ['synthetic-only']), /unbound frame/);
});
