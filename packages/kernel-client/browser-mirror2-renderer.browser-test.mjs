// MP-08/MP-10: the DOM mirror v2 renderer in real Chrome (the bundled source
// renderer applying protocol 489 packets in a headless viewer; no kernel).
// usage: MIRROR_LAB_NODE_MODULES=<Cloud apps/web/node_modules> node --test browser-mirror2-renderer.browser-test.mjs
import test, { before, after } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
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
const header = { wire: 2, subscription_id: 's', tab_id: 't', generation: 1, document_id: 'd', ops: [], scroll: [0, 0], focused: null, selection: null, resources: [], tiles: [], css_width: 1280, css_height: 800, device_scale_factor: 1 };
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
      assert.deepEqual(await sent(), [{ kind: 'selection', anchor_id: id, anchor_offset: anchor, focus_id: id, focus_offset: focus }, id === 'n4' ? { kind: 'text', text: 'x' } : { kind: 'key', key: 'Backspace' }], `MP-08: the ${gesture} range of ${id} reaches the kernel first`);
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
      await apply(page, delta(Number(await page.evaluate(() => window.r.sequence)) + 1, [{ op: 'form', id: 'n5', form: { selection_start: 4, selection_end: 4 } }])); await sent();
    }
  } finally { await page.close(); }
});
