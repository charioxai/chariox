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
test('MP-08/MP-10/MP-11 copied raw text is masked; MP-11 (owner 2026-10-08) media is not masked whole', async () => {
  assert.deepEqual(await locateBrowserRegions([target], fixture(), ['synthetic-only']), [[10, 120, 100, 30], [200, 110, 200, 20], [0, 0, 800, 100], [0, 776, 800, 24]]);
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
test('MD-5 content capture reuses Room field/echo masks without native chrome offsets', async () => {
  const browser = fixture({ hidden: true, windowHeight: 600 });
  const regions = await locateBrowserRegions([target], browser, ['synthetic-only'], { contentTarget: 'target' });
  assert.deepEqual(regions, [[10, 20, 100, 30], [200, 10, 200, 20]]);
  assert(!browser.methods.includes('Browser.getWindowForTarget'));
  await assert.rejects(locateBrowserRegions([target], fixture({ windowHeight: 600 }), ['synthetic-only']), /unbound frame/);
});

// MP-11 (owner 2026-10-08): frames are scanned, not masked whole. Strings:
// 0 '#text' 1 echo 2 'IFRAME' 3 'IMG' 4 'plain'.
function framed({ transformed = false, hiddenOwner = false, unlinked = false } = {}) {
  const methods = [];
  const snapshots = {
    // top: an in-process child document (doc 1) owned by node 1 (backend 11),
    // an isolated owner node 2 (backend 12), a stray iframe 3 (backend 13), an image.
    session: { strings: ['#text', 'synthetic-only', 'IFRAME', 'IMG', 'plain'], documents: [
      { scrollOffsetX: 0, scrollOffsetY: 50, nodes: { nodeName: [0, 2, 2, 2, 3], nodeValue: [4], backendNodeId: [10, 11, 12, 13, 14], contentDocumentIndex: { index: [1], value: [1] } },
        layout: { nodeIndex: [1, 2, 3, 4], bounds: [[100, 100, 310, 210], [500, 100, 306, 206], [900, 100, 50, 50], [0, 400, 300, 100]] } },
      { scrollOffsetX: 0, scrollOffsetY: 0, nodes: { nodeName: [0], nodeValue: [1], backendNodeId: [20] }, layout: { nodeIndex: [0], bounds: [[10, 20, 58, 16]] } }] },
    child: { strings: ['#text', 'synthetic-only'], documents: [{ scrollOffsetX: 0, scrollOffsetY: 0, nodes: { nodeName: [0], nodeValue: [1], backendNodeId: [30] }, layout: { nodeIndex: [0], bounds: [[5, 100, 120, 18]] } }] },
  };
  const box = ([x, y, w, h], inset = 0, scale = 1) => ({ width: w, height: h, border: [x, y, x + w * scale, y, x + w * scale, y + h * scale, x, y + h * scale],
    content: [x + inset, y + inset, x + (w - inset) * scale, y + inset, x + (w - inset) * scale, y + (h - inset) * scale, x + inset, y + (h - inset) * scale] });
  return { methods, async resolvePageTarget() { return { sessionId: 'session', connection: { async send(method, params, session) {
    methods.push([method, session]);
    if (method === 'Page.getFrameTree') return session === 'child' ? { frameTree: { frame: { id: 'F', parentId: unlinked ? 'elsewhere' : 'root', loaderId: 'L' } } } : { frameTree: { frame: { id: 'root', loaderId: 'document' } } };
    if (method === 'Target.getTargets') return { targetInfos: [{ type: 'iframe', targetId: 'F' }] };
    if (method === 'Target.attachToTarget') return { sessionId: 'child' };
    if (method === 'Target.detachFromTarget') return {};
    if (method === 'Page.createIsolatedWorld') return { executionContextId: 7 };
    if (method === 'Runtime.evaluate') return { result: { value: ['visible', 1, 800] } };
    if (method === 'Page.getLayoutMetrics') return { cssLayoutViewport: { clientHeight: 800, pageX: 0, pageY: 50 }, cssVisualViewport: { scale: 1 } };
    if (method === 'DOM.getFrameOwner') return { backendNodeId: 12 };
    if (method === 'DOM.getBoxModel' && params.backendNodeId === 11) return { model: box([100, 50, 310, 210], 5) };
    if (method === 'DOM.getBoxModel' && params.backendNodeId === 12) {
      if (hiddenOwner) throw new Error('Protocol error (DOM.getBoxModel): Could not compute box model.');
      return { model: transformed ? box([500, 50, 306, 206], 3, 0.5) : box([500, 50, 306, 206], 3) };
    }
    if (method === 'DOMSnapshot.captureSnapshot') return snapshots[session];
    throw new Error(method);
  } } }; } };
}
const content = { contentTarget: 'target' }, echo = { kind: 'browser', target_id: 'target', document_id: 'document', echo_only: true };
test('MP-11 echoes inside in-process and isolated frames are masked at their offsets; frames and images stay visible', async () => {
  const browser = framed();
  const regions = await locateBrowserRegions([echo], browser, ['synthetic-only'], content);
  // child document echo at owner content (105,55); isolated echo at (503,53); stray iframe fails closed.
  const sorted = list => list.map(r => r.join()).sort();
  assert.deepEqual(sorted(regions), sorted([[900, 50, 50, 50], [115, 75, 58, 16], [508, 153, 120, 18]]));
  assert(browser.methods.some(([method, session]) => method === 'DOMSnapshot.captureSnapshot' && session === 'child'), 'the isolated frame is scanned in its own session');
  assert(!regions.some(r => r[0] === 0 && r[1] === 350), 'images are not masked whole');
});
test('MP-11 a transformed isolated owner masks its whole border box; a hidden owner renders nothing', async () => {
  const transformed = await locateBrowserRegions([echo], framed({ transformed: true }), ['synthetic-only'], content);
  assert(transformed.some(r => r.join() === [500, 50, 153, 103].join()), JSON.stringify(transformed));
  assert(!transformed.some(r => r[0] === 508), JSON.stringify(transformed));
  const hidden = await locateBrowserRegions([echo], framed({ hiddenOwner: true }), ['synthetic-only'], content);
  assert(!hidden.some(r => r[0] >= 500 && r[0] < 900), JSON.stringify(hidden));
});
test('MP-11 an iframe whose document is not reachable is masked whole (fail closed)', async () => {
  const regions = await locateBrowserRegions([echo], framed({ unlinked: true }), ['synthetic-only'], content);
  assert(regions.some(r => r.join() === [500, 50, 306, 206].join()), JSON.stringify(regions));
});
