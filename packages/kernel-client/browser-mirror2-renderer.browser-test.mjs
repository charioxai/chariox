// MP-08/MP-10: the DOM mirror v2 renderer in real Chrome (the bundled source
// renderer applying protocol 489 packets in a headless viewer; no kernel).
// usage: MIRROR_LAB_NODE_MODULES=<Cloud apps/web/node_modules> node --test browser-mirror2-renderer.browser-test.mjs
import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const tools = process.env.MIRROR_LAB_NODE_MODULES;
assert(tools && path.isAbsolute(tools), 'MP-10: MIRROR_LAB_NODE_MODULES required');
const { chromium } = createRequire(path.join(tools, 'x.js'))('playwright-core');
let root, server, browser, url;

before(async () => {
  root = await mkdtemp(path.join(tmpdir(), 'mirror2-renderer-'));
  execFileSync(path.join(tools, '.bin/esbuild'), [path.join(here, 'src/browser-mirror2.ts'), '--bundle', '--format=iife', '--global-name=Mirror2', '--target=es2022', `--outfile=${path.join(root, 'mirror2.js')}`], { stdio: 'inherit' });
  const bundle = await readFile(path.join(root, 'mirror2.js'));
  server = createServer((request, response) => {
    if (request.url === '/mirror2.js') { response.setHeader('Content-Type', 'text/javascript'); response.end(bundle); return; }
    response.setHeader('Content-Type', 'text/html'); response.end('<!doctype html><div id="c"></div><script src="/mirror2.js"></script>');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  url = `http://127.0.0.1:${server.address().port}/`;
  browser = await chromium.launch({ executablePath: process.env.MIRROR_LAB_CHROME ?? '/opt/google/chrome/chrome', headless: true });
});
after(async () => { await browser?.close(); await new Promise(resolve => server?.close(resolve)); if (root) await rm(root, { recursive: true, force: true }); });

// A viewer page with one renderer; `window.sent` collects the inputs it forwards.
async function viewer() {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  await page.goto(url);
  await page.evaluate(async () => { window.sent = []; window.r = new Mirror2.BrowserMirror2Renderer(document.getElementById('c'), async action => { window.sent.push(action); }); await window.r.ready(); });
  return page;
}
const header = { snapshot_complete: true, wire: 2, subscription_id: 's', tab_id: 't', generation: 1, document_id: 'd', ops: [], scroll: [0, 0], focused: null, selection: null, resources: [], tiles: [], css_width: 1280, css_height: 800, device_scale_factor: 1 };
const snapshot = nodes => ({ ...header, sequence: 1, base_sequence: null, reset: true, root: nodes[0].id, nodes });
const delta = (sequence, ops = [], extra = {}) => ({ ...header, sequence, base_sequence: sequence - 1, reset: false, ops, ...extra });
const apply = (page, packet) => page.evaluate(packet => window.r.apply(packet), packet);
const frames = () => new Promise(resolve => setTimeout(resolve, 50));

test('MP-10: review #941-1 kernel positions of other scrollers that arrive while the viewer scrolls apply once it settles (element, shadow, same-origin and cross-origin child documents); the viewer\'s later scroll wins', async () => {
  const page = await viewer();
  try {
    const tall = id => ({ id, parent: null, kind: 'element', tag: 'div', attrs: { style: 'height:3000px' } });
    const scroller = (id, parent) => ({ id, parent, kind: 'element', tag: 'div', attrs: { style: 'height:100px;overflow:auto' } });
    await apply(page, snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body', attrs: { style: 'height:5000px' } },
      scroller('n4', 'n3'), { ...tall('n5'), parent: 'n4' }, scroller('n6', 'n3'), { ...tall('n7'), parent: 'n6' },
      { id: 'n8', parent: 'n3', kind: 'element', tag: 'div' }, { id: 'n9', parent: 'n8', kind: 'shadow' }, scroller('n10', 'n9'), { ...tall('n11'), parent: 'n10' },
      { id: 'n12', parent: 'n3', kind: 'frame', tag: 'iframe', attrs: { style: 'width:200px;height:100px' } }, { id: 'n13', parent: 'n12', kind: 'document' }, { id: 'n14', parent: 'n13', kind: 'element', tag: 'html' }, { id: 'n15', parent: 'n14', kind: 'element', tag: 'body', attrs: { style: 'height:3000px' } },
      { id: 'n16', parent: 'n3', kind: 'frame', tag: 'iframe', attrs: { style: 'width:200px;height:100px' } }, { id: 'n17', parent: 'n16', kind: 'document' }, { id: 'n18', parent: 'n17', kind: 'element', tag: 'html' }, { id: 'n19', parent: 'n18', kind: 'element', tag: 'body', attrs: { style: 'height:3000px' } }]));
    // This viewer scrolls its window; another viewer's scrolls arrive as one-shot ops meanwhile.
    await page.evaluate(() => window.r.frame.contentWindow.scrollTo(0, 100)); await frames();
    await apply(page, delta(2, [{ op: 'scroll', id: 'n4', scroll: [0, 50] }, { op: 'scroll', id: 'n10', scroll: [0, 60] }, { op: 'scroll', id: 'n14', scroll: [0, 70] }, { op: 'scroll', id: 'n17', scroll: [0, 90] }]));
    await apply(page, delta(3, [{ op: 'scroll', id: 'n6', scroll: [0, 80] }]));
    await page.evaluate(() => { const d = window.r.frame.contentDocument; d.querySelectorAll('div')[2].scrollTop = 40; }); await frames(); // this viewer then scrolls n6 itself
    await new Promise(resolve => setTimeout(resolve, 600));
    await apply(page, delta(4, [], { scroll: [0, 100] })); // a heartbeat once settled
    const at = await page.evaluate(() => { const d = window.r.frame.contentDocument, divs = d.querySelectorAll('div'), shadow = divs[4].shadowRoot.firstChild; return { inner: divs[0].scrollTop, own: divs[2].scrollTop, shadow: shadow.scrollTop, child: d.querySelectorAll('iframe')[0].contentWindow.scrollY, foreign: d.querySelectorAll('iframe')[1].contentWindow.scrollY, window: window.r.frame.contentWindow.scrollY }; });
    assert.deepEqual(at, { inner: 50, own: 40, shadow: 60, child: 70, foreign: 90, window: 100 }, 'MP-10: deferred kernel positions apply when settled; the viewer\'s own later scroll is not overwritten');
  } finally { await page.close(); }
});

test('MP-10: review #320-1 an existing select keeps its recorded selection when its options change (regrouped into an optgroup, replaced beneath it)', async () => {
  const page = await viewer();
  try {
    const option = (id, parent, value) => ({ id, parent, kind: 'element', tag: 'option', attrs: { value } });
    await apply(page, snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
      { id: 'n4', parent: 'n3', kind: 'element', tag: 'select', form: { value: 'b', checked: false, selected_index: 1, selection_start: null, selection_end: null } }, option('n5', 'n4', 'a'), option('n6', 'n4', 'b')]));
    const state = () => page.evaluate(() => { const s = window.r.frame.contentDocument.querySelector('select'); return [s.selectedIndex, s.value]; });
    assert.deepEqual(await state(), [1, 'b']);
    // The page moves its options into an optgroup and sets the select back to b: the observer's form state is unchanged (no form op).
    await apply(page, delta(2, [{ op: 'children', id: 'n4', children: ['n7'], nodes: [{ id: 'n7', parent: 'n4', kind: 'element', tag: 'optgroup', attrs: { label: 'g' } }, option('n8', 'n7', 'a'), option('n9', 'n7', 'b')] }]));
    assert.deepEqual(await state(), [1, 'b'], 'MP-10: regrouped options keep the recorded selection');
    await apply(page, delta(3, [{ op: 'children', id: 'n7', children: ['n10', 'n11'], nodes: [option('n10', 'n7', 'a'), option('n11', 'n7', 'b')] }]));
    assert.deepEqual(await state(), [1, 'b'], 'MP-10: options replaced beneath the optgroup keep the recorded selection');
  } finally { await page.close(); }
});

test('MP-10: review #320-2 a frame document replaced by one without adopted stylesheets drops the old ones', async () => {
  const page = await viewer();
  try {
    const frameDoc = (base, adopted) => [{ id: `n${base}`, parent: 'n4', kind: 'document', ...(adopted ? { adopted } : {}) }, { id: `n${base + 1}`, parent: `n${base}`, kind: 'element', tag: 'html' }, { id: `n${base + 2}`, parent: `n${base + 1}`, kind: 'element', tag: 'body' }, { id: `n${base + 3}`, parent: `n${base + 2}`, kind: 'element', tag: 'p' }];
    await apply(page, snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
      { id: 'n4', parent: 'n3', kind: 'frame', tag: 'iframe' }, ...frameDoc(5, ['p{color:rgb(1, 2, 3)}'])]));
    const nested = () => page.evaluate(() => { const d = window.r.frame.contentDocument.querySelector('iframe').contentDocument; return [d.adoptedStyleSheets.length, getComputedStyle(d.querySelector('p')).color]; });
    assert.deepEqual(await nested(), [1, 'rgb(1, 2, 3)']);
    // The child navigates (the frame adapter rebuilds the frame's document through a children op).
    await apply(page, delta(2, [{ op: 'children', id: 'n4', children: ['n9'], nodes: frameDoc(9) }]));
    assert.deepEqual(await nested(), [0, 'rgb(0, 0, 0)'], 'MP-10: the new page is not styled by the old adopted sheets');
  } finally { await page.close(); }
});

test('MP-10: review #941-2 a multiple select shows every selected option (snapshot, a secondary option changing, replaced options)', async () => {
  const page = await viewer();
  try {
    const form = (checked, extra = {}) => ({ value: '', checked, selected_index: -1, selection_start: null, selection_end: null, ...extra });
    const option = (id, parent, text, checked) => [{ id, parent, kind: 'element', tag: 'option', form: form(checked, { value: text }) }, { id: `${id}0`, parent: id, kind: 'text', text }];
    await apply(page, snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
      { id: 'n4', parent: 'n3', kind: 'element', tag: 'select', attrs: { multiple: '' }, form: form(false, { value: 'b', selected_index: 1 }) }, ...option('n5', 'n4', 'a', false), ...option('n6', 'n4', 'b', true), ...option('n7', 'n4', 'c', true)]));
    const selected = () => page.evaluate(() => [...window.r.frame.contentDocument.querySelector('select').selectedOptions].map(o => o.textContent));
    assert.deepEqual(await selected(), ['b', 'c'], 'MP-10: both selected options are shown');
    await apply(page, delta(2, [{ op: 'form', id: 'n7', form: { checked: false } }, { op: 'form', id: 'n5', form: { checked: true } }]));
    assert.deepEqual(await selected(), ['a', 'b'], 'MP-10: a secondary option changes while the first stays selected');
    await apply(page, delta(3, [{ op: 'children', id: 'n4', children: ['n8', 'n9', 'n10'], nodes: [...option('n8', 'n4', 'x', false), ...option('n9', 'n4', 'y', true), ...option('n10', 'n4', 'z', true)] }]));
    assert.deepEqual(await selected(), ['y', 'z'], 'MP-10: replaced options keep their selectedness');
    await apply(page, delta(4, [{ op: 'form', id: 'n4', form: { value: 'y', selected_index: 1 } }]));
    assert.deepEqual(await selected(), ['y', 'z'], 'MP-10: the select\'s own form state does not collapse the set');
  } finally { await page.close(); }
});

test('MP-08: review #941-1 a text control\'s own range (select-all, mouse drag) is forwarded before the text or key that replaces it (input and textarea)', async () => {
  const page = await viewer();
  try {
    const form = (value, at) => ({ value, checked: false, selected_index: -1, selection_start: at, selection_end: at });
    const values = { n4: 'hello world', n5: 'one two\nthree' };
    await apply(page, snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
      { id: 'n4', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'font:16px monospace;width:300px;display:block' }, form: form(values.n4, 11) },
      { id: 'n5', parent: 'n3', kind: 'element', tag: 'textarea', attrs: { style: 'font:16px monospace;width:300px;height:80px;display:block' }, form: form(values.n5, 13) }]));
    const sent = () => page.evaluate(() => window.sent.splice(0).filter(action => action.kind !== 'scroll_to'));
    const range = id => page.evaluate(id => { const e = window.r.frame.contentDocument.querySelectorAll('input,textarea')[id === 'n4' ? 0 : 1]; return [e.selectionStart, e.selectionEnd, e.selectionDirection]; }, id);
    // Real pointer events (CDP): a drag with the button held, right to left.
    const cdp = await page.context().newCDPSession(page);
    const drag = async id => {
      const box = await page.evaluate(id => { const f = window.r.frame.getBoundingClientRect(), t = window.r.frame.contentDocument.querySelectorAll('input,textarea')[id === 'n4' ? 0 : 1].getBoundingClientRect(); return { x: f.x + t.x, y: f.y + t.y + 9 }; }, id);
      const mouse = (type, x, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x: box.x + x, y: box.y, button: 'left', buttons, clickCount: 1 });
      await mouse('mousePressed', 60, 1); for (const x of [50, 40, 30, 22]) await mouse('mouseMoved', x, 1); await mouse('mouseReleased', 22, 0); await frames();
    };
    let sequence = 1;
    for (const id of ['n4', 'n5']) for (const gesture of ['select-all', 'drag']) {
      // The kernel focuses the control with its caret at the end.
      await apply(page, delta(++sequence, [{ op: 'form', id, form: { selection_start: values[id].length, selection_end: values[id].length } }], { focused: id })); await sent();
      if (gesture === 'select-all') await page.keyboard.press('Control+A'); else await drag(id);
      await frames();
      const [start, end, direction] = await range(id);
      assert(start < end && direction === (gesture === 'drag' ? 'backward' : 'forward'), `MP-08: the viewer selected a range of ${id} (${gesture}): ${start},${end},${direction}`);
      if (id === 'n4') await page.keyboard.type('x'); else await page.keyboard.press('Backspace');
      await frames();
      const [anchor, focus] = direction === 'backward' ? [end, start] : [start, end];
      assert.deepEqual(await sent(), [{ kind: 'selection', anchor_id: id, anchor_offset: anchor, focus_id: id, focus_offset: focus }, id === 'n4' ? { kind: 'key', key: 'x' } : { kind: 'key', key: 'Backspace' }], `MP-08: the ${gesture} range of ${id} reaches the kernel first`);
    }
  } finally { await page.close(); }
});

test('MP-10: review #320-1 the viewer clears the kernel\'s selection once the kernel clears it (top-level and nested documents); a focused text control keeps its own range', async () => {
  const page = await viewer();
  try {
    await apply(page, snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
      { id: 'n4', parent: 'n3', kind: 'element', tag: 'p' }, { id: 'n5', parent: 'n4', kind: 'text', text: 'top level text' },
      { id: 'n6', parent: 'n3', kind: 'element', tag: 'input', form: { value: 'field', checked: false, selected_index: -1, selection_start: 1, selection_end: 4 } },
      { id: 'n7', parent: 'n3', kind: 'frame', tag: 'iframe' }, { id: 'n8', parent: 'n7', kind: 'document' }, { id: 'n9', parent: 'n8', kind: 'element', tag: 'html' }, { id: 'n10', parent: 'n9', kind: 'element', tag: 'body' },
      { id: 'n11', parent: 'n10', kind: 'element', tag: 'p' }, { id: 'n12', parent: 'n11', kind: 'text', text: 'nested text' }]));
    const shown = () => page.evaluate(() => { const top = window.r.frame.contentDocument, nested = top.querySelector('iframe').contentDocument, field = top.querySelector('input'); return [String(top.getSelection()), String(nested.getSelection()), [field.selectionStart, field.selectionEnd]]; });
    const range = (anchor, from, to) => ({ anchor_id: anchor, anchor_offset: from, focus_id: anchor, focus_offset: to });
    await apply(page, delta(2, [], { selection: range('n5', 0, 3) }));
    assert.deepEqual((await shown()).slice(0, 2), ['top', ''], 'MP-10: the kernel selection is shown');
    await apply(page, delta(3, [], { selection: null }));
    assert.deepEqual((await shown()).slice(0, 2), ['', ''], 'MP-10: a kernel removeAllRanges clears the top-level range');
    await apply(page, delta(4, [], { selection: range('n12', 0, 6) }));
    assert.deepEqual((await shown()).slice(0, 2), ['', 'nested'], 'MP-10: the kernel selection inside a frame document is shown');
    await apply(page, delta(5, [], { selection: null }));
    assert.deepEqual((await shown()).slice(0, 2), ['', ''], 'MP-10: a cleared kernel selection clears the nested range');
    // The kernel's focus moves into the field (its own range 1..4) as its document selection clears.
    await apply(page, delta(6, [], { selection: range('n5', 4, 9) }));
    await apply(page, delta(7, [], { selection: null, focused: 'n6' }));
    assert.deepEqual((await shown())[2], [1, 4], 'MP-10: clearing does not drop a focused text control\'s own range');
  } finally { await page.close(); }
});

test('MP-08: review #320-2 modified deletions in a focused text control are forwarded as the kernel\'s editing keys (word, line, cut; input and textarea)', async () => {
  const page = await viewer();
  try {
    const form = (value, at) => ({ value, checked: false, selected_index: -1, selection_start: at, selection_end: at });
    await apply(page, { ...snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
      { id: 'n4', parent: 'n3', kind: 'element', tag: 'input', form: form('hello world', 11) }, { id: 'n5', parent: 'n3', kind: 'element', tag: 'textarea', form: form('one two\nthree', 4) }]), focused: 'n4' });
    const sent = () => page.evaluate(() => window.sent.splice(0).filter(action => action.kind !== 'scroll_to'));
    const key = key => ({ kind: 'key', key });
    await page.keyboard.press('Control+Backspace'); await frames();
    assert.deepEqual(await sent(), [key('Ctrl+Backspace')], 'MP-08: word deletion backward');
    await apply(page, delta(2, [], { focused: 'n5' })); await sent();
    await page.keyboard.press('Control+Delete'); await frames();
    assert.deepEqual(await sent(), [key('Ctrl+Delete')], 'MP-08: word deletion forward');
    // macOS line deletions (Cmd+Backspace, Ctrl+K) arrive as these input types.
    for (const [inputType, keys] of [['deleteSoftLineBackward', ['Shift+Home', 'Backspace']], ['deleteHardLineForward', ['Shift+End', 'Delete']]]) {
      await page.evaluate(inputType => window.r.frame.contentDocument.querySelector('textarea').dispatchEvent(new InputEvent('beforeinput', { inputType, bubbles: true, cancelable: true, composed: true })), inputType); await frames();
      assert.deepEqual(await sent(), keys.map(key), `MP-08: ${inputType}`);
    }
    // A range (select-all) is deleted as a whole by any modified deletion, and by cut.
    for (const shortcut of ['Control+Backspace', 'Control+X']) {
      await page.keyboard.press('Control+A'); await frames(); await sent();
      await page.keyboard.press(shortcut); await frames();
      assert.deepEqual(await sent(), [{ kind: 'selection', anchor_id: 'n5', anchor_offset: 0, focus_id: 'n5', focus_offset: 13 }, key('Delete')], `MP-08: ${shortcut} on a range`);
      await apply(page, delta(Number(await page.evaluate(() => window.r.sequence)) + 1, [{ op: 'form', id: 'n5', form: { selection_start: 4, selection_end: 4 } }], { focused: 'n5' })); await sent();
    }
  } finally { await page.close(); }
});

test('MP-08: review #941-1 an IME composition keeps the kernel\'s insertion point (no viewer-local range is sent while composing)', async () => {
  const page = await viewer();
  try {
    await apply(page, { ...snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
      { id: 'n4', parent: 'n3', kind: 'element', tag: 'input', form: { value: 'hello world', checked: false, selected_index: -1, selection_start: 5, selection_end: 5 } }]), focused: 'n4' });
    const cdp = await page.context().newCDPSession(page);
    // A real composition: the viewer's own value shows the composing text, then the IME commits it.
    await cdp.send('Input.imeSetComposition', { text: 'にほ', selectionStart: 2, selectionEnd: 2 }); await frames();
    await cdp.send('Input.imeSetComposition', { text: '日本', selectionStart: 2, selectionEnd: 2 }); await frames();
    await cdp.send('Input.insertText', { text: '日本' }); await frames();
    await page.keyboard.press('ArrowRight'); await frames();
    assert.deepEqual(await page.evaluate(() => window.sent.filter(action => action.kind !== 'scroll_to')), [{ kind: 'text', text: '日本' }, { kind: 'key', key: 'ArrowRight' }], 'MP-08: the composed text inserts at the kernel caret');
    // A selected range (select-all) that a composition replaces is synced as the composition starts.
    await page.evaluate(() => window.sent.splice(0)); await page.keyboard.press('Control+A'); await frames();
    await cdp.send('Input.imeSetComposition', { text: 'か', selectionStart: 1, selectionEnd: 1 }); await frames();
    await cdp.send('Input.insertText', { text: 'か' }); await frames();
    assert.deepEqual(await page.evaluate(() => window.sent.filter(action => action.kind !== 'scroll_to')), [{ kind: 'selection', anchor_id: 'n4', anchor_offset: 0, focus_id: 'n4', focus_offset: 13 }, { kind: 'text', text: 'か' }], 'MP-08: the composition replaces the synced range');
  } finally { await page.close(); }
});

test('MP-08: review #941-1 keys pressed while composing stay with the viewer\'s IME (candidate navigation, editing, Enter confirmation); only the committed text reaches the kernel', async () => {
  const page = await viewer();
  try {
    await apply(page, { ...snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
      { id: 'n4', parent: 'n3', kind: 'element', tag: 'form' }, { id: 'n5', parent: 'n4', kind: 'element', tag: 'input', form: { value: 'hello ', checked: false, selected_index: -1, selection_start: 6, selection_end: 6 } }]), focused: 'n5' });
    // After the renderer's capture listener: what the page-side IME would see.
    await page.evaluate(() => { window.keys = []; window.r.frame.contentDocument.addEventListener('keydown', e => window.keys.push([e.key, e.isComposing, e.defaultPrevented])); });
    const cdp = await page.context().newCDPSession(page);
    // As an IME reports them: the key with keyCode 229 while it updates its own composition.
    const key = key => cdp.send('Input.dispatchKeyEvent', { type: 'rawKeyDown', key, code: key, windowsVirtualKeyCode: 229 }).then(() => cdp.send('Input.dispatchKeyEvent', { type: 'keyUp', key, code: key, windowsVirtualKeyCode: 229 }));
    const compose = text => cdp.send('Input.imeSetComposition', { text, selectionStart: text.length, selectionEnd: text.length });
    await compose('にほんg'); await frames();
    for (const [name, text] of [['Backspace', 'にほん'], ['ArrowDown', '日本'], ['ArrowUp', '二本'], ['ArrowDown', '日本'], ['Escape', 'にほん'], ['ArrowDown', '日本'], ['Enter', null]]) { await key(name); if (text) await compose(text); await frames(); }
    await cdp.send('Input.insertText', { text: '日本' }); await frames();
    const keys = await page.evaluate(() => window.keys);
    assert(keys.length === 7 && keys.every(([, composing]) => composing), `MP-08: the keys arrived while composing: ${JSON.stringify(keys)}`);
    assert.deepEqual(keys.filter(([, , prevented]) => prevented), [], 'MP-08: no composing key is taken from the viewer\'s IME');
    assert.deepEqual(await page.evaluate(() => window.sent.filter(action => action.kind !== 'scroll_to')), [{ kind: 'text', text: '日本' }], 'MP-08: only the committed text reaches the kernel (no Enter submits the source form)');
  } finally { await page.close(); }
});

test('MP-08: review #320-1 a drag that selects in a control the kernel has not focused sends its range when the drag ends (the kernel focuses it), before any edit', async () => {
  const page = await viewer();
  try {
    const form = (value, at) => ({ value, checked: false, selected_index: -1, selection_start: at, selection_end: at });
    await apply(page, { ...snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
      { id: 'n4', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'font:16px monospace;width:300px;display:block' }, form: form('first field', 11) },
      { id: 'n5', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'font:16px monospace;width:300px;display:block' }, form: form('second field', 12) }]), focused: 'n4' });
    const cdp = await page.context().newCDPSession(page);
    const box = await page.evaluate(() => { const f = window.r.frame.getBoundingClientRect(), t = window.r.frame.contentDocument.querySelectorAll('input')[1].getBoundingClientRect(); return { x: f.x + t.x, y: f.y + t.y + 9 }; });
    const mouse = (type, x, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x: box.x + x, y: box.y, button: 'left', buttons, clickCount: 1 });
    await mouse('mousePressed', 70, 1); for (const x of [80, 95, 110, 125]) await mouse('mouseMoved', x, 1); await mouse('mouseReleased', 125, 0); await frames();
    const [start, end] = await page.evaluate(() => { const e = window.r.frame.contentDocument.activeElement; return [e.selectionStart, e.selectionEnd]; });
    assert(start < end, 'MP-08: the drag selected inside the second control');
    const sent = () => page.evaluate(() => window.sent.splice(0).filter(action => action.kind !== 'scroll_to'));
    assert.deepEqual(await sent(), [{ kind: 'focus', node_id: 'n5' }, { kind: 'selection', anchor_id: 'n5', anchor_offset: start, focus_id: 'n5', focus_offset: end }], 'MP-08: the focus (protocol 492) and the range go when the drag ends');
    await page.keyboard.type('x'); await frames();
    assert.deepEqual(await sent(), [{ kind: 'key', key: 'x' }], 'MP-08: the edit follows without resending the range');
  } finally { await page.close(); }
});

// Protocol 492 (audit 2026-10-10 §4.2): focus transitions and committed control state, not keystrokes.
const page2 = (extra = []) => [{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body', attrs: { style: 'height:600px' } }, ...extra];
const textForm = (value, at) => ({ value, checked: false, selected_index: -1, selection_start: at, selection_end: at });
test('MP-08: protocol 492 a viewer select, date or range commits its own pick as `set` (typeahead, arrows, typed date); a click on it is not forwarded and opens no kernel popup', async () => {
  const page = await viewer();
  try {
    const option = (id, value) => [{ id, parent: 'n4', kind: 'element', tag: 'option', attrs: { value } }, { id: `${id}0`, parent: id, kind: 'text', text: value }];
    await apply(page, { ...snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'select', form: { value: 'Apple', checked: false, selected_index: 0, selection_start: null, selection_end: null } }, ...option('n5', 'Apple'), ...option('n6', 'Banana'), ...option('n7', 'Cherry'),
      { id: 'n8', parent: 'n3', kind: 'element', tag: 'input', attrs: { type: 'date' }, form: textForm('', null) }, { id: 'n9', parent: 'n3', kind: 'element', tag: 'input', attrs: { type: 'range', min: '0', max: '10' }, form: textForm('5', null) }])), focused: 'n4' });
    const sent = () => page.evaluate(() => window.sent.splice(0).filter(action => action.kind !== 'scroll_to'));
    await page.keyboard.press('b'); await page.keyboard.press('ArrowDown'); await frames();
    assert.deepEqual(await sent(), [{ kind: 'set', node_id: 'n4', selected_index: 1 }, { kind: 'set', node_id: 'n4', selected_index: 2 }], 'MP-08: the typeahead picks reach the kernel as committed state');
    const box = await page.evaluate(() => { const f = window.r.frame.getBoundingClientRect(), t = window.r.frame.contentDocument.querySelector('select').getBoundingClientRect(); return [f.x + t.x + 5, f.y + t.y + 5]; });
    await page.mouse.click(box[0], box[1]); await page.keyboard.press('Escape'); await frames();
    assert.deepEqual((await sent()).filter(action => action.kind === 'click'), [], 'MP-08: no click on the select reaches the kernel (its popup stays closed)');
    await apply(page, delta(2, [], { focused: 'n8' })); await sent();
    await page.keyboard.type('10102026'); await frames();
    assert.deepEqual((await sent()).at(-1), { kind: 'set', node_id: 'n8', value: '2026-10-10' }, 'MP-08: the typed date commits as one value');
    await apply(page, delta(3, [], { focused: 'n9' })); await sent();
    await page.keyboard.press('ArrowRight'); await frames();
    assert.deepEqual(await sent(), [{ kind: 'set', node_id: 'n9', value: '6' }], 'MP-08: the range step commits locally, no arrow key forwarded');
  } finally { await page.close(); }
});

test('MP-08: protocol 492 a viewer focus change reaches the kernel as `focus`, also when a drag repeats a cached range (#320 R8-1); kernel-applied focus sends nothing', async () => {
  const page = await viewer();
  try {
    await apply(page, { ...snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'font:16px monospace;width:300px;display:block' }, form: textForm('first field', 11) },
      { id: 'n5', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'font:16px monospace;width:300px;display:block' }, form: textForm('second field', 12) }])), focused: 'n4' });
    const sent = () => page.evaluate(() => window.sent.splice(0).filter(action => action.kind !== 'scroll_to'));
    assert.deepEqual(await sent(), [], 'MP-08: the kernel\'s own focus is not echoed');
    const cdp = await page.context().newCDPSession(page);
    const box = await page.evaluate(() => { const f = window.r.frame.getBoundingClientRect(), t = window.r.frame.contentDocument.querySelectorAll('input')[1].getBoundingClientRect(); return { x: f.x + t.x, y: f.y + t.y + 9 }; });
    const mouse = (type, x, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x: box.x + x, y: box.y, button: 'left', buttons, clickCount: 1 });
    const drag = async () => { await mouse('mousePressed', 70, 1); for (const x of [80, 95, 110, 125]) await mouse('mouseMoved', x, 1); await mouse('mouseReleased', 125, 0); await frames(); };
    await drag();
    const first = await sent();
    assert.deepEqual(first[0], { kind: 'focus', node_id: 'n5' }, 'MP-08: the viewer\'s focus moved to the second control');
    // The kernel's focus goes back to the first control (after the packets in flight at the focus reply,
    // which could predate it); the same drag repeats the same (cached) range.
    for (let sequence = 2; sequence <= 7; sequence++) await apply(page, delta(sequence, [], { focused: 'n4' }));
    assert.deepEqual(await sent(), []);
    await drag(); await page.keyboard.type('x'); await frames();
    const second = await sent();
    assert.deepEqual([second[0], second.at(-1)], [{ kind: 'focus', node_id: 'n5' }, { kind: 'key', key: 'x' }], `MP-08: the text goes where the viewer typed: ${JSON.stringify(second)}`);
  } finally { await page.close(); }
});

test('MP-08: protocol 492 the kernel\'s blur (focused: null) blurs the viewer\'s control (#941 R8-1); the viewer leaving a control to nothing sends `blur`, the viewer window losing focus does not', async () => {
  const page = await viewer();
  try {
    await apply(page, { ...snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'input', form: textForm('query', 5) }, { id: 'n5', parent: 'n3', kind: 'element', tag: 'p' }, { id: 'n6', parent: 'n5', kind: 'text', text: 'plain text' }])), focused: 'n4' });
    const active = () => page.evaluate(() => window.r.frame.contentDocument.activeElement.localName);
    const sent = () => page.evaluate(() => window.sent.splice(0).filter(action => action.kind !== 'scroll_to'));
    assert.equal(await active(), 'input');
    await apply(page, delta(2, [], { focused: null }));
    assert.equal(await active(), 'body', 'MP-08: the kernel blurred its field; the viewer\'s caret goes too');
    assert.deepEqual(await sent(), [], 'MP-08: the kernel-driven blur is not echoed');
    await apply(page, delta(3, [], { focused: 'n4' }));
    await page.evaluate(() => { const outside = document.createElement('input'); outside.id = 'outside'; document.body.append(outside); outside.focus(); }); await frames();
    assert.deepEqual(await sent(), [], 'MP-08: the viewer window losing focus keeps the kernel\'s focus');
    await apply(page, delta(4, [], { focused: 'n4' }));
    assert.equal(await page.evaluate(() => document.activeElement.id), 'outside', 'MP-08: a later packet does not pull the viewer\'s focus back into the mirror');
    await page.evaluate(() => window.r.frame.contentDocument.querySelector('input').focus()); await frames(); await sent();
    const box = await page.evaluate(() => { const f = window.r.frame.getBoundingClientRect(), t = window.r.frame.contentDocument.querySelector('p').getBoundingClientRect(); return [f.x + t.x + 3, f.y + t.y + 3]; });
    await page.mouse.click(box[0], box[1]); await frames();
    assert.deepEqual((await sent()).filter(action => action.kind === 'blur'), [{ kind: 'blur', node_id: 'n4' }], 'MP-08: clicking plain text leaves the control in the kernel too');
  } finally { await page.close(); }
});

test('MP-10: protocol 492 the viewer takes the kernel\'s UA state (open popover, modal dialog with backdrop, indeterminate checkbox, custom element definitions) and marks the fragment\'s target', async () => {
  const page = await viewer();
  try {
    const css = '[popover]:popover-open{color:rgb(0, 0, 255)} dialog:modal{color:rgb(0, 128, 0)} input:indeterminate{width:31px} x-late:not(:not([chariox-undefined])),x-never:not(:not([chariox-undefined])){display:none} [chariox-target]{color:rgb(255, 0, 0)}';
    await apply(page, snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'style', css }, { id: 'n5', parent: 'n3', kind: 'element', tag: 'div', attrs: { popover: '', id: 'p' }, state: { popover_open: true } }, { id: 'n6', parent: 'n5', kind: 'text', text: 'pop' },
      { id: 'n7', parent: 'n3', kind: 'element', tag: 'dialog', attrs: { open: '' }, state: { modal: true } }, { id: 'n8', parent: 'n7', kind: 'text', text: 'dialog' },
      { id: 'n9', parent: 'n3', kind: 'element', tag: 'input', attrs: { type: 'checkbox' }, state: { indeterminate: true }, form: textForm('on', null) },
      { id: 'n10', parent: 'n3', kind: 'element', tag: 'x-ready' }, { id: 'n11', parent: 'n3', kind: 'element', tag: 'x-late', state: { defined: false } }, { id: 'n12', parent: 'n11', kind: 'text', text: 'late' }, { id: 'n13', parent: 'n3', kind: 'element', tag: 'x-never', state: { defined: false } },
      { id: 'n14', parent: 'n3', kind: 'element', tag: 'p', attrs: { id: 'sec' } }, { id: 'n15', parent: 'n14', kind: 'text', text: 'section' }])));
    const seen = () => page.evaluate(() => { const d = window.r.frame.contentDocument, $ = s => d.querySelector(s); return { popover: $('#p').matches(':popover-open'), modal: $('dialog').matches(':modal'), backdrop: $('dialog').matches(':modal') ? getComputedStyle($('dialog'), '::backdrop').position : null, indeterminate: $('input').matches(':indeterminate'), ready: $('x-ready').matches(':not([chariox-undefined])'), late: getComputedStyle($('x-late')).display, never: $('x-never').matches(':not([chariox-undefined])'), target: getComputedStyle($('#sec')).color }; });
    assert.deepEqual(await seen(), { popover: true, modal: true, backdrop: 'fixed', indeterminate: true, ready: true, late: 'none', never: false, target: 'rgb(0, 0, 0)' });
    await apply(page, delta(2, [{ op: 'state', id: 'n5', state: { popover_open: false } }, { op: 'state', id: 'n7', state: { modal: false } }, { op: 'state', id: 'n9', state: { indeterminate: false } }, { op: 'state', id: 'n11', state: { defined: true } }], { fragment: 'sec' }));
    assert.deepEqual(await seen(), { popover: false, modal: false, backdrop: null, indeterminate: false, ready: true, late: 'inline', never: false, target: 'rgb(255, 0, 0)' });
    assert.equal(await page.evaluate(() => window.r.frame.contentDocument.querySelector('dialog').open), true, 'MP-10: a dialog that leaves modal stays open in flow (its open attribute)');
    await apply(page, delta(3, [], { fragment: null }));
    assert.equal((await seen()).target, 'rgb(0, 0, 0)', 'MP-10: a cleared fragment unmarks the target');
  } finally { await page.close(); }
});

test('MP-10: denylisted attributes render like the source: named slots, constraint-validation pseudo-classes, image/month/file inputs (file never opens a chooser)', async () => {
  const page = await viewer();
  try {
    await apply(page, snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'style', css: 'input:invalid{outline:2px solid rgb(255, 0, 0)} input:valid{outline:0px none rgb(0, 0, 0)}' },
      { id: 'n5', parent: 'n3', kind: 'element', tag: 'x-card' }, { id: 'n6', parent: 'n5', kind: 'element', tag: 'h2', attrs: { slot: 'title' } }, { id: 'n7', parent: 'n6', kind: 'text', text: 'Title' }, { id: 'n8', parent: 'n5', kind: 'element', tag: 'p' }, { id: 'n9', parent: 'n8', kind: 'text', text: 'body' },
      { id: 'n10', parent: 'n5', kind: 'shadow' }, { id: 'n11', parent: 'n10', kind: 'element', tag: 'slot' }, { id: 'n12', parent: 'n10', kind: 'element', tag: 'slot', attrs: { name: 'title' } },
      { id: 'n13', parent: 'n3', kind: 'element', tag: 'input', attrs: { required: '', pattern: '[a-z]+' }, form: textForm('', 0) },
      { id: 'n14', parent: 'n3', kind: 'element', tag: 'input', attrs: { type: 'image', alt: 'Go' } }, { id: 'n15', parent: 'n3', kind: 'element', tag: 'input', attrs: { type: 'month' }, form: textForm('2026-10', null) },
      { id: 'n16', parent: 'n3', kind: 'element', tag: 'input', attrs: { type: 'file' }, form: textForm('', null) }])));
    const seen = () => page.evaluate(() => { const d = window.r.frame.contentDocument, card = d.querySelector('x-card'), slots = card.shadowRoot.querySelectorAll('slot'), inputs = d.querySelectorAll('input'); return { first: slots[1].assignedElements().map(e => e.localName), rest: slots[0].assignedElements().map(e => e.localName), invalid: getComputedStyle(inputs[0]).outlineColor, image: inputs[1].type, month: [inputs[2].type, inputs[2].value], file: inputs[3].type }; });
    assert.deepEqual(await seen(), { first: ['h2'], rest: ['p'], invalid: 'rgb(255, 0, 0)', image: 'image', month: ['month', '2026-10'], file: 'file' });
    await apply(page, delta(2, [{ op: 'form', id: 'n13', form: { value: 'abc', selection_start: 3, selection_end: 3 } }]));
    assert.equal((await seen()).invalid, 'rgb(0, 0, 0)', 'MP-10: the field becomes valid with the kernel\'s value');
    const box = await page.evaluate(() => { const f = window.r.frame.getBoundingClientRect(), t = window.r.frame.contentDocument.querySelectorAll('input')[3].getBoundingClientRect(); return [f.x + t.x + 5, f.y + t.y + 5]; });
    const chooser = page.waitForEvent('filechooser', { timeout: 700 }).then(() => true, () => false);
    await page.mouse.click(box[0], box[1]);
    assert.equal(await chooser, false, 'MP-10: a file input never opens the viewer\'s chooser');
  } finally { await page.close(); }
});

test('MP-08: protocol 492 a click on a focusable control is forwarded alone (the kernel\'s click focuses it): no focus round trip ahead of the click', async () => {
  const page = await viewer();
  try {
    await apply(page, snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'width:200px' }, form: textForm('one', 3) }, { id: 'n5', parent: 'n3', kind: 'element', tag: 'button', attrs: { style: 'width:100px;height:30px' } }, { id: 'n6', parent: 'n5', kind: 'text', text: 'go' }])));
    const sent = () => page.evaluate(() => window.sent.splice(0).filter(action => action.kind !== 'scroll_to').map(action => action.kind + ':' + action.node_id));
    for (const selector of ['input', 'button']) {
      const box = await page.evaluate(selector => { const f = window.r.frame.getBoundingClientRect(), t = window.r.frame.contentDocument.querySelector(selector).getBoundingClientRect(); return [f.x + t.x + 5, f.y + t.y + 5]; }, selector);
      await page.mouse.click(box[0], box[1]); await frames(); await frames();
      assert.deepEqual(await sent(), [`click:${selector === 'input' ? 'n4' : 'n5'}`], `MP-08: ${selector}`);
    }
  } finally { await page.close(); }
});

// MP-08/MP-10: the press dirties the viewer's style and layout (focus, :active, or the page itself); the click's
// offset comes from the target's box read at the press, so the click handler forces no layout ahead of the input.
test('MP-08/MP-10: a click is addressed by its target\'s box at the press, not re-measured at the click', async () => {
  const page = await viewer();
  try {
    await apply(page, snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'button', attrs: { style: 'display:block;width:200px;height:40px' } }, { id: 'n5', parent: 'n4', kind: 'text', text: 'go' }])));
    // The press moves the button 5 px down (still under the pointer, so the click targets it).
    await page.evaluate(() => window.r.frame.contentWindow.addEventListener('mousedown', () => { window.r.frame.contentDocument.querySelector('button').style.cssText += ';position:relative;top:5px'; }));
    const box = await page.evaluate(() => { const f = window.r.frame.getBoundingClientRect(), t = window.r.frame.contentDocument.querySelector('button').getBoundingClientRect(); return [f.x + t.x + 30, f.y + t.y + 10]; });
    await page.mouse.click(box[0], box[1]); await frames(); await frames();
    const click = (await page.evaluate(() => window.sent)).find(action => action.kind === 'click');
    assert.deepEqual([click?.node_id, Math.round(click?.x), Math.round(click?.y)], ['n4', 30, 10], 'MP-08: the offset within the pressed box');
  } finally { await page.close(); }
});

test('MP-08: a kernel packet that still names the old focus while the viewer\'s pointer is pressed in another control keeps the viewer\'s focus (round 10 hosted dragrepeat)', async () => {
  const page = await viewer();
  try {
    await apply(page, { ...snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'font:16px monospace;width:300px;display:block' }, form: textForm('first field', 11) },
      { id: 'n5', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'font:16px monospace;width:300px;display:block' }, form: textForm('second field', 12) }])), focused: 'n4' });
    const sent = () => page.evaluate(() => window.sent.splice(0).filter(action => action.kind !== 'scroll_to'));
    const cdp = await page.context().newCDPSession(page);
    const box = await page.evaluate(() => { const f = window.r.frame.getBoundingClientRect(), t = window.r.frame.contentDocument.querySelectorAll('input')[1].getBoundingClientRect(); return { x: f.x + t.x, y: f.y + t.y + 9 }; });
    const mouse = (type, x, buttons) => cdp.send('Input.dispatchMouseEvent', { type, x: box.x + x, y: box.y, button: 'left', buttons, clickCount: 1 });
    await mouse('mousePressed', 70, 1); await mouse('mouseMoved', 90, 1); await frames();
    // The page's own change (A's suggestions) arrives mid-drag; the kernel has not heard of the viewer's focus yet.
    await apply(page, delta(2, [], { focused: 'n4' }));
    await mouse('mouseMoved', 125, 1); await mouse('mouseReleased', 125, 0); await frames();
    const active = await page.evaluate(() => window.r.frame.contentDocument.activeElement === window.r.frame.contentDocument.querySelectorAll('input')[1]);
    assert(active, 'MP-08: the viewer keeps its pressed control focused');
    await page.keyboard.type('x'); await frames();
    const actions = await sent();
    assert.deepEqual([actions[0], actions.at(-1)], [{ kind: 'focus', node_id: 'n5' }, { kind: 'key', key: 'x' }], `MP-08: focus then text for the pressed control: ${JSON.stringify(actions)}`);
  } finally { await page.close(); }
});

test('MP-08: a packet drained before the kernel applied the viewer\'s focus (it arrives after the focus reply) does not refocus the old control; a later packet does', async () => {
  const page = await viewer();
  try {
    await apply(page, { ...snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'font:16px monospace;width:300px;display:block' }, form: textForm('first field', 11) },
      { id: 'n5', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'font:16px monospace;width:300px;display:block' }, form: textForm('second field', 12) },
      { id: 'n6', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'font:16px monospace;width:300px;display:block' }, form: textForm('third field', 11) }])), focused: 'n4' });
    const active = () => page.evaluate(() => { const d = window.r.frame.contentDocument; return [...d.querySelectorAll('input')].indexOf(d.activeElement); });
    const box = await page.evaluate(() => { const f = window.r.frame.getBoundingClientRect(), t = window.r.frame.contentDocument.querySelectorAll('input')[1].getBoundingClientRect(); return { x: f.x + t.x + 40, y: f.y + t.y + 9 }; });
    await page.mouse.click(box.x, box.y); await frames();
    assert.deepEqual((await page.evaluate(() => window.sent.splice(0))).map(a => a.kind), ['click'], 'MP-08: the click carries the focus');
    // The reply came back; a packet the kernel drained before applying the click still names the old focus.
    await apply(page, delta(2, [], { focused: 'n4' })); await frames();
    assert.equal(await active(), 1, 'MP-08: the viewer keeps the control it focused');
    await apply(page, delta(3, [], { focused: 'n5' })); await frames();
    assert.equal(await active(), 1);
    // Past the credit window the kernel's focus is authoritative again (a page script moved it back).
    for (let sequence = 4; sequence <= 8; sequence++) await apply(page, delta(sequence, [], { focused: 'n4' }));
    await frames();
    assert.equal(await active(), 0, 'MP-08: a later kernel focus change still reaches the viewer');
    // A click whose page script focuses a third control: that focus applies at once (it is not the old one).
    await page.mouse.click(box.x, box.y); await frames(); await page.evaluate(() => window.sent.splice(0));
    await apply(page, delta(9, [], { focused: 'n6' })); await frames();
    assert.equal(await active(), 2, 'MP-08: a new kernel focus inside the window applies immediately');
  } finally { await page.close(); }
});

// Review #941 @36a395b05 finding 1: `change` from native controls is not composed, so a select or picker inside an open
// shadow root never reached the document listener; `set` is their only path to the kernel.
test('MP-08: review #941@36a395b05-1 a select and a date picker inside an open shadow root commit their picks as `set`', async () => {
  const page = await viewer();
  try {
    const option = (id, value) => [{ id, parent: 'n6', kind: 'element', tag: 'option', attrs: { value } }, { id: `${id}0`, parent: id, kind: 'text', text: value }];
    await apply(page, { ...snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'div' }, { id: 'n5', parent: 'n4', kind: 'shadow' },
      { id: 'n6', parent: 'n5', kind: 'element', tag: 'select', form: { value: 'Apple', checked: false, selected_index: 0, selection_start: null, selection_end: null } }, ...option('n7', 'Apple'), ...option('n8', 'Banana'),
      { id: 'n9', parent: 'n5', kind: 'element', tag: 'input', attrs: { type: 'date' }, form: textForm('', null) }])), focused: 'n6' });
    const sent = () => page.evaluate(() => window.sent.splice(0).filter(action => action.kind !== 'scroll_to'));
    await page.keyboard.press('ArrowDown'); await frames();
    assert.deepEqual(await sent(), [{ kind: 'set', node_id: 'n6', selected_index: 1 }], 'MP-08: the shadow select\'s pick reaches the kernel');
    await apply(page, delta(2, [], { focused: 'n9' })); await sent();
    await page.keyboard.type('10102026'); await frames();
    assert.deepEqual((await sent()).at(-1), { kind: 'set', node_id: 'n9', value: '2026-10-10' }, 'MP-08: the shadow date commits as one value');
  } finally { await page.close(); }
});

// Review #320 @90c1195fc finding 1: two opposing option changes before the first `set` echoes: the second compared
// against the unchanged kernel record and sent nothing, so the kernel kept the option the user deselected.
test('MP-08: review #320@90c1195fc-1 selecting then deselecting a multiple-select option before the echo sends both changes', async () => {
  const page = await viewer();
  try {
    const form = (checked, extra = {}) => ({ value: '', checked, selected_index: -1, selection_start: null, selection_end: null, ...extra });
    const option = (id, text) => [{ id, parent: 'n4', kind: 'element', tag: 'option', form: form(false, { value: text }) }, { id: `${id}0`, parent: id, kind: 'text', text }];
    await apply(page, snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'select', attrs: { multiple: '', size: '3' }, form: form(false) }, ...option('n5', 'alpha'), ...option('n6', 'beta')])));
    const at = await page.evaluate(() => { const f = window.r.frame.getBoundingClientRect(), o = window.r.frame.contentDocument.querySelector('option').getBoundingClientRect(); return [f.x + o.x + 5, f.y + o.y + o.height / 2]; });
    for (let i = 0; i < 2; i++) { await page.keyboard.down('Control'); await page.mouse.click(at[0], at[1]); await page.keyboard.up('Control'); await frames(); }
    const sets = (await page.evaluate(() => window.sent.splice(0))).filter(action => action.kind === 'set');
    assert.deepEqual(sets, [{ kind: 'set', node_id: 'n5', checked: true }, { kind: 'set', node_id: 'n5', checked: false }], 'MP-08: the deselect follows the select');
    // The kernel's echoes then apply in order; a later page change starts again from the kernel's state.
    await apply(page, delta(2, [{ op: 'form', id: 'n5', form: { checked: true } }])); await apply(page, delta(3, [{ op: 'form', id: 'n5', form: { checked: false } }]));
    await page.keyboard.down('Control'); await page.mouse.click(at[0], at[1]); await page.keyboard.up('Control'); await frames();
    assert.deepEqual((await page.evaluate(() => window.sent.splice(0))).filter(action => action.kind === 'set'), [{ kind: 'set', node_id: 'n5', checked: true }]);
  } finally { await page.close(); }
});

// Review #320 @90c1195fc finding 2: definitions lived in the sandbox registry, which a new source document (top-level
// reset or a replaced frame document) reuses: an undefined tag of the new document still matched `:defined`. The kernel
// rewrites `:defined` to `:not([chariox-undefined])`; the viewer marks undefined custom elements instead of registering.
test('MP-10: review #320@90c1195fc-2 a custom tag defined in one source document is undefined in the next (reset, replaced frame document) and becomes defined by a state op', async () => {
  const page = await viewer();
  try {
    const css = 'x-card:not(:not([chariox-undefined])){display:none}';
    const doc = (id, extra) => [{ id: `${id}1`, parent: null, kind: 'document' }, { id: `${id}2`, parent: `${id}1`, kind: 'element', tag: 'html' }, { id: `${id}3`, parent: `${id}2`, kind: 'element', tag: 'body' }, { id: `${id}4`, parent: `${id}3`, kind: 'element', tag: 'style', css }, ...extra];
    const card = (id, parent, state) => [{ id, parent, kind: 'element', tag: 'x-card', ...(state ? { state } : {}) }, { id: `${id}0`, parent: id, kind: 'text', text: 'card' }];
    const display = () => page.evaluate(() => [...window.r.frame.contentDocument.querySelectorAll('x-card'), ...[...window.r.frame.contentDocument.querySelectorAll('iframe')].flatMap(f => [...f.contentDocument.querySelectorAll('x-card')])].map(e => getComputedStyle(e).display));
    await apply(page, snapshot(doc('n', card('n5', 'n3'))));
    assert.deepEqual(await display(), ['inline'], 'MP-10: a defined card shows');
    await apply(page, { ...snapshot(doc('n', card('n5', 'n3', { defined: false }))), document_id: 'd2' });
    assert.deepEqual(await display(), ['none'], 'MP-10: the next document\'s undefined card is not :defined');
    await apply(page, { ...delta(2, [{ op: 'state', id: 'n5', state: { defined: true } }]), document_id: 'd2' });
    assert.deepEqual(await display(), ['inline'], 'MP-10: its definition arrives as a state op');
    // A frame document replaced by a children op on the frame.
    const frameDoc = (base, state) => { const n = k => `n${base + k}`; return [{ id: n(1), parent: 'n7', kind: 'document' }, { id: n(2), parent: n(1), kind: 'element', tag: 'html' }, { id: n(3), parent: n(2), kind: 'element', tag: 'body' }, { id: n(4), parent: n(3), kind: 'element', tag: 'style', css }, ...card(n(5), n(3), state)]; };
    await apply(page, { ...snapshot(doc('n', [{ id: 'n7', parent: 'n3', kind: 'frame', tag: 'iframe' }, ...frameDoc(20)])), document_id: 'd3' });
    assert.deepEqual(await display(), ['inline'], 'MP-10: the frame\'s defined card shows');
    await apply(page, { ...delta(2, [{ op: 'children', id: 'n7', children: ['n31'], nodes: frameDoc(30, { defined: false }) }]), document_id: 'd3' });
    assert.deepEqual(await display(), ['none'], 'MP-10: the replaced frame document\'s undefined card is not :defined');
  } finally { await page.close(); }
});

// Protocol 501: the kernel's custom validity (setCustomValidity) makes the viewer's copy :invalid too (form included).
test('MP-10: protocol 501 the viewer takes the kernel\'s custom validity (record state, state ops): :invalid styles match', async () => {
  const page = await viewer();
  try {
    const css = 'input:invalid{outline:3px solid rgb(255, 0, 0)} form:invalid{background-color:rgb(255, 255, 0)}';
    await apply(page, snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'style', css }, { id: 'n5', parent: 'n3', kind: 'element', tag: 'form' },
      { id: 'n6', parent: 'n5', kind: 'element', tag: 'input', state: { custom_invalid: true }, form: textForm('abc', null) }, { id: 'n7', parent: 'n5', kind: 'element', tag: 'input', form: textForm('ok', null) }])));
    const seen = () => page.evaluate(() => { const d = window.r.frame.contentDocument, [a, b] = d.querySelectorAll('input'); return [getComputedStyle(a).outlineColor, getComputedStyle(b).outlineStyle, getComputedStyle(d.querySelector('form')).backgroundColor]; });
    assert.deepEqual(await seen(), ['rgb(255, 0, 0)', 'none', 'rgb(255, 255, 0)'], 'MP-10: the invalid control (and its form) match :invalid');
    await apply(page, delta(2, [{ op: 'state', id: 'n6', state: { custom_invalid: false } }, { op: 'state', id: 'n7', state: { custom_invalid: true } }]));
    assert.deepEqual(await seen(), ['rgb(0, 0, 0)', 'solid', 'rgb(255, 255, 0)'], 'MP-10: state ops move the invalidity');
  } finally { await page.close(); }
});

// Protocol 501: an external <use> sprite. data: URLs are not followed by <use>, so the viewer validates the file and
// uses it as a same-origin blob; a file that fails validation (a script) is refused and the use renders nothing.
test('MP-10: protocol 501 an external <use> sprite renders from a validated blob of its file (fragment changes follow); an unsafe file is refused', async () => {
  const page = await viewer();
  try {
    const file = text => { const bytes = Buffer.from(text); return { key: 'r1', resource_id: createHash('sha256').update(bytes).digest('hex'), mime_type: 'image/svg+xml', data_base64: bytes.toString('base64') }; };
    const good = '<svg xmlns="http://www.w3.org/2000/svg"><style>.a{fill:rgb(255,0,0)}</style><symbol id="icon" viewBox="0 0 10 10"><rect class="a" width="10" height="10"/></symbol><symbol id="wide" viewBox="0 0 20 10"><rect width="20" height="10"/></symbol></svg>';
    const nodes = page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'svg', ns: 'svg', attrs: { width: '40', height: '40' } }, { id: 'n5', parent: 'n4', kind: 'element', tag: 'use', ns: 'svg', attrs: { href: '#icon', width: '40', height: '40' }, res: 'r1' }]);
    const seen = () => page.evaluate(() => { const use = window.r.frame.contentDocument.querySelector('use'), b = use.getBBox(); return { href: use.getAttribute('href'), box: [b.width, b.height] }; });
    await apply(page, { ...snapshot(nodes), resources: [file(good)] }); await frames();
    let at = await seen();
    assert.match(at.href ?? '', /^blob:.*#icon$/, 'MP-10: the use names the validated blob and its fragment');
    assert.deepEqual(at.box, [40, 40], 'MP-10: the symbol renders');
    await apply(page, delta(2, [{ op: 'attr', id: 'n5', name: 'href', value: '#wide' }])); await frames();
    assert.match((await seen()).href ?? '', /^blob:.*#wide$/, 'MP-10: a fragment change keeps the file');
    for (const unsafe of [good.replace('</svg>', '<script>alert(1)</script></svg>'), good.replace('class="a"', 'class="a" onclick="alert(1)"'), good.replace('<rect class', '<image href="http://evil.test/x.png"/><rect class'), good.replace('fill:rgb(255,0,0)', 'fill:url(http://evil.test/p)')]) {
      await apply(page, { ...snapshot(nodes), document_id: `d${unsafe.length}`, resources: [file(unsafe)] }); await frames();
      at = await seen();
      assert.deepEqual([at.href, at.box], [null, [0, 0]], `MP-10: refused: ${unsafe.slice(0, 120)}`);
    }
  } finally { await page.close(); }
});

// MP-10 strict Selkies scroll gap: precise wheel steps every 50 ms (a CDP pixel wheel) jumped once per step, so the
// viewer presented one change per step (~20 fps) where a browser animating its wheel ticks presents every frame.
// The viewer eases each step over the frames until the next one and ends exactly where the native steps would.
test('MP-10: spaced precise wheel steps scroll the viewer every frame (document, inner scroller, reversal) and end at the exact native position; frame-rate steps stay immediate; snap scrollers stay native', async () => {
  const page = await viewer();
  try {
    await apply(page, snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body', attrs: { style: 'margin:0;height:6000px;background:linear-gradient(red,blue)' } },
      { id: 'n4', parent: 'n3', kind: 'element', tag: 'div', attrs: { id: 'inner', style: 'position:fixed;left:0;top:0;width:400px;height:300px;overflow:auto' } }, { id: 'n5', parent: 'n4', kind: 'element', tag: 'div', attrs: { style: 'height:3000px' } },
      { id: 'n6', parent: 'n3', kind: 'element', tag: 'div', attrs: { id: 'snap', style: 'position:fixed;left:600px;top:0;width:300px;height:300px;overflow:auto;scroll-snap-type:y mandatory' } },
      ...[7, 8, 9].map(n => ({ id: `n${n}`, parent: 'n6', kind: 'element', tag: 'div', attrs: { style: 'height:300px;scroll-snap-align:start' } }))]));
    await frames();
    // Every animation frame, the scroll positions the viewer presents.
    const run = async (x, y, steps, interval, read) => {
      await page.mouse.move(x, y);
      await page.evaluate(read => { window.seen = []; const doc = window.r.frame.contentDocument; const sample = () => { window.seen.push(read === 'window' ? doc.defaultView.scrollY : doc.getElementById(read).scrollTop); window.sampling = requestAnimationFrame(sample); }; sample(); }, read);
      for (const [n, dy] of steps.entries()) { await page.mouse.wheel(0, dy); if (n < steps.length - 1) await page.waitForTimeout(interval); }
      await page.waitForTimeout(400);
      return page.evaluate(() => { cancelAnimationFrame(window.sampling); return window.seen; });
    };
    const distinct = seen => seen.filter((v, i) => i && v !== seen[i - 1]).length;
    const down = Array(10).fill(16), reversal = [...Array(5).fill(16), ...Array(5).fill(-16)];
    let seen = await run(900, 600, down, 50, 'window');
    assert.equal(seen.at(-1), 160, 'MP-10: the document ends exactly at the sum of its steps');
    assert(distinct(seen) >= 25, `MP-10: 10 spaced steps present at least 25 document frames, got ${distinct(seen)}`);
    seen = await run(200, 150, reversal, 50, 'inner');
    assert.equal(seen.at(-1), 0, 'MP-10: the inner scroller takes the wheel (chaining) and a reversal returns exactly');
    assert(distinct(seen) >= 25, `MP-10: inner scroller frames, got ${distinct(seen)}`);
    assert.equal(await page.evaluate(() => window.r.frame.contentWindow.scrollY), 160, 'MP-10: the document stays while the inner scroller moves');
    // Steps at frame rate (a trackpad) are not delayed: each one is where native scrolling would put it at once.
    await page.mouse.move(900, 600);
    const immediate = await page.evaluate(async () => { const doc = window.r.frame.contentDocument, at = []; for (let n = 0; n < 8; n++) { doc.querySelector('body').dispatchEvent(new WheelEvent('wheel', { deltaY: 10, bubbles: true, cancelable: true, composed: true })); at.push(doc.defaultView.scrollY); await new Promise(r => setTimeout(r, 8)); } return at; });
    assert(immediate.slice(1).every((v,i)=>v-immediate[i]>=9), 'MP-10: every frame-rate step applies immediately, while a previous eased tail finishes smoothly');
    await page.waitForTimeout(350);assert.equal(await page.evaluate(()=>window.r.frame.contentWindow.scrollY),240,'MP-10: exact final position without flushing the old tail');
    // A mouse wheel's notches ease continuously: a reversal presents a change every frame (no stall at the turn).
    const before = await page.evaluate(() => window.r.frame.contentWindow.scrollY);
    const turn = await run(900, 600, [120, 120, -120, -120], 50, 'window');
    assert.equal(turn.at(-1), before, 'MP-10: notches end exactly where native steps would');
    const moving = turn.slice(turn.indexOf(before) >= 0 ? 1 : 0).filter((v, i, a) => i && v !== a[i - 1]).length;
    assert(moving >= 14, `MP-10: notch frames keep moving through the reversal, got ${moving}`);
    // A snap container keeps the native wheel (the browser's own snapping); the event is not taken.
    const taken = await page.evaluate(() => { const snap = window.r.frame.contentDocument.getElementById('snap'), e = new WheelEvent('wheel', { deltaY: 16, bubbles: true, cancelable: true, composed: true }); snap.dispatchEvent(e); return e.defaultPrevented; });
    assert.equal(taken, false, 'MP-10: a snap scroller keeps the native wheel');
  } finally { await page.close(); }
});

// Review #320@4f8b54237 P3: the kernel refuses a text action over 16384 UTF-16 units, so a long paste changed nothing.
test('MP-08: review #320@4f8b54237-1 a paste longer than one text action reaches the kernel in order, in slices that never split a surrogate pair', async () => {
  const page = await viewer();
  try {
    await apply(page, { ...snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'textarea', form: textForm('', 0) }, { id: 'n5', parent: 'n3', kind: 'element', tag: 'div', attrs: { contenteditable: 'true' } }, { id: 'n6', parent: 'n5', kind: 'text', text: 'rich' }])), focused: 'n4' });
    const text = '漢'.repeat(6000) + '😀'.repeat(6000);
    for (const selector of ['textarea', 'div']) {
      const actions = await page.evaluate(async ({ selector, text }) => {
        const doc = window.r.frame.contentDocument, el = doc.querySelector(selector); el.focus(); window.sent.splice(0);
        const transfer = new DataTransfer(); transfer.setData('text/plain', text);
        el.dispatchEvent(new InputEvent('beforeinput', { inputType: 'insertFromPaste', data: selector === 'textarea' ? text : null, dataTransfer: selector === 'textarea' ? null : transfer, bubbles: true, cancelable: true, composed: true }));
        await new Promise(resolve => setTimeout(resolve, 50));
        return window.sent.splice(0).filter(action => action.kind === 'text').map(action => action.text);
      }, { selector, text });
      assert(actions.length === 3 && actions.every(slice => Buffer.byteLength(slice, 'utf8') <= 16384), `MP-08: ${selector} slices ${actions.map(slice => slice.length)}`);
      assert(actions.every(slice=>!/[\uD800-\uDBFF]$/.test(slice)&&! /^[\uDC00-\uDFFF]/.test(slice)), 'MP-08: UTF-8 slices never split a surrogate pair');
      assert.equal(actions.join(''), text, `MP-08: the ${selector} paste arrives whole and in order`);
    }
  } finally { await page.close(); }
});

// b3 r13 hosted A/B (GitHub search): the faster echo overtook the click's reply. The packet naming the search
// dialog's input applied while the click was pending (focus withheld), and the hold then took that new focus for the
// old one: the viewer kept the header button focused and the typed text never reached an editable control.
test('MP-08: an echo that names a new focus before its click\'s reply arrives focuses that control once the reply lands', async () => {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  try {
    await page.goto(url);
    // Replies are held until the test releases them, like a slow input round trip.
    await page.evaluate(async () => { window.sent = []; window.release = []; window.r = new Mirror2.BrowserMirror2Renderer(document.getElementById('c'), action => { window.sent.push(action); return new Promise(resolve => window.release.push(resolve)); }); await window.r.ready(); });
    await apply(page, { ...snapshot(page2([{ id: 'n4', parent: 'n3', kind: 'element', tag: 'button', attrs: { style: 'width:200px;height:30px;display:block' } }, { id: 'n5', parent: 'n4', kind: 'text', text: 'Search' },
      { id: 'n6', parent: 'n3', kind: 'element', tag: 'input', attrs: { style: 'width:300px;display:block' }, form: textForm('', 0) }])), focused: null });
    const box = await page.evaluate(() => { const f = window.r.frame.getBoundingClientRect(), t = window.r.frame.contentDocument.querySelector('button').getBoundingClientRect(); return { x: f.x + t.x + 20, y: f.y + t.y + 10 }; });
    await page.mouse.click(box.x, box.y); await frames();
    assert.deepEqual(await page.evaluate(() => window.sent.map(a => a.kind)), ['click']);
    await apply(page, delta(2, [], { focused: 'n6' })); await frames(); // the echo, before the reply
    await page.evaluate(() => window.release.shift()()); await frames();
    assert.equal(await page.evaluate(() => window.r.frame.contentDocument.activeElement?.localName), 'input', 'MP-08: the kernel\'s new focus is taken once the input settles');
    await page.keyboard.type('c'); await frames();
    assert.deepEqual(await page.evaluate(() => window.sent.slice(1).map(a => [a.kind, a.key])), [['key', 'c']], 'MP-08: typing reaches the kernel');
  } finally { await page.close(); }
});

test('MP-08/MP-10: review r19 an attached renderer preserves its host page\'s endless animations (Chromium otherwise presents every echo a frame later) and resumes them on close; the mirrored page keeps its own', async () => {
  const page = await browser.newPage({ viewport: { width: 1400, height: 900 } });
  try {
    await page.goto(url);
    const states = () => page.evaluate(() => Object.fromEntries(document.getAnimations().map(a => [a.animationName ?? a.id, a.playState])));
    await page.evaluate(() => {
      const style = document.createElement('style');
      style.textContent = '@keyframes pulse{50%{opacity:.4}} .status::before{content:"";display:inline-block;width:8px;height:8px;background:#0a0;animation:pulse 1.8s ease-in-out infinite} @keyframes toast{from{opacity:0}} .toast{animation:toast 30s} .status.app-paused::before{animation-play-state:paused}';
      document.head.append(style); document.body.insertAdjacentHTML('beforeend', '<span class="status">pending</span><div class="toast">saved</div>');
    });
    assert.deepEqual(await states(), { pulse: 'running', toast: 'running' });
    await page.evaluate(async () => { window.r = new Mirror2.BrowserMirror2Renderer(document.getElementById('c'), async () => {}); await window.r.ready(); window.r.frame.contentDocument.documentElement.animate([{ opacity: 1 }, { opacity: 0.9 }], { duration: 1000, iterations: Infinity, id: 'mirrored' }); });
    assert.deepEqual(await states(), { pulse: 'running', toast: 'running' }, 'MP-10: the host page\'s endless animation is paused, a finite one plays out');
    await page.evaluate(() => document.body.insertAdjacentHTML('beforeend', '<span class="status">later</span>'));
    await frames();
    assert.deepEqual((await page.evaluate(() => document.getAnimations().filter(a => a.animationName === 'pulse').map(a => a.playState))), ['running', 'running'], 'MP-10: late host animations remain under host control');
    assert.equal(await page.evaluate(() => window.r.frame.contentDocument.getAnimations()[0].playState), 'running', 'MP-10: the mirrored page keeps its own animations');
    await page.evaluate(() => { document.querySelector('.status').classList.add('app-paused'); const el=document.body.appendChild(document.createElement('span')); window.late=el.animate([{opacity:1},{opacity:.5}],{duration:1000,iterations:Infinity}); });
    await frames(); assert.equal(await page.evaluate(()=>window.late.playState),'running','MP-10: late WAAPI animation remains owned by the host');
    await page.evaluate(() => window.r.close());
    assert.deepEqual((await page.evaluate(() => document.getAnimations().filter(a => a.animationName === 'pulse').map(a => a.playState))), ['paused', 'running'], 'MP-10: closing a mirror respects an app-paused host animation');
    await page.evaluate(()=>document.querySelector('.status').classList.remove('app-paused'));await frames();
    assert.deepEqual(await page.evaluate(()=>document.getAnimations().filter(a=>a.animationName==='pulse').map(a=>a.playState)),['running','running'],'MP-10: CSS play-state control still works after the mirror closes');
  } finally { await page.close(); }
});

// MP-08/MP-10: review #320 RTL and native-scroll ownership; review #941 fast trackpad tail.
test('MP-10: review r19 RTL wheel keeps signed offsets and an unrelated axis', async () => {
  const page=await viewer();
  try{
    await apply(page,snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},
      {id:'n4',parent:'n3',kind:'element',tag:'div',attrs:{id:'rtl',style:'width:300px;height:200px;overflow:auto;direction:rtl'}},{id:'n5',parent:'n4',kind:'element',tag:'div',attrs:{style:'width:1200px;height:1200px'}}]));
    await page.evaluate(()=>window.r.frame.contentDocument.getElementById('rtl').scrollTo({left:-200,top:100,behavior:'instant'}));await frames();
    const box=await page.locator('iframe:not([inert])').boundingBox();await page.mouse.move(box.x+150,box.y+100);await page.mouse.wheel(0,16);await page.waitForTimeout(350);
    assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getElementById('rtl').scrollLeft),-200,'vertical wheel leaves negative horizontal position unchanged');
    await page.mouse.wheel(16,0);await page.waitForTimeout(350);assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getElementById('rtl').scrollLeft),-184,'positive horizontal step moves within signed bounds');
    await page.mouse.wheel(-16,0);await page.waitForTimeout(350);assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getElementById('rtl').scrollLeft),-200,'negative horizontal step is admitted');
  }finally{await page.close();}
});
test('MP-10: review r19 a native scroll takes ownership during a viewport or inner glide', async()=>{
 const page=await viewer();try{
  await apply(page,snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body',attrs:{style:'height:4000px'}},{id:'n4',parent:'n3',kind:'element',tag:'div',attrs:{id:'inner',style:'width:300px;height:200px;overflow:auto'}},{id:'n5',parent:'n4',kind:'element',tag:'div',attrs:{style:'height:3000px'}}]));
  for(const inner of [false,true]){
   const result=await page.evaluate(inner=>{const d=window.r.frame.contentDocument,e=inner?d.getElementById('inner'):d.scrollingElement,w=window.r.wheel;w.close();e.scrollTo({top:100,behavior:'instant'});w.take(new WheelEvent('wheel',{deltaY:120}),e,100);w.step(w.glides.get(e),116);e.scrollTo({top:700,behavior:'instant'});w.step(w.glides.get(e),132);return{position:inner?e.scrollTop:d.defaultView.scrollY,glides:w.glides.size}},inner);
   assert.equal(result.position,700,'native movement survives the next animation frame');assert.equal(result.glides,0,'the old glide relinquishes ownership');
  }
 }finally{await page.close();}
});
test('MP-10: review r19 large frame-rate deltas apply immediately while an existing eased tail stays smooth',async()=>{
 const page=await viewer();try{
  await apply(page,snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body',attrs:{style:'height:6000px'}}]));
  const result=await page.evaluate(()=>{const d=window.r.frame.contentDocument,e=d.scrollingElement,w=window.r.wheel;w.close();e.scrollTo({top:100,behavior:'instant'});w.take(new WheelEvent('wheel',{deltaY:120}),e,100);w.step(w.glides.get(e),116);const before=d.defaultView.scrollY;w.take(new WheelEvent('wheel',{deltaY:120}),e,116);const fast=d.defaultView.scrollY;w.take(new WheelEvent('wheel',{deltaY:10}),e,132);const tail=d.defaultView.scrollY;w.step(w.glides.get(e),148);return{before,fast,tail,frame:d.defaultView.scrollY}});
  assert(result.fast-result.before>=119,'large trackpad step follows its frame, rather than being classified as a notch');assert(result.tail-result.fast<=11,'falling below 50px does not flush the previous backlog');assert(result.frame-result.tail<50,'the retained tail remains eased');
 }finally{await page.close();}
});

// MP-08/MP-10/MP-11: a retired mirror is inert but holds the last safe paint
// until protected video has actually painted its replacement.
test('MP-10: retiring a mirror preserves inert paint until replacement disposal',async()=>{
 const page=await viewer()
 try{
  await apply(page,snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},{id:'n4',parent:'n3',kind:'element',tag:'button'},{id:'n5',parent:'n4',kind:'text',text:'Last painted page'}]))
  await page.evaluate(()=>window.r.close({preserveFrame:true}))
  assert.deepEqual(await page.evaluate(()=>({connected:window.r.frame.isConnected,text:window.r.frame.contentDocument?.body?.innerText})),{connected:true,text:'Last painted page'})
  await page.evaluate(()=>window.r.frame.contentDocument.querySelector('button').click())
  assert.equal(await page.evaluate(()=>window.sent.length),0)
  await page.evaluate(()=>window.r.close())
  assert.equal(await page.evaluate(()=>window.r.frame.isConnected),false)
 }finally{await page.close()}
})

for(const delay of [30,700])test(`MP-08/MP-10/MP-11 M2: a same-document reset preserves a wheel in progress under its new epoch (${delay}ms input)`,async()=>{
 const page=await viewer();try{
  const nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body',attrs:{style:'margin:0;height:6000px'}}];
  await apply(page,snapshot(nodes));await frames();
  await page.evaluate(()=>{window.epochs=[];window.r.send=async(action,epoch)=>{window.epochs.push({action,epoch});if(!window.released)await new Promise(resolve=>window.releaseInput=resolve)};});
  await page.mouse.move(900,600);await page.mouse.wheel(0,500);
  await page.waitForTimeout(delay);
  await apply(page,{...snapshot(nodes),sequence:2});await frames();
  assert.equal(await page.evaluate(()=>window.r.frame.contentWindow.scrollY),500,'a resnapshot must not cancel the remaining wheel intent');
  await page.evaluate(()=>{window.released=true;window.releaseInput?.()});await frames();
  const sent=await page.evaluate(()=>window.epochs);
  assert(sent.some(e=>e.action.kind==='scroll_to'&&e.action.node_id===null&&Math.abs(e.action.y-500)<0.001&&e.epoch.sequence===2),'only a current-epoch root position reaches the normal input path: '+JSON.stringify(sent));
  await apply(page,{...snapshot(nodes),sequence:3,document_id:'another-document'});await frames();
  assert.equal(await page.evaluate(()=>window.r.frame.contentWindow.scrollY),0,'navigation never inherits the previous document wheel');
  assert(!(await page.evaluate(()=>window.epochs)).some(e=>e.epoch.document_id==='another-document'),'no previous-document gesture is replayed');
 }finally{await page.evaluate(()=>{window.released=true;window.releaseInput?.()}).catch(()=>{});await page.close()}
});

test('MP-08/MP-10: delayed mirror layout cannot turn scroll anchoring into user input',async()=>{
  const page=await viewer();try{
    await apply(page,snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},
      {id:'n3',parent:'n2',kind:'element',tag:'body',attrs:{style:'margin:0'}},
      {id:'n4',parent:'n3',kind:'element',tag:'div',attrs:{style:'height:1000px'}},
      {id:'n5',parent:'n3',kind:'element',tag:'p',attrs:{style:'height:1000px'}},
      {id:'n6',parent:'n5',kind:'text',text:'visible anchor'},
      {id:'n7',parent:'n3',kind:'element',tag:'div',attrs:{style:'height:1000px'}}]));
    await page.evaluate(()=>window.r.frame.contentWindow.scrollTo(0,1000));await frames();
    await page.evaluate(()=>window.sent=[]);
    await apply(page,delta(2,[{op:'attr',id:'n4',name:'style',value:'height:1200px'}]));await frames();
    assert.equal(await page.evaluate(()=>window.r.frame.contentWindow.scrollY),1000,'late resources/layout must preserve the viewer scroll');
    assert.equal((await page.evaluate(()=>window.sent)).filter(a=>a.kind==='scroll_to').length,0,'internal hydration must not send another user scroll');
  }finally{await page.close()}
});

// MP-08/MP-10/MP-11: replacement records reuse their source IDs.
test('MP-08/MP-10/MP-11 replacement subtree retains its live index for subsequent deltas',async()=>{
 const page=await viewer();try{
  const nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},{id:'n4',parent:'n3',kind:'element',tag:'div'},{id:'n5',parent:'n4',kind:'element',tag:'button',attrs:{'aria-label':'replacement action'}}];
  await apply(page,snapshot(nodes));
  await apply(page,delta(2,[{op:'children',id:'n3',children:['n4'],nodes:nodes.slice(3)}]));
  assert.equal(await page.evaluate(()=>window.r.dom.get('n4')===window.r.frame.contentDocument.querySelector('div')),true,'the visible replacement owns its source ID');
  await apply(page,delta(3,[{op:'children',id:'n4',children:['n5','n6'],nodes:[{id:'n6',parent:'n4',kind:'text',text:'live update after replacement'}]}]));
  assert.equal(await page.frameLocator('iframe:not([inert])').getByText('live update after replacement').isVisible(),true);
  await page.frameLocator('iframe:not([inert])').getByRole('button',{name:'replacement action'}).click();
  assert((await page.evaluate(()=>window.sent)).some(a=>a.kind==='click'&&a.node_id==='n5'),'replacement input retains its node ID');
 }finally{await page.close()}
});

for(const shadow of [false,true])test(`MP-08/MP-10/MP-11 promoted known child keeps its index after wrapper retirement shadow=${shadow}`,async()=>{
 const page=await viewer();try{
  const nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},{id:'n4',parent:'n3',kind:'element',tag:'div'},...(shadow?[{id:'n7',parent:'n4',kind:'shadow'}]:[]),{id:'n5',parent:shadow?'n7':'n4',kind:'element',tag:'button',attrs:{'aria-label':'promoted action'}},{id:'n6',parent:'n5',kind:'text',text:'kept'}];
  await apply(page,snapshot(nodes));
  await apply(page,delta(2,[{op:'children',id:'n3',children:['n5'],nodes:[]}]));
  assert.equal(await page.evaluate(()=>window.r.dom.get('n5')===window.r.frame.contentDocument.querySelector('button')),true,'moving a descendant out must not retire its live index with the removed wrapper');
  assert.equal(await page.evaluate(()=>window.r.records.get('n5')?.parent),'n3','the live record follows its new parent');
  await apply(page,delta(3,[{op:'children',id:'n5',children:['n6','n8'],nodes:[{id:'n8',parent:'n5',kind:'text',text:'still live after promotion'}]}]));
  assert(await page.frameLocator('iframe:not([inert])').getByText('still live after promotion',{exact:false}).isVisible());
  await page.frameLocator('iframe:not([inert])').getByRole('button',{name:'promoted action'}).click();
  assert((await page.evaluate(()=>window.sent)).some(a=>a.kind==='click'&&a.node_id==='n5'));
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11: retiring a frame must retain a child adopted into its parent document.
test('MP-08/MP-10/MP-11 promoted frame child keeps its index after frame retirement',async()=>{
 const page=await viewer();try{
  const nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},
   {id:'n4',parent:'n3',kind:'frame',tag:'iframe'},{id:'n7',parent:'n4',kind:'document'},{id:'n8',parent:'n7',kind:'element',tag:'html'},{id:'n9',parent:'n8',kind:'element',tag:'body'},
   {id:'n5',parent:'n9',kind:'element',tag:'button',attrs:{'aria-label':'promoted frame action'}},{id:'n6',parent:'n5',kind:'text',text:'kept'}];
  await apply(page,snapshot(nodes));
  await apply(page,delta(2,[{op:'children',id:'n3',children:['n5'],nodes:[]}]));
  assert.equal(await page.evaluate(()=>window.r.dom.get('n5')===window.r.frame.contentDocument.querySelector('button')),true,'adopting a frame descendant must not retire its live index with the removed frame');
  await apply(page,delta(3,[{op:'children',id:'n5',children:['n6','n10'],nodes:[{id:'n10',parent:'n5',kind:'text',text:'live after frame promotion'}]}]));
  assert(await page.frameLocator('iframe:not([inert])').getByText('live after frame promotion',{exact:false}).isVisible());
  await page.frameLocator('iframe:not([inert])').getByRole('button',{name:'promoted frame action'}).click();
  assert((await page.evaluate(()=>window.sent)).some(a=>a.kind==='click'&&a.node_id==='n5'));
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11: selection capture must follow all written mirror input replies.
test('MP-10: Notes waits for mirror selection input receipts',async()=>{
 const page=await viewer();
 try{
  await page.evaluate(async()=>{
   window.r.close();window.sent=[];
   window.r=new Mirror2.BrowserMirror2Renderer(document.getElementById('c'),async action=>{window.sent.push(action);await new Promise(resolve=>{window.releaseInput=resolve})});await window.r.ready();
  });
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'The full released selection'}])));
  await page.evaluate(()=>{const d=window.r.frame.contentDocument,t=d.querySelector('p').firstChild;d.getSelection().setBaseAndExtent(t,0,t,12)});
  await page.waitForFunction(()=>window.sent.some(a=>a.kind==='selection'));
  await page.evaluate(()=>{window.idle=false;window.idlePromise=window.r.settleInput().then(()=>{window.idle=true})});
  await frames();assert.equal(await page.evaluate(()=>window.idle),false);
  await page.evaluate(async()=>{window.releaseInput();await window.idlePromise});assert.equal(await page.evaluate(()=>window.idle),true);
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11: waiting for input cannot authorize capture in a replacement document.
test('MP-10: Notes input settling rejects a document replaced during the wait',async()=>{
 const page=await viewer();
 try{
  await page.evaluate(async()=>{window.r.close();window.sent=[];window.r=new Mirror2.BrowserMirror2Renderer(document.getElementById('c'),async action=>{window.sent.push(action);await new Promise(resolve=>{window.releaseInput=resolve})});await window.r.ready()});
  const nodes=page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'The released selection'}]);
  await apply(page,snapshot(nodes));
  await page.evaluate(()=>{const d=window.r.frame.contentDocument,t=d.querySelector('p').firstChild;d.getSelection().setBaseAndExtent(t,0,t,12)});
  await page.waitForFunction(()=>window.sent.some(a=>a.kind==='selection'));
  await page.evaluate(()=>{window.idlePromise=window.r.settleInput().then(()=>null,e=>e.message)});
  await apply(page,{...snapshot(nodes),document_id:'new-document'});
  assert.equal(await page.evaluate(async()=>{window.releaseInput();return await window.idlePromise}),'MP-08: mirror document changed');
 }finally{await page.close()}
});


// MP-08/MP-10/MP-11 r38: real GitHub's large head spans multiple bounded packets.
// The first reset must not expose a head-only document between those continuations.
test('MP-10 navigation keeps the prior page until the continued document has visible content', async () => {
  const page = await viewer();
  try {
    await apply(page, snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},
      {id:'n3',parent:'n2',kind:'element',tag:'body'},{id:'n4',parent:'n3',kind:'element',tag:'p'},
      {id:'n5',parent:'n4',kind:'text',text:'Prior page stays visible'}]));
    const shown = () => page.evaluate(() => [...document.querySelectorAll('#c > iframe')].filter(f=>getComputedStyle(f).visibility!=='hidden').map(f=>f.contentDocument?.body?.innerText??'').join('|'));
    assert.match(await shown(),/Prior page stays visible/);
    await apply(page, {...snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},
      {id:'n3',parent:'n2',kind:'element',tag:'head'},{id:'n4',parent:'n3',kind:'element',tag:'style',css:'body{margin:0}'}]),document_id:'next',sequence:2});
    await frames();
    assert.match(await shown(),/Prior page stays visible/,'MP-10: head-only reset is prepared offscreen');
    await apply(page,delta(3,[{op:'children',id:'n2',children:['n3','n5'],nodes:[
      {id:'n5',parent:'n2',kind:'element',tag:'body'},{id:'n6',parent:'n5',kind:'element',tag:'a',attrs:{style:'position:absolute;width:1px;height:1px;clip:rect(0px,0px,0px,0px);overflow:hidden'}},
      {id:'n7',parent:'n6',kind:'text',text:'Skip to content'}]}],{document_id:'next'}));
    await frames();assert.match(await shown(),/Prior page stays visible/,'MP-10: a clipped skip link is not a paint');
    await apply(page,delta(4,[{op:'children',id:'n5',children:['n6','n8'],nodes:[
      {id:'n8',parent:'n5',kind:'element',tag:'h1'},{id:'n9',parent:'n8',kind:'text',text:'Sign in to GitHub'}]}],{document_id:'next'}));
    await frames();assert.match(await shown(),/Sign in to GitHub/);assert.doesNotMatch(await shown(),/Prior page stays visible/);
    assert.equal(await page.locator('#c > iframe:not([inert])').count(),1,'MP-11: one active document; the retired document is released');
    assert.equal(await page.locator('#c > iframe[inert]').count(),1,'one inert preloaded spare');
  } finally { await page.close(); }
});

// BROWSER-UX-09 (browserux r20, typed github.com): the head-only continuation of a real page shows
// an unstyled, visible "Skip to content" link; presenting it put a near-white page over the old one.
test('MP-10 an unfinished document holds the prior page until it covers the viewport; a sparse one presents after a bound', async () => {
  const page = await viewer();
  try {
    const shown = () => page.evaluate(() => [...document.querySelectorAll('#c > iframe')].filter(f=>getComputedStyle(f).visibility!=='hidden').map(f=>f.contentDocument?.body?.innerText??'').join('|'));
    const pending = { snapshot_complete: false };
    const skip = (document_id, sequence) => ({...snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},
      {id:'n4',parent:'n3',kind:'element',tag:'a'},{id:'n5',parent:'n4',kind:'text',text:'Skip to content'}]),...pending,document_id,sequence});
    await apply(page, snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'Prior page stays visible'}])));
    await apply(page, skip('next', 2)); await frames();
    assert.match(await shown(), /Prior page stays visible/, 'a visible skip link alone is not meaningful content');
    await apply(page, delta(3, [{op:'children',id:'n3',children:['n4','n6'],nodes:[{id:'n6',parent:'n3',kind:'element',tag:'main'},{id:'n7',parent:'n6',kind:'element',tag:'h1'},{id:'n8',parent:'n7',kind:'text',text:'Repository title'},
      {id:'n9',parent:'n6',kind:'element',tag:'p'},{id:'n10',parent:'n9',kind:'text',text:'Readme text '.repeat(200)}]}], {...pending,document_id:'next'})); await frames();
    assert.match(await shown(), /Repository title/); assert.doesNotMatch(await shown(), /Prior page stays visible/);
    await apply(page, skip('sparse', 4)); await frames();
    assert.match(await shown(), /Repository title/, 'the sparse continuation is held');
    await page.waitForFunction(() => window.r.painted, {}, { timeout: 3000 });
    assert.equal(await shown(), 'Skip to content', 'a sparse page presents after the bound');
    await apply(page, {...snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'Tiny'}])),document_id:'tiny',sequence:5}); await frames();
    assert.equal(await shown(), 'Tiny', 'a completed small page presents at once');
  } finally { await page.close(); }
});

test('MP-11 navigation retires unpainted replacements, preserves the last paint on failure, and closes both frames', async () => {
  const page = await viewer();
  try {
    const old = [{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},
      {id:'n3',parent:'n2',kind:'element',tag:'body'},{id:'n4',parent:'n3',kind:'element',tag:'input',attrs:{id:'old'}}];
    await apply(page,snapshot(old));await page.locator('#c > iframe:not([inert])').contentFrame().locator('input').click();await page.evaluate(async()=>{await window.r.settleInput();window.sent=[]});
    const head = [{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'head'}];
    await apply(page,{...snapshot(head),sequence:2,document_id:'next'});
    await page.keyboard.type('must not reach the new document');
    assert.deepEqual(await page.evaluate(()=>window.sent),[],'MP-11: the held document has no input authority');
    await apply(page,{...snapshot(head),sequence:3,document_id:'third'});
    assert.equal(await page.locator('#c > iframe').count(),2,'MP-11: one held paint plus one bounded replacement');
    await page.evaluate(()=>window.r.close({preserveFrame:true}));
    assert.equal(await page.locator('#c > iframe').count(),1);
    assert.equal(await page.locator('#c > iframe').evaluate(f=>f.inert&&!!f.contentDocument.querySelector('#old')),true);
    await page.evaluate(()=>window.r.close());assert.equal(await page.locator('#c > iframe').count(),0);
  } finally {await page.close();}
});

test('MP-11 same-document protection reset replaces the live field immediately', async()=>{
  const page=await viewer();
  try{
    const base=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'}];
    await apply(page,snapshot([...base,{id:'n4',parent:'n3',kind:'element',tag:'input',attrs:{id:'field'}}]));
    await apply(page,{...snapshot([...base,{id:'n4',parent:'n3',kind:'mask',tag:'input',size:[100,30]}]),sequence:2});
    assert.equal(await page.locator('#c > iframe:not([inert])').count(),1,'MP-11: a policy reset cannot retain the unmasked field');
    assert.equal(await page.locator('#c > iframe:not([inert])').evaluate(f=>f.contentDocument.querySelector('[aria-label="Protected content"]')?.getBoundingClientRect().width),100);
  }finally{await page.close();}
});

test('MP-10 a continued page containing only an open shadow root can paint', async()=>{
 const page=await viewer();
 try{
  await apply(page,snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'Prior'}]));
  await apply(page,{...snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},
    {id:'n4',parent:'n3',kind:'element',tag:'div'},{id:'n5',parent:'n4',kind:'shadow'},{id:'n6',parent:'n5',kind:'element',tag:'p'},{id:'n7',parent:'n6',kind:'text',text:'Shadow page'}]),sequence:2,document_id:'shadow'});
  assert.equal(await page.locator('#c > iframe:not([inert])').count(),1);assert.equal(await page.evaluate(()=>window.r.painted),true);
 }finally{await page.close();}
});

// MP-08/MP-10/MP-11: an already acknowledged prefix can arrive while a person extends their drag.
test('MP-10: a selection echo cannot move an in-progress pointer range',async()=>{
 const page=await viewer();
 try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'The full released selection'}])));
  await page.evaluate(()=>{const d=window.r.frame.contentDocument,p=d.querySelector('p'),t=p.firstChild;p.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0,buttons:1}));d.getSelection().setBaseAndExtent(t,0,t,17)});
  await page.waitForFunction(()=>window.sent.some(a=>a.kind==='selection'));await page.evaluate(()=>window.r.settleInput());
  await apply(page,delta(2,[],{selection:{anchor_id:'n5',anchor_offset:0,focus_id:'n5',focus_offset:3}}));
  assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getSelection().toString()),'The full released','the old kernel prefix must not replace the person’s held range');
  await page.evaluate(()=>window.r.frame.contentDocument.querySelector('p').dispatchEvent(new PointerEvent('pointerup',{bubbles:true,button:0,buttons:0})));
  await apply(page,delta(3,[],{selection:{anchor_id:'n5',anchor_offset:0,focus_id:'n5',focus_offset:3}}));
  assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getSelection().toString()),'The full released','released endpoint survives stale in-flight prefixes');
  for(let sequence=4;sequence<=7;sequence++)await apply(page,delta(sequence,[],{selection:{anchor_id:'n5',anchor_offset:0,focus_id:'n5',focus_offset:3}}));
  assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getSelection().toString()),'The','after the bounded credit window another actor’s selection still projects');
 }finally{await page.close()}
});

test('MP-10: a cancelled or externally released drag cannot keep later kernel selections frozen',async()=>{
 for(const release of ['cancel','outside','blur','navigation']){
  const page=await viewer();
  try{
   const nodes=page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'The full released selection'}]);
   await apply(page,snapshot(nodes));
   await page.evaluate(()=>{const d=window.r.frame.contentDocument,p=d.querySelector('p'),t=p.firstChild;p.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0,buttons:1}));d.getSelection().setBaseAndExtent(t,0,t,17)});
   await page.waitForFunction(()=>window.sent.some(a=>a.kind==='selection'));await page.evaluate(()=>window.r.settleInput());
   if(release==='navigation')await apply(page,{...snapshot(nodes),document_id:'replacement',selection:{anchor_id:'n5',anchor_offset:0,focus_id:'n5',focus_offset:3}});
   else{
    await page.evaluate(release=>{if(release==='cancel')window.r.frame.contentDocument.querySelector('p').dispatchEvent(new PointerEvent('pointercancel',{bubbles:true}));else if(release==='outside')document.dispatchEvent(new PointerEvent('pointerup',{bubbles:true}));else window.dispatchEvent(new Event('blur'))},release);
    for(let sequence=2;sequence<=7;sequence++)await apply(page,delta(sequence,[],{selection:{anchor_id:'n5',anchor_offset:0,focus_id:'n5',focus_offset:3}}));
   }
   assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getSelection().toString()),'The',release);
  }finally{await page.close()}
 }
});

// MP-08/MP-10/MP-11: same native document, rebased snapshot IDs and a final selectionchange still queued.
test('MP-10: same-document resets reconcile released ranges and Notes flushes the final endpoint',async()=>{
 const page=await viewer();
 try{
  const nodes=base=>[{id:`n${base}`,parent:null,kind:'document'},{id:`n${base+1}`,parent:`n${base}`,kind:'element',tag:'html'},{id:`n${base+2}`,parent:`n${base+1}`,kind:'element',tag:'body'},{id:`n${base+3}`,parent:`n${base+2}`,kind:'element',tag:'p'},{id:`n${base+4}`,parent:`n${base+3}`,kind:'text',text:'The witness has called'}];
  await apply(page,snapshot(nodes(1)));
  await page.evaluate(()=>{const d=window.r.frame.contentDocument,t=d.querySelector('p').firstChild;window.originalRoot=d.documentElement;d.getSelection().setBaseAndExtent(t,0,t,t.length)});
  await page.waitForFunction(()=>window.sent.some(a=>a.kind==='selection'));
  await apply(page,{...snapshot(nodes(21)),sequence:2});
  assert.deepEqual(await page.evaluate(()=>{const d=window.r.frame.contentDocument;return[d.documentElement===window.originalRoot,d.getSelection().toString()]}),[true,'The witness has called']);
  await page.evaluate(async()=>{const d=window.r.frame.contentDocument,t=d.querySelector('p').firstChild;d.getSelection().setBaseAndExtent(t,0,t,t.length-2);await window.r.settleInput()});
  assert.deepEqual((await page.evaluate(()=>window.sent.filter(a=>a.kind==='selection'))).at(-1),{kind:'selection',anchor_id:'n25',anchor_offset:0,focus_id:'n25',focus_offset:20},'Notes samples the current endpoint before its asynchronous selectionchange');
 }finally{await page.close()}
});

test('MP-11: reset range reconciliation cannot cross changed text or a protection mask',async()=>{
 for(const mode of ['text','mask','document']){
  const page=await viewer();try{
   const nodes=page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'The witness has called'}]);
   await apply(page,snapshot(nodes));await page.evaluate(()=>{const d=window.r.frame.contentDocument,t=d.querySelector('p').firstChild;d.getSelection().setBaseAndExtent(t,0,t,t.length)});await frames();
   const changed=structuredClone(nodes);if(mode==='text')changed[4].text='Another witness called';if(mode==='mask')changed.splice(3,2,{id:'n4',parent:'n3',kind:'mask',tag:'span',size:[100,20]});
   await apply(page,{...snapshot(changed),sequence:2,...(mode==='document'?{document_id:'new'}:{})});
   assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getSelection().toString()),'',mode);
  }finally{await page.close()}
 }
});

test('MP-11: a refused final selection cannot silently authorize an older Notes quote',async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'The witness has called'}])));
  const result=await page.evaluate(async()=>{window.r.send=async()=>{throw Error('stale epoch')};const d=window.r.frame.contentDocument,t=d.querySelector('p').firstChild;d.getSelection().setBaseAndExtent(t,0,t,t.length);try{await window.r.settleInput();return null}catch(e){return e.message}});
  assert.equal(result,'MP-10: mirror selection refused');
 }finally{await page.close()}
});

test('MP-11: same-document reset cannot resurrect a range the kernel cleared',async()=>{
 const page=await viewer();try{
  const nodes=page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'Native selection'}]);
  await apply(page,snapshot(nodes));await apply(page,delta(2,[],{selection:{anchor_id:'n5',anchor_offset:0,focus_id:'n5',focus_offset:6}}));await frames();
  await apply(page,{...snapshot(nodes),sequence:3});await frames();
  assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getSelection().toString()),'');
  assert.equal(await page.evaluate(()=>window.sent.filter(a=>a.kind==='selection').length),0);
 }finally{await page.close()}
});

test('MP-11: an old-document selection refusal cannot poison the new document range',async()=>{
 const page=await viewer();try{
  const nodes=page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'Current selection'}]);
  await page.evaluate(()=>{window.r.send=async(_a,epoch)=>{if(epoch.document_id==='d')await new Promise((_resolve,reject)=>{window.rejectOld=()=>reject(Error('retired document'))})}});
  await apply(page,snapshot(nodes));await page.evaluate(()=>{const d=window.r.frame.contentDocument,t=d.querySelector('p').firstChild;d.getSelection().setBaseAndExtent(t,0,t,7)});await page.waitForFunction(()=>!!window.rejectOld);
  await apply(page,{...snapshot(nodes),document_id:'new'});
  const result=await page.evaluate(async()=>{const d=window.r.frame.contentDocument,t=d.querySelector('p').firstChild;d.getSelection().setBaseAndExtent(t,0,t,7);const settled=window.r.settleInput().then(()=>null,e=>e.message);window.rejectOld();return settled});
  assert.equal(result,null,'a refused input for d must not mark the current new-document selection refused');
 }finally{await page.close()}
});

test('MP-11: a same-document ID rebase cannot focus a reused unrelated ID on drag release',async()=>{
 const page=await viewer();try{
  const nodes=base=>[{id:`n${base}`,parent:null,kind:'document'},{id:`n${base+1}`,parent:`n${base}`,kind:'element',tag:'html'},{id:`n${base+2}`,parent:`n${base+1}`,kind:'element',tag:'body'},{id:`n${base+3}`,parent:`n${base+2}`,kind:'element',tag:'p',attrs:{tabindex:'0'}},{id:`n${base+4}`,parent:`n${base+3}`,kind:'text',text:'The witness has called'}];
  await apply(page,snapshot(nodes(1)));await page.evaluate(()=>{const d=window.r.frame.contentDocument,p=d.querySelector('p'),t=p.firstChild;p.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,button:0,buttons:1}));p.focus();d.getSelection().setBaseAndExtent(t,0,t,10)});await page.waitForFunction(()=>window.sent.some(a=>a.kind==='selection'));
  await apply(page,{...snapshot([...nodes(21),{id:'n4',parent:'n23',kind:'element',tag:'button'}]),sequence:2});
  await page.evaluate(()=>window.r.frame.contentDocument.querySelector('p').dispatchEvent(new PointerEvent('pointerup',{bubbles:true,button:0,buttons:0})));await frames();
  assert(!(await page.evaluate(()=>window.sent)).some(a=>a.kind==='focus'&&a.node_id==='n4'),'no old pointer-focus ID can be relabelled with the new epoch');
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11 review #1016 P2 / Cloud #368 P2: empty successful pages have no paint to arrive.
for (const prior of [false,true]) test(`MP-10 completed empty document releases its navigation hold prior=${prior}`, async()=>{
 const page=await viewer();try{
  if(prior)await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'Previous site'}])));
  await apply(page,{...snapshot(page2([])),document_id:prior?'empty':'d',sequence:prior?2:1});
  await page.waitForFunction(()=>window.r.painted,{},{timeout:3000});
  assert.equal(await page.locator('#c > iframe:not([inert])').count(),1,'a completed blank page must retire the previous site');
  assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.body.innerText),'');
  assert.equal(await page.evaluate(()=>getComputedStyle(window.r.frame).visibility),'visible');
 }finally{await page.close();}
});

test('MP-11 an empty document timer cannot reveal a retired navigation or preserved failure frame',async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'Previous site'}])));
  await apply(page,{...snapshot(page2([])),document_id:'empty',sequence:2});
  await page.evaluate(()=>window.r.close({preserveFrame:true}));
  await new Promise(resolve=>setTimeout(resolve,2200));
  assert.equal(await page.locator('#c > iframe').count(),1);
  assert.equal(await page.locator('#c > iframe').evaluate(f=>f.inert&&f.contentDocument.body.innerText==='Previous site'),true);
 }finally{await page.close();}
});

test('MP-10 empty-document hold never releases a head-only continuation',async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'Previous site'}])));
  await apply(page,{...snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'head'}]),document_id:'empty',sequence:2});
  await new Promise(resolve=>setTimeout(resolve,2200));
  assert.equal(await page.evaluate(()=>window.r.painted),false,'a missing body is still a continuation');
  assert.equal(await page.locator('#c > iframe').count(),2);
  await apply(page,delta(3,[{op:'children',id:'n2',children:['n3','n4'],nodes:[{id:'n4',parent:'n2',kind:'element',tag:'body'}]}],{document_id:'empty'}));
  await page.waitForFunction(()=>window.r.painted,{},{timeout:3000});
  assert.equal(await page.locator('#c > iframe:not([inert])').count(),1);
  assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.body.innerText),'');
 }finally{await page.close();}
});

// MP-08/MP-10/MP-11 review#1023: a transient refusal must allow the same visible selection again.
test('MP-11 a refused selection can be settled again without changing its range', async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'The witness has called'}])));
  const result=await page.evaluate(async()=>{
   let calls=0;window.r.send=async action=>{if(action.kind==='selection'&&++calls===1)throw Error('transient refusal')};
   const d=window.r.frame.contentDocument,t=d.querySelector('p').firstChild;d.getSelection().setBaseAndExtent(t,0,t,t.length);
   const first=await window.r.settleInput().then(()=>null,e=>e.message);
   const second=await window.r.settleInput().then(()=>null,e=>e.message);
   return {first,second,calls,range:d.getSelection().toString()};
  });
  assert.equal(result.first,'MP-10: mirror selection refused');assert.equal(result.second,null);assert.equal(result.calls,2);assert.equal(result.range,'The witness has called');
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11 review#1023: delayed continuation with a hidden body prefix.
test('MP-10 a hidden body prefix cannot time out over a held page',async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'Previous usable site'}])));
  await apply(page,{...snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'header',attrs:{hidden:''}},{id:'n5',parent:'n4',kind:'text',text:'Hidden prefix'}])),document_id:'large',sequence:2,snapshot_complete:false});
  await new Promise(resolve=>setTimeout(resolve,2300));
  assert.equal(await page.evaluate(()=>window.r.painted),false,'a pending hidden prefix is not a completed empty page');
  assert.equal(await page.locator('#c > iframe').count(),2);
  await apply(page,{...delta(3,[{op:'children',id:'n3',children:['n4','n6'],nodes:[{id:'n6',parent:'n3',kind:'element',tag:'p'},{id:'n7',parent:'n6',kind:'text',text:'Actual viewport'}]}],{document_id:'large'}),snapshot_complete:true});
  await page.waitForFunction(()=>window.r.painted);
  assert.equal(await page.locator('#c > iframe:not([inert])').count(),1);
  assert((await page.evaluate(()=>window.r.frame.contentDocument.body.innerText)).includes('Actual viewport'));
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11: completion can arrive in a header-only compact delta.
test('MP-10 completion after an empty prefix starts a bounded empty-page hold', async () => {
 const page=await viewer();try {
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'Previous site'}])));
  await apply(page,{...snapshot(page2([])),document_id:'empty',sequence:2,snapshot_complete:false});
  await new Promise(resolve=>setTimeout(resolve,2200));
  assert.equal(await page.evaluate(()=>window.r.painted),false);
  await apply(page,{...delta(3),document_id:'empty',snapshot_complete:true});
  await page.waitForFunction(()=>window.r.painted,null,{timeout:3000});
  assert.equal(await page.locator('#c > iframe:not([inert])').count(),1);
  assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.body.innerText),'');
 } finally { await page.close() }
});

// MP-08/MP-10/MP-11 review60e: no stale refusal when a viewer drops the text range.
for (const mode of ['collapsed','absent']) test(`MP-11 a refused selection is not sticky after ${mode}`, async()=>{
 const page=await viewer();try {
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'The witness has called'}])));
  const result=await page.evaluate(async mode=>{
   let calls=0;window.r.send=async action=>{if(action.kind==='selection'){calls++;throw Error('transient refusal')}};
   const d=window.r.frame.contentDocument,t=d.querySelector('p').firstChild,s=d.getSelection();s.setBaseAndExtent(t,0,t,t.length);
   const first=await window.r.settleInput().then(()=>null,e=>e.message);
   if(mode==='collapsed')s.collapse(t,0);else s.removeAllRanges();
   return {first,second:await window.r.settleInput().then(()=>null,e=>e.message),calls};
  },mode);
  assert.equal(result.first,'MP-10: mirror selection refused');assert.equal(result.second,null);assert.equal(result.calls,1);
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11: a refusal can arrive after the person has dropped the range.
test('MP-11 collapsed range is rechecked after a pending selection refusal',async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'element',tag:'p'},{id:'n5',parent:'n4',kind:'text',text:'The witness has called'}])));
  const result=await page.evaluate(async()=>{
   let reject;window.r.send=action=>action.kind==='selection'?new Promise((_resolve,r)=>{reject=r}):Promise.resolve();
   const d=window.r.frame.contentDocument,t=d.querySelector('p').firstChild,s=d.getSelection();s.setBaseAndExtent(t,0,t,t.length);
   const settled=window.r.settleInput().then(()=>null,e=>e.message);
   while(!reject)await new Promise(r=>setTimeout(r,0));s.collapse(t,0);reject(Error('transient refusal'));return settled;
  });
  assert.equal(result,null);
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11: an idle completed page must present without another reply.
test('MP-10 completed empty paint notifies the attached consumer without another packet', async()=>{
 const page=await viewer();try{
  await page.evaluate(async packet=>{
   window.r.close();window.paintNotifications=[];let first=true;
   window.attached=await Mirror2.attachBrowserMirror2({protocolVersion:550,request:async request=>{
    const command=request.KernelBrowser.command;
    if(command.op==='mirror_subscribe')return{KernelBrowser:{result:{subscription_id:'s'}}};
    if(command.op==='mirror_close')return{KernelBrowser:{result:{closed:true}}};
    if(first){first=false;return{KernelBrowser:{result:packet}}}
    return new Promise(()=>{});
   }},document.getElementById('c'),{tab_id:'t',generation:1,device_scale_factor:1},{failure:e=>{throw e},presented:packet=>window.paintNotifications.push({sequence:packet.sequence,painted:window.attached?.renderer.painted})});
  },{...snapshot(page2([])),nodes:[[1,0,1],[1,1,'html'],[1,1,'body']]} );
  await page.waitForFunction(()=>window.paintNotifications.some(n=>n.painted),null,{timeout:3000});
  assert.deepEqual(await page.evaluate(()=>window.paintNotifications.filter(n=>n.painted)),[{sequence:1,painted:true}]);
 }finally{await page.evaluate(()=>window.attached?.close());await page.close()}
});

// BROWSER-UX-09 (r45 probe, typed github.com repo at 8 Mbit/s): navigation refuses the long-polling credit;
// treating that refusal as a lost reply asked for a resend, which returned the new 538 KB reset a second time.
test('MP-10 a credit refused for a retired document is not resent', async()=>{
 const page=await viewer();try{
  const credits=await page.evaluate(async packet=>{
   window.r.close();const credits=[];let refused=false;
   const mirror=await Mirror2.attachBrowserMirror2({protocolVersion:550,request:async request=>{
    const command=request.KernelBrowser.command;
    if(command.op==='mirror_subscribe')return{KernelBrowser:{result:{subscription_id:'s'}}};
    if(command.op==='mirror_close')return{KernelBrowser:{result:{closed:true}}};
    credits.push({after:command.after_sequence,resend:command.resend===true});
    if(command.after_sequence===0)return{KernelBrowser:{result:packet}};
    if(!refused){refused=true;throw Error('MD-2: host browser operation failed; refresh state or check host browser readiness (Error stale_document_reference kernel-browser-mirror2.mjs:351:15)')}
    return new Promise(()=>{});
   }},document.getElementById('c'),{tab_id:'t',generation:1,device_scale_factor:1},{failure:e=>{throw e}});
   await new Promise(r=>setTimeout(r,1500));await mirror.close();return credits;
  },{...snapshot(page2([])),nodes:[[1,0,1],[1,1,'html'],[1,1,'body'],[1,1,0,'Old document']]});
  assert(credits.length>=3,'credits continue after the refusal');
  assert.deepEqual(credits.filter(c=>c.resend),[],'no resend for a refused (answered) credit');
 }finally{await page.close()}
});

for(const preserveFrame of [false,true])test(`MP-11 closed empty attachment never emits a deferred presentation preserve=${preserveFrame}`,async()=>{
 const page=await viewer();try{
  await page.evaluate(async({packet,preserveFrame})=>{
   window.r.close();window.presented=[];let first=true;
   const mirror=await Mirror2.attachBrowserMirror2({protocolVersion:550,request:async request=>{
    const command=request.KernelBrowser.command;
    if(command.op==='mirror_subscribe')return{KernelBrowser:{result:{subscription_id:'s'}}};
    if(command.op==='mirror_close')return{KernelBrowser:{result:{closed:true}}};
    if(first){first=false;return{KernelBrowser:{result:packet}}}return new Promise(()=>{});
   }},document.getElementById('c'),{tab_id:'t',generation:1,device_scale_factor:1},{failure:e=>{throw e},presented:p=>window.presented.push(p.sequence)});
   while(!mirror.renderer.frame.dataset.mirrorSequence)await new Promise(r=>setTimeout(r,10));await mirror.close({preserveFrame});
  },{packet:{...snapshot(page2([])),nodes:[[1,0,1],[1,1,'html'],[1,1,'body']]},preserveFrame});
  await new Promise(r=>setTimeout(r,2200));assert.deepEqual(await page.evaluate(()=>window.presented),[]);
 }finally{await page.close()}
});


test('MP-10: navigation paint diagnostics follow a revealed frame and a paint opportunity once', async () => {
  const page = await viewer();
  try {
    await page.evaluate(() => { window.paintMarks = []; window.r.onTiming = sample => window.paintMarks.push(sample) });
    await apply(page, snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' }, { id: 'n4', parent: 'n3', kind: 'text', text: 'Visible page' }]));
    await page.waitForFunction(() => window.r.timings.some(t => t.stage === 'mirror_navigation_paint'));
    await apply(page, delta(2));
    await frames();
    const marks = await page.evaluate(() => window.r.timings.filter(t => t.stage === 'mirror_navigation_paint'));
    assert.equal(marks.length, 1);
    const observed = await page.evaluate(() => window.paintMarks.filter(t => t.stage === 'mirror_navigation_paint'));
    assert.deepEqual(observed, marks, 'the host client can export the same paint mark without reading mirrored content');
    assert.ok(marks[0].duration_ms >= 0 && Number.isFinite(marks[0].ended_ms));
  } finally { await page.close(); }
});

test('MP-10 a same-site navigation reuses retained CSS without carrying its text again', async () => {
 const page=await viewer();
 try {
  const digest='a'.repeat(24),nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'head'},{id:'n4',parent:'n3',kind:'element',tag:'style',css_ref:digest},{id:'n5',parent:'n2',kind:'element',tag:'body'},{id:'n6',parent:'n5',kind:'element',tag:'p'},{id:'n7',parent:'n6',kind:'text',text:'First destination'}];
  await apply(page,{...snapshot(nodes),sheets:{[digest]:'p{color:rgb(12,34,56)}'}});
  assert.deepEqual(await page.evaluate(()=>window.r.knownSheets()),[digest]);
  await apply(page,{...snapshot(nodes.map(r=>r.id==='n7'?{...r,text:'Second destination'}:r)),document_id:'second',sequence:2});
  assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.querySelector('p').textContent),'Second destination');
  assert.equal(await page.evaluate(()=>window.r.frame.contentWindow.getComputedStyle(window.r.frame.contentDocument.querySelector('p')).color),'rgb(12, 34, 56)');
 }finally{await page.close();}
});

test('MP-10 image hydration keeps the existing font-face rule alive', async () => {
 const page=await viewer();
 try {
  const nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'head'},{id:'n4',parent:'n3',kind:'element',tag:'style',css:'@font-face{font-family:MirrorFont;src:url("mr:r1")}p{background-image:url("mr:r2")}'},{id:'n5',parent:'n2',kind:'element',tag:'body'},{id:'n6',parent:'n5',kind:'element',tag:'p'},{id:'n7',parent:'n6',kind:'text',text:'Visible destination'}];
  await apply(page,snapshot(nodes));
  await page.evaluate(()=>{const d=window.r.frame.contentDocument;window.fontFaceBefore=[...d.styleSheets].flatMap(s=>[...s.cssRules]).find(r=>r.type===5)});
  const data=Buffer.from([137,80,78,71,13,10,26,10]);
  await apply(page,delta(2,[],{resources:[{key:'r2',resource_id:createHash('sha256').update(data).digest('hex'),mime_type:'image/png',data_base64:data.toString('base64')}]}));
  assert.equal(await page.evaluate(()=>{const d=window.r.frame.contentDocument;return [...d.styleSheets].flatMap(s=>[...s.cssRules]).find(r=>r.type===5)===window.fontFaceBefore}),true,'unrelated image bytes must not reparse/restart the font face');
 }finally{await page.close();}
});

test('MP-10 next navigation consumes one preloaded inert sandbox and closes the spare', async () => {
 const page=await viewer();
 try {
  const nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},{id:'n4',parent:'n3',kind:'text',text:'First destination'}];
  await apply(page,snapshot(nodes));
  assert.equal(await page.evaluate(()=>{window.spare=document.querySelector('#c > iframe[inert]');return !!window.spare&&getComputedStyle(window.spare).display==='none'}),true);
  await apply(page,{...snapshot(nodes.map(r=>r.id==='n4'?{...r,text:'Next destination'}:r)),sequence:2,document_id:'next'});
  assert.equal(await page.evaluate(()=>window.r.frame===window.spare&&!window.r.frame.inert),true);
  assert.equal(await page.locator('#c > iframe:not([inert])').count(),1);
  await page.evaluate(()=>window.r.close());assert.equal(await page.locator('#c > iframe').count(),0);
 }finally{await page.close();}
});

test('MP-08/MP-10/MP-11 native preview holds old paint and focus until a complete prepared document is released',async()=>{
 const page=await viewer();
 try{
  const nodes=(text)=>[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},{id:'n4',parent:'n3',kind:'element',tag:'input',form:{value:text,checked:false,selected_index:-1,selection_start:text.length,selection_end:text.length}}];
  await apply(page,snapshot(nodes('old')));
  await page.evaluate(()=>{window.old=window.r.frame;window.r.holdForHandover()});
  await apply(page,{...snapshot(nodes('new')),document_id:'new',sequence:2,focused:'n4'});
  assert.deepEqual(await page.evaluate(()=>({oldConnected:window.old.isConnected,oldVisible:getComputedStyle(window.old).visibility,newVisible:getComputedStyle(window.r.frame).visibility,painted:window.r.painted,focus:document.activeElement===window.r.frame})),{oldConnected:true,oldVisible:'visible',newVisible:'hidden',painted:false,focus:false});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),true);
  await page.evaluate(()=>window.r.handover(true));
  assert.deepEqual(await page.evaluate(()=>({oldConnected:window.old.isConnected,newVisible:getComputedStyle(window.r.frame).visibility,painted:window.r.painted,value:window.r.frame.contentDocument.activeElement.value,selection:window.r.frame.contentDocument.activeElement.selectionStart})),{oldConnected:false,newVisible:'visible',painted:true,value:'new',selection:3});
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11: held continuation packets must not synchronously lay out
// the growing document before the handover can use it.
test('MP-08/MP-10/MP-11 native-held continuations defer repeated layout and retain final scroll',async()=>{
 const page=await viewer(),cdp=await page.context().newCDPSession(page);
 try{
  await apply(page,snapshot([{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'},{id:'n4',parent:'n3',kind:'text',text:'old page'}]));
  await page.evaluate(()=>{window.old=window.r.frame;window.r.holdForHandover()});
  const nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'body'}];
  for(let i=0;i<1000;i++)nodes.push({id:'n'+(4+i*2),parent:'n3',kind:'element',tag:'p'},{id:'n'+(5+i*2),parent:'n'+(4+i*2),kind:'text',text:'continued page '+i});
  nodes.push({id:'n2004',parent:'n3',kind:'element',tag:'div',attrs:{id:'staged-scroller',style:'height:80px;overflow:auto'},scroll:[0,400]},{id:'n2005',parent:'n2004',kind:'element',tag:'div',attrs:{style:'height:1000px'}});
  await apply(page,{...snapshot(nodes),document_id:'new',sequence:2,snapshot_complete:false});await frames();
  // Browser layout can move an inert scroller; it is not a new viewer action.
  await page.evaluate(()=>window.r.frame.contentDocument.getElementById('staged-scroller').scrollTop=10);await frames();
  await cdp.send('Performance.enable');const count=async()=>{const r=await cdp.send('Performance.getMetrics');return r.metrics.find(m=>m.name==='LayoutCount').value};const before=await count();
  const packets=Array.from({length:10},(_,i)=>({...delta(i+3,[{op:'attr',id:'n3',name:'style',value:'padding-left:'+i+'px'}]),document_id:'new',snapshot_complete:false}));
  await page.evaluate(async packets=>{for(const packet of packets)await window.r.apply(packet)},packets);
  const layouts=(await count())-before;
  assert(layouts<=2,`held continuation batch forced ${layouts} layouts; final geometry is not needed yet`);
  assert.deepEqual(await page.evaluate(()=>({oldConnected:window.old.isConnected,oldText:window.old.contentDocument.body.textContent,painted:window.r.painted,sent:window.sent.length})),{oldConnected:true,oldText:'old page',painted:false,sent:0});
  await apply(page,{...delta(13,[],{scroll:[0,500]}),document_id:'new',snapshot_complete:true});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),true);await page.evaluate(()=>window.r.handover());
  assert.equal(await page.evaluate(()=>window.r.frame.contentWindow.scrollY),500);assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.querySelector('p').textContent),'continued page 0');
  assert.equal(await page.evaluate(()=>window.r.frame.contentDocument.getElementById('staged-scroller').scrollTop),400);
 }finally{await cdp.detach();await page.close()}
});

// MP-08/MP-10: a visible image the page never loads (DuckDuckGo results favicons) holds the
// handover only briefly; the native preview must not stay up for the whole visit.
test('MP-08/MP-10 an undelivered visible image stops holding the handover after a bound',async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'old page'}])));
  await page.evaluate(()=>window.r.holdForHandover());
  await apply(page,{...snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'new page'},{id:'n5',parent:'n3',kind:'element',tag:'img',attrs:{width:'20',height:'20'},res:'r1'}])),document_id:'new',sequence:2});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),false,'a fresh document waits for its visible image');
  // Other images still arriving keep the hold (GitHub: handing over mid-stream raced the kernel window).
  const data=Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==','base64');
  await page.waitForTimeout(2500);
  await apply(page,{...delta(3,[]),document_id:'new',resources:[{key:'r2',resource_id:createHash('sha256').update(data).digest('hex'),mime_type:'image/png',data_base64:data.toString('base64')}]});
  await page.waitForTimeout(1000);
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),false,'resources still arriving keep the image hold');
  await page.waitForTimeout(2300);
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),true,'the image never arrives: hand over without it');
 }finally{await page.close()}
});

// c#368 @3c6b391d8 review: a visible CSS background whose image fails (404) held the switch forever.
test('MP-08/MP-10 an undelivered visible CSS image stops holding the handover after the same bound',async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'old page'}])));
  await page.evaluate(()=>window.r.holdForHandover());
  await apply(page,{...snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'new page'},{id:'n5',parent:'n3',kind:'element',tag:'div',attrs:{style:'display:block;width:20px;height:20px;background-image:url("mr:r1")'}}])),document_id:'new',sequence:2});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),false,'a fresh document waits for its visible CSS image');
  await page.waitForTimeout(3300);
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),true,'the CSS image never arrives: hand over without it');
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11: diagnose an actual held-page seam without logging content.
test('MP-08/MP-10/MP-11 handover reports a visible resource blocker without exposing its identity',async()=>{
 const page=await viewer();try{
  await page.evaluate(()=>{window.handoverMarks=[];window.r.onTiming=sample=>window.handoverMarks.push(sample)});
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'old page'}])));
  await page.evaluate(()=>window.r.holdForHandover());
  await apply(page,{...snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'new page'},{id:'n5',parent:'n3',kind:'element',tag:'img',attrs:{width:'20',height:'20'},res:'r1'}])),document_id:'new',sequence:2});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),false);
  const marks=await page.evaluate(()=>window.handoverMarks.filter(s=>s.stage.startsWith('handover_')));
  assert.deepEqual(marks.map(s=>s.stage),['handover_wait_image']);
  for(const mark of marks){assert.deepEqual(Object.keys(mark).sort(),['duration_ms','ended_ms','stage']);assert(Number.isFinite(mark.duration_ms)&&Number.isFinite(mark.ended_ms))}
  await page.evaluate(()=>window.r.readyForHandover());
  assert.equal(await page.evaluate(()=>window.handoverMarks.filter(s=>s.stage==='handover_wait_image').length),1,'unchanged blockers do not flood diagnostics');
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11: an unused font variant cannot hold a completed public page.
for(const usage of ['hidden','text','pseudo','control'])test(`MP-08/MP-10/MP-11 font readiness checks rendered usage=${usage}`,async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'old page'}])));await page.evaluate(()=>window.r.holdForHandover());
  const css='@font-face{font-family:MissingMirror;src:url("mr:r1")}@font-face{font-family:UnusedMirror;src:url("mr:r2")}body{font-family:Arial}'+(usage==='pseudo'?'p::before{content:"Generated text";font-family:MissingMirror}':'');
  const nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'head'},{id:'n4',parent:'n3',kind:'element',tag:'style',css},{id:'n5',parent:'n2',kind:'element',tag:'body'},{id:'n6',parent:'n5',kind:'element',tag:usage==='control'?'input':'p',attrs:usage==='text'||usage==='control'?{style:'font-family:MissingMirror'}:{},...(usage==='control'?{form:{value:'Visible input',checked:false,selected_index:-1,selection_start:0,selection_end:0}}:{})},...(usage==='control'?[]:[{id:'n7',parent:'n6',kind:'text',text:'Visible page'}])];
  await apply(page,{...snapshot(nodes),document_id:'font-page',sequence:2});
  await page.evaluate(async()=>{const d=window.r.frame.contentDocument;d.body.getBoundingClientRect();await d.fonts.load('16px MissingMirror').catch(()=>{});await d.fonts.ready});
  assert.equal(await page.evaluate(()=>[...window.r.frame.contentDocument.fonts].some(f=>f.status==='error')),true);
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),usage==='hidden','only fonts required by visible content constrain the switch');
  if(usage!=='hidden'){const canonical=JSON.stringify(['missingmirror','normal','400','100%','U+0-10FFFF']);await apply(page,{...delta(3,[],{font_failures:[createHash('sha256').update(canonical).digest('hex')]}),document_id:'font-page'});assert.equal(await page.evaluate(()=>window.r.readyForHandover()),true,'a certified native font failure renders the same fallback, rather than waiting forever')}
 }finally{await page.close()}
});

test('MP-08/MP-10/MP-11 native font requirements ignore a failed unused fallback, while pending or missing required fonts hold',async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'old page'}])));await page.evaluate(()=>window.r.holdForHandover());
  const css='@font-face{font-family:NativeFailed;src:url("mr:r1")}@font-face{font-family:UnusedFailed;src:url("mr:r2")}body{font-family:NativeFailed,Arial}';
  const nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'head'},{id:'n4',parent:'n3',kind:'element',tag:'style',css},{id:'n5',parent:'n2',kind:'element',tag:'body'},{id:'n6',parent:'n5',kind:'text',text:'Visible native fallback'}];
  await apply(page,{...snapshot(nodes),document_id:'requirements',sequence:2,ops:[{op:'fonts',id:'n1',required:null}]});
  await page.evaluate(async()=>{const d=window.r.frame.contentDocument;d.body.getBoundingClientRect();await Promise.all(['NativeFailed','UnusedFailed'].map(f=>d.fonts.load('16px '+f).catch(()=>{})));await d.fonts.ready});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),false,'pending native fonts hold');
  await apply(page,{...delta(3,[{op:'fonts',id:'n1',required:[]}]),document_id:'requirements'});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),true,'the source uses system fallback; an unused failed face does not hold forever');
  await apply(page,{...delta(4,[{op:'fonts',id:'n1',required:['a'.repeat(64)]}]),document_id:'requirements'});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),false,'a missing required font never becomes a fallback');
 }finally{await page.close()}
});

test('MP-08/MP-10/MP-11 native font requirements belong to each document and retire with its frame or reset',async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'old page'}])));await page.evaluate(()=>window.r.holdForHandover());
  const nodes=page2([{id:'n4',parent:'n3',kind:'text',text:'new page'},{id:'n5',parent:'n3',kind:'frame',tag:'iframe'},{id:'n6',parent:'n5',kind:'document'},{id:'n7',parent:'n6',kind:'element',tag:'html'},{id:'n8',parent:'n7',kind:'element',tag:'body'},{id:'n9',parent:'n8',kind:'text',text:'child'}]);
  await apply(page,{...snapshot(nodes),document_id:'requirements',sequence:2,ops:[{op:'fonts',id:'n1',required:[]}]});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),false,'the main document cannot certify its child');
  await apply(page,{...delta(3,[{op:'fonts',id:'n6',required:[]}]),document_id:'requirements'});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),true);
  await apply(page,{...delta(4,[{op:'fonts',id:'n6',required:null},{op:'children',id:'n3',children:['n4'],nodes:[]}]),document_id:'requirements'});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),true,'a removed frame cannot retain a pending font requirement');
  await apply(page,{...snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'fresh legacy document'}])),document_id:'fresh',sequence:5});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),true,'a reset retires the old certificate');
 }finally{await page.close()}
});

test('MP-08/MP-10/MP-11 required font bytes hydrate a previously failed face and preload an offscreen native face',async()=>{
 const page=await viewer();try{
  const bytes=await readFile(process.env.MIRROR_LAB_FONT??'/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf');
  const signature=createHash('sha256').update(JSON.stringify(['requiredmirror','normal','400','100%','U+0-10FFFF'])).digest('hex');
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'old page'}])));await page.evaluate(()=>window.r.holdForHandover());
  const nodes=[{id:'n1',parent:null,kind:'document'},{id:'n2',parent:'n1',kind:'element',tag:'html'},{id:'n3',parent:'n2',kind:'element',tag:'head'},{id:'n4',parent:'n3',kind:'element',tag:'style',css:'@font-face{font-family:RequiredMirror;src:url("mr:r1")}body{font-family:Arial}'},{id:'n5',parent:'n2',kind:'element',tag:'body'},{id:'n6',parent:'n5',kind:'text',text:'System font text'}];
  await apply(page,{...snapshot(nodes),document_id:'requirements',sequence:2,ops:[{op:'fonts',id:'n1',required:[signature]}]});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),false,'missing source-required bytes remain held');
  await apply(page,{...delta(3),document_id:'requirements',resources:[{key:'r1',resource_id:createHash('sha256').update(bytes).digest('hex'),mime_type:'font/ttf',data_base64:bytes.toString('base64')}]});
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),true,'materialized native font preloads even without local text using it');
  assert.equal(await page.evaluate(()=>[...window.r.frame.contentDocument.fonts][0].status),'loaded');
 }finally{await page.close()}
});

test('MP-08/MP-10/MP-11 a packet received during image decoding invalidates the font handover proof',async()=>{
 const page=await viewer();try{
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'old page'}])));await page.evaluate(()=>window.r.holdForHandover());
  await apply(page,{...snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'new page'},{id:'n5',parent:'n3',kind:'element',tag:'img',attrs:{width:'1',height:'1'}}])),document_id:'requirements',sequence:2,ops:[{op:'fonts',id:'n1',required:[]}]});
  await page.evaluate(()=>{const image=window.r.frame.contentDocument.querySelector('img');image.removeAttribute('src');image.setAttribute('src','data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aD1sAAAAASUVORK5CYII=');image.decode=()=>new Promise(resolve=>{window.finishDecode=resolve});window.proof=window.r.readyForHandover()});
  await page.waitForFunction(()=>!!window.finishDecode);
  await apply(page,{...delta(3,[{op:'attr',id:'n3',name:'class',value:'updated'}]),document_id:'requirements'});
  assert.equal(await page.evaluate(async()=>{window.finishDecode();return window.proof}),false,'retry against the applied packet instead of an earlier font proof');
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11: an explicit capability acknowledgement is fail-closed.
test('MP-08/MP-10/MP-11 negotiated font requirements cannot silently use legacy metadata',async()=>{
 const page=await viewer();try{
  await page.evaluate(()=>window.r.requireNativeFonts(true));
  await apply(page,snapshot(page2([{id:'n4',parent:'n3',kind:'text',text:'awaiting native certificate'}])));
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),false);
  await apply(page,delta(2,[{op:'fonts',id:'n1',required:[]}]));
  assert.equal(await page.evaluate(()=>window.r.readyForHandover()),true);
 }finally{await page.close()}
});

// MP-08/MP-10/MP-11: a standalone transparent canvas does not expose the web app theme.
for (const [name, css] of [['transparent', ''], ['dark', 'html{color-scheme:dark}'], ['author', 'html{background:rgb(231,17,51)}']]) test('MP-08/MP-10/MP-11 standalone canvas background ' + name, async () => {
  const page = await viewer(), native = await browser.newPage({ viewport: { width: 1280, height: 800 } });
  try {
    await native.setContent('<!doctype html><style>' + css + '</style><p>Visible text</p>');
    await page.evaluate(() => { document.body.style.cssText = 'margin:0;background:#ebebeb'; });
    await apply(page, snapshot([{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'head' }, { id: 'n4', parent: 'n3', kind: 'element', tag: 'style', css }, { id: 'n5', parent: 'n2', kind: 'element', tag: 'body' }, { id: 'n6', parent: 'n5', kind: 'element', tag: 'p' }, { id: 'n7', parent: 'n6', kind: 'text', text: 'Visible text' }]));
    const { decodePng } = await import('../../apps/kernel/slice-linux-docker/docker/kernel-browser-pixels.mjs');
    const pixel = async page => Array.from(decodePng((await page.screenshot({ clip: { x: 500, y: 100, width: 1, height: 1 } })).toString('base64')).pixels.slice(0, 3));
    assert.deepEqual(await pixel(page), await pixel(native), 'the viewer preserves the standalone source canvas, including source color-scheme and author backgrounds');
  } finally { await page.close(); await native.close(); }
});

// MP-08/MP-10/MP-11 (r73): a stack name the native machine lacks is shadowed, so the viewer lands on the same family as the
// native page even when only the viewer has the first one installed (GitHub: "Noto Sans" before Helvetica).
test('MP-08/MP-10/MP-11: a font the native machine lacks does not style the viewer copy', async () => {
  const page = await viewer();
  try {
    const nodes = stack => [{ id: 'n1', parent: null, kind: 'document' }, { id: 'n2', parent: 'n1', kind: 'element', tag: 'html' }, { id: 'n3', parent: 'n2', kind: 'element', tag: 'body' },
      { id: 'n4', parent: 'n3', kind: 'element', tag: 'span', attrs: { id: 'probe', style: `font:40px ${stack}` } }, { id: 'n5', parent: 'n4', kind: 'text', text: 'Hamburgefonstiv mmmmm 0123456789' }];
    const width = (selector = '#probe') => page.evaluate(selector => window.r.frame.contentDocument.querySelector(selector).getBoundingClientRect().width, selector);
    await apply(page, snapshot(nodes('"Liberation Serif", "Liberation Mono"')));
    const viewerOnly = await width();
    await apply(page, delta(2, [{ op: 'fonts', id: 'n1', required: [], absent: ["liberation serif"] }]));
    await frames(); const shadowed = await width();
    await apply(page, delta(3, [{ op: 'attr', id: 'n4', name: 'style', value: 'font:40px "Liberation Mono"' }]));
    const native = await width();
    assert.notEqual(viewerOnly, native, 'the fixture needs a viewer-only family to differ');
    assert.equal(shadowed, native, 'the shadowed name falls through to the next family');
    await apply(page, delta(4, [{ op: 'fonts', id: 'n1', required: [] }, { op: 'attr', id: 'n4', name: 'style', value: 'font:40px "Liberation Serif", "Liberation Mono"' }]));
    assert.equal(await width(), viewerOnly, 'a name no longer reported absent styles the copy again');
  } finally { await page.close(); }
});
