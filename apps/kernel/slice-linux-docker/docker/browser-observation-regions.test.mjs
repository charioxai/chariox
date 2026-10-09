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
