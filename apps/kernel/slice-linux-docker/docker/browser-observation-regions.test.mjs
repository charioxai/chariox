import assert from 'node:assert/strict';
import test from 'node:test';
import { locateBrowserRegions } from './browser-observation-regions.mjs';
import { handleBrowserControllerRequest } from './browser-controller.mjs';
import { RENDER_ORDER_STYLES } from './browser-controller-snapshot.mjs';
import { DEFAULT_RENDER_STYLE } from './browser-protection-fixture.mjs';

const target = { kind: 'browser', target_id: 'target', document_id: 'document', node_ref: 'backend:42' };
// Computed styles as Chromium returns them, only when the request asks for
// RENDER_ORDER_STYLES (text inherits its parent's; css overrides by node).
function styled(params, snapshot, css = {}) {
  const { strings, documents: [document] } = snapshot;
  if (params?.computedStyles?.join() !== RENDER_ORDER_STYLES.join()) return snapshot;
  const id = value => { const at = strings.indexOf(value); return at >= 0 ? at : strings.push(value) - 1; };
  document.layout.styles = (document.layout.nodeIndex ?? []).map(i => {
    const own = strings[document.nodes.nodeName[i]] === '#text' ? document.nodes.parentIndex?.[i] : i;
    return RENDER_ORDER_STYLES.map(p => id({ ...DEFAULT_RENDER_STYLE, ...css[own] }[p]));
  });
  return snapshot;
}
// generated: a DIV renders the value as ::before + nested SPAN text + ::after
// (listed before the SPAN, as Chromium does); no DOM value holds it.
// reversed: a row-reverse flex DIV shows SPAN 'only' after SPAN 'synthetic-'.
function fixture({ stale = false, hidden = false, replaced = false, scrollbar = 0, noEcho = false, windowHeight = 800, generated = false, reversed = false } = {}) {
  const methods = [];
  return { methods, async resolvePageTarget() { return { sessionId: 'session', connection: { async send(method, params) {
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
    if (method === 'DOMSnapshot.captureSnapshot' && generated) return styled(params, { strings: ['DIV', '::before', '::after', '#text', 'synthetic', 'only', '-', 'SPAN'], documents: [{
      nodes: { nodeName: [0, 1, 2, 7, 3], nodeType: [1, 1, 1, 1, 3], parentIndex: [-1, 0, 0, 0, 3], nodeValue: [-1, -1, -1, -1, 6] },
      layout: { nodeIndex: [0, 1, 2, 3, 4], bounds: [[400, 10, 200, 20], [400, 10, 80, 20], [500, 10, 40, 20], [480, 10, 20, 20], [480, 10, 20, 20]], text: [-1, 4, 5, -1, 6] } }] }, { 3: { display: 'inline' } });
    if (method === 'DOMSnapshot.captureSnapshot' && reversed) return styled(params, { strings: ['DIV', 'SPAN', '#text', 'only', 'synthetic-'], documents: [{
      nodes: { nodeName: [0, 1, 2, 1, 2], parentIndex: [-1, 0, 1, 0, 3], nodeValue: [-1, -1, 3, -1, 4] },
      layout: { nodeIndex: [0, 1, 2, 3, 4], bounds: [[400, 10, 200, 20], [470, 10, 30, 20], [470, 10, 30, 20], [400, 10, 70, 20], [400, 10, 70, 20]], text: [-1, -1, 3, -1, 4] } }] },
    { 0: { display: 'flex', 'flex-direction': 'row-reverse' } });
    if (method === 'DOMSnapshot.captureSnapshot') return styled(params, { strings: ['#text', 'synthetic-only', 'CANVAS'], documents: [{ nodes: { nodeName: [0, 2], nodeValue: [1], inputValue: { index: [0], value: [1] } }, layout: { nodeIndex: [0, 1], bounds: [[200, 10, 200, 20], [0, 300, 300, 100]], text: [1, -1] } }] });
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
test('MP-08/MP-10/MP-11 CSS-generated text across nested inline elements is masked with its container; unreadable layout text fails closed', async () => {
  const regions = await locateBrowserRegions([target], fixture({ stale: true, generated: true }), ['synthetic-only']);
  assert.deepEqual(regions, [[400, 110, 200, 20], [400, 110, 80, 20], [500, 110, 40, 20], [480, 110, 20, 20], [0, 0, 800, 100], [0, 776, 800, 24]]);
  const unreadable = fixture({ stale: true, generated: true }), resolve = unreadable.resolvePageTarget;
  unreadable.resolvePageTarget = async () => { const page = await resolve(), send = page.connection.send;
    page.connection.send = async (method, params) => { const result = await send(method, params); if (method === 'DOMSnapshot.captureSnapshot') delete result.documents[0].layout.text; return result; };
    return page; };
  await assert.rejects(locateBrowserRegions([target], unreadable, ['synthetic-only']), /layout text/);
});
test('MP-08/MP-10/MP-11 a container whose visual order differs from DOM order is masked whole', async () => {
  const regions = await locateBrowserRegions([target], fixture({ stale: true, reversed: true }), ['synthetic-only']);
  assert.deepEqual(regions, [[400, 110, 200, 20], [0, 0, 800, 100], [0, 776, 800, 24]]);
  // Masked whenever a value is registered, matching or not; never without one.
  assert.deepEqual(await locateBrowserRegions([target], fixture({ stale: true, reversed: true }), ['unrelated']), regions);
  assert.deepEqual(await locateBrowserRegions([target], fixture({ stale: true, reversed: true })), [[0, 0, 800, 100], [0, 776, 800, 24]]);
});

test('MP-08/MP-11 Browser-panel masks cover overflowing and zero-sized uncertain containers and their local clip fallback', async () => {
  for(const width of [1,0]){
    const browser=fixture({stale:true,reversed:true}),resolve=browser.resolvePageTarget;
    browser.resolvePageTarget=async()=>{
      const page=await resolve(),send=page.connection.send;
      page.connection.send=async(method,params)=>{
        const reply=await send(method,params);
        if(method==='DOMSnapshot.captureSnapshot'){
          reply.documents[0].layout.bounds[0]=[500,10,width,width?20:0];
          reply.documents[0].textBoxes={layoutIndex:[2],bounds:[[470,10,40,20]]};
        }
        return reply;
      };return page;
    };
    const regions=await locateBrowserRegions([target],browser,['synthetic-only']);
    for(const x of [401,475,505])assert(regions.some(([left,top,w,h])=>left<=x&&x<left+w&&top<=115&&115<top+h),`overflow x=${x} width=${width}`);
  }
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
        layout: { nodeIndex: [1, 2, 3, 4], bounds: [[100, 100, 310, 210], [500, 100, 306, 206], [900, 100, 50, 50], [0, 400, 300, 100]], text: [-1, -1, -1, -1] } },
      { scrollOffsetX: 0, scrollOffsetY: 0, nodes: { nodeName: [0], nodeType: [3], nodeValue: [1], backendNodeId: [20] }, layout: { nodeIndex: [0], bounds: [[10, 20, 58, 16]], text: [1] } }] },
    child: { strings: ['#text', 'synthetic-only'], documents: [{ scrollOffsetX: 0, scrollOffsetY: 0, nodes: { nodeName: [0], nodeType: [3], nodeValue: [1], backendNodeId: [30] }, layout: { nodeIndex: [0], bounds: [[5, 100, 120, 18]], text: [1] } }] },
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
    if (method === 'DOMSnapshot.captureSnapshot') { const copy = structuredClone(snapshots[session]); for (const document of copy.documents) styled(params, { strings: copy.strings, documents: [document] }); return copy; }
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
