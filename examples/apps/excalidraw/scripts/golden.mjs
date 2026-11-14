// Golden fixtures for the Rust port of the Excalidraw App's diagram actions.
//
// WHY: examples/apps/excalidraw/backend re-implements add_elements, connect_elements and update_elements
// (src/scene.js `diagramActions`) in pure Rust. Their output is only trustworthy if it matches the REAL code:
// Excalidraw 0.18.1's convertToExcalidrawElements / newElementWith and Chrome's canvas text metrics. This script
// produces that reference, so the backend tests compare against recorded truth instead of a second opinion.
//
// HOW: it bundles a tiny harness page (esbuild) that imports ../src/scene.js UNCHANGED plus the library, serves it
// from 127.0.0.1, loads the Virgil font the library ships, and runs the actions in headless Chrome against a minimal
// in-page fake `api` (scene store only). Every step of every case records the elements exactly as scene.js passed
// them to api.updateScene (live elements, in order), or the thrown error message. It also measures the per-character
// advances of Virgil and verifies them against real measureText and against the library's own text sizing.
//
// DETERMINISM: the page freezes Date.now (the library's `updated` and its seed generator start from it), replaces
// crypto.randomUUID by a counter ('uuid-1', 'uuid-2', ... per case; scene.js uses it for connect arrow ids and group
// ids) and crypto.getRandomValues by a fixed PRNG (the library's nanoid ids for bound text and its random id).
// Each case runs in a fresh page, so the random sequence of one case never depends on another. `seed`,
// `versionNonce`, `updated` and the nanoid ids of bound text are therefore reproducible but arbitrary: a port should
// ignore them (compare everything else; for a bound text compare by containerId, not by id).
//
// FONT: the script exits non-zero unless Virgil is really loaded: the font must be `loaded` in document.fonts and
// its measurements must differ from a fallback family. Every canvas font the library sets during measurement is
// recorded and must be `<size>px Virgil, Segoe UI Emoji` (what getFontString gives for fontFamily 1).
//
// REGENERATE:  node scripts/golden.mjs /absolute/scratch/dir      (scratch dir outside the checkout; fresh or reused)
//   run it through the bounded runner with NODE_OPTIONS=--max-old-space-size=4096. It rewrites
//   backend/tests/golden.json and backend/src/virgil.json (the backend's text metrics). Review the diff: a change means the library, Chrome or scene.js changed behaviour.
import { build } from 'esbuild';
import { chromium } from 'playwright-core';
import { createServer } from 'node:http';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const scratch = process.argv[2];
if (!scratch || !isAbsolute(scratch) || !relative(root, scratch).startsWith('..')) {
  throw Error('usage: node scripts/golden.mjs /absolute/scratch/dir (outside the checkout)');
}
const FONT = join(root, 'node_modules/@excalidraw/excalidraw/dist/prod/fonts/Virgil/Virgil-Regular.woff2');
const OUTPUT = join(root, 'backend/tests/golden.json');

// ---- cases -------------------------------------------------------------------------------------------------------
const add = (...elements) => ({ action: 'add_elements', input: { elements } });
const connect = (from, to, label) => ({ action: 'connect_elements', input: { from, to, ...(label === undefined ? {} : { label }) } });
const update = (input) => ({ action: 'update_elements', input });
const rect = (id, x, y, extra = {}) => ({ id, type: 'rectangle', x, y, ...extra });
const ellipse = (id, x, y, extra = {}) => ({ id, type: 'ellipse', x, y, ...extra });
const diamond = (id, x, y, extra = {}) => ({ id, type: 'diamond', x, y, ...extra });
const text = (id, x, y, extra = {}) => ({ id, type: 'text', x, y, ...extra });
const arrow = (id, x, y, extra = {}) => ({ id, type: 'arrow', x, y, ...extra });
const LONG = 'This is a long label that must wrap over several lines inside the box';
// Two labeled shapes with room for an arrow between them; the base of the connect and update cases.
const pair = [add(rect('r1', 0, 0, { label: 'Alpha' }), ellipse('e1', 400, 0, { label: 'Beta' }))];

const cases = [
  ['add_rectangle_label_default_size', [add(rect('r1', 10, 20, { label: 'Hello' }))]],
  ['add_ellipse_label', [add(ellipse('e1', 10, 20, { label: 'Server' }))]],
  ['add_diamond_label', [add(diamond('d1', 10, 20, { label: 'Yes?' }))]],
  ['add_rectangle_explicit_size_long_label_wraps', [add(rect('r1', 0, 0, { width: 120, height: 60, label: LONG }))]],
  ['add_rectangle_narrow_tall_long_label', [add(rect('r1', 0, 0, { width: 80, height: 300, label: LONG }))]],
  ['add_ellipse_long_label_wraps', [add(ellipse('e1', 0, 0, { width: 140, height: 100, label: LONG }))]],
  ['add_diamond_long_label_wraps', [add(diamond('d1', 0, 0, { width: 200, height: 140, label: LONG }))]],
  ['add_rectangle_label_taller_than_box', [add(rect('r1', 0, 0, { width: 200, height: 20, label: 'one\ntwo\nthree' }))]],
  ['add_rectangle_two_line_label', [add(rect('r1', 5, 5, { label: 'a\nb' }))]],
  ['add_rectangle_empty_label', [add(rect('r1', 5, 5, { label: '' }))]],
  ['add_rectangle_label_spaces_and_latin1', [add(rect('r1', 5, 5, { label: '  Café  naïve ü  ' }))]],
  ['add_rectangle_no_label', [add(rect('r1', 30, 40))]],
  ['add_rectangle_negative_position', [add(rect('r1', -50.5, -20.25, { width: 33.5, height: 17.25, label: 'neg' }))]],
  ['add_text_single_line', [add(text('t1', 100, 50, { text: 'Plain text' }))]],
  ['add_text_multiline', [add(text('t1', 100, 50, { text: 'first line\nsecond line is longer\nthird' }))]],
  ['add_text_from_label_only', [add(text('t1', 0, 0, { label: 'from label' }))]],
  ['add_text_without_text', [add(text('t1', 0, 0))]],
  ['add_arrow_default', [add(arrow('a1', 10, 10))]],
  ['add_arrow_end_point', [add(arrow('a1', 10, 10, { endX: 210, endY: 110 }))]],
  ['add_arrow_end_point_label', [add(arrow('a1', 10, 10, { endX: 260, endY: 60, label: 'flows' }))]],
  ['add_arrow_up_left_label', [add(arrow('a1', 300, 300, { endX: 100, endY: 120, label: 'back\nup' }))]],
  ['add_arrow_end_x_only', [add(arrow('a1', 10, 10, { endX: 50 }))]],
  ['add_arrow_end_y_only_vertical', [add(arrow('a1', 10, 10, { endX: 10, endY: 200 }))]],
  ['add_mixed_batch', [add(
    rect('r1', 0, 0, { label: 'Start' }), diamond('d1', 300, 0, { label: 'Ok?', width: 140, height: 100 }),
    ellipse('e1', 600, 0, { label: 'End', width: 120, height: 80 }), text('t1', 0, 200, { text: 'Note\nover two lines' }),
    arrow('a1', 160, 45, { endX: 300, endY: 50, label: 'go' }), arrow('a2', 440, 50, { endX: 600, endY: 40 }),
    rect('r2', 0, 300))]],
  ['add_second_call_keeps_first', [add(rect('r1', 0, 0, { label: 'one' })), add(ellipse('e1', 300, 0, { label: 'two' }))]],
  ['add_duplicate_id_error', [add(rect('r1', 0, 0)), add(rect('r1', 50, 50))]],
  ['add_duplicate_id_in_batch_error', [add(rect('r1', 0, 0), rect('r1', 50, 50))]],

  ['connect_rect_ellipse_right_no_label', [add(rect('r1', 0, 0), ellipse('e1', 400, 0)), connect('r1', 'e1')]],
  ['connect_rect_ellipse_right_with_label', [...pair, connect('r1', 'e1', 'sends')]],
  ['connect_rect_ellipse_below', [add(rect('r1', 0, 0, { label: 'Top' }), ellipse('e1', 0, 300, { label: 'Bottom' })), connect('r1', 'e1')]],
  ['connect_reversed_left', [add(rect('r1', 400, 0, { label: 'Right' }), ellipse('e1', 0, 0, { label: 'Left' })), connect('r1', 'e1', 'back')]],
  ['connect_reversed_up', [add(rect('r1', 0, 300, { label: 'Low' }), ellipse('e1', 0, 0, { label: 'High' })), connect('r1', 'e1')]],
  ['connect_diagonal_vertical_dominant', [add(rect('r1', 0, 300), ellipse('e1', 60, 0)), connect('r1', 'e1')]],
  ['connect_diagonal_tie_is_horizontal', [add(rect('r1', 0, 0, { width: 100, height: 100 }), rect('r2', 300, 300, { width: 100, height: 100 })), connect('r1', 'r2')]],
  ['connect_rect_diamond', [add(rect('r1', 0, 0, { label: 'Task' }), diamond('d1', 400, 0, { label: 'Done?' })), connect('r1', 'd1', 'check')]],
  ['connect_diamond_rect_no_label', [add(diamond('d1', 0, 0), rect('r1', 0, 250)), connect('d1', 'r1')]],
  ['connect_labeled_shapes_no_label', [...pair, connect('r1', 'e1')]],
  ['connect_chain_accumulates_bound_elements', [add(rect('r1', 0, 0), rect('r2', 300, 0), rect('r3', 600, 0)),
    connect('r1', 'r2'), connect('r2', 'r3'), connect('r1', 'r3', 'skip')]],
  ['connect_with_mixed_elements_present', [add(rect('r1', 0, 0), ellipse('e1', 300, 0), text('t1', 0, 200, { text: 'note' }), arrow('a1', 0, 300)), connect('e1', 'r1', 'x')]],
  ['connect_same_shape_error', [add(rect('r1', 0, 0)), connect('r1', 'r1')]],
  ['connect_text_element_error', [add(rect('r1', 0, 0), text('t1', 300, 0, { text: 'note' })), connect('r1', 't1')]],
  ['connect_arrow_element_error', [add(rect('r1', 0, 0), arrow('a1', 300, 0)), connect('r1', 'a1')]],
  ['connect_unknown_id_error', [add(rect('r1', 0, 0)), connect('r1', 'nope')]],

  ['update_move_labeled_rect_with_labeled_arrow', [...pair, connect('r1', 'e1', 'go'), update({ ids: ['r1'], dx: 30, dy: 20 })]],
  ['update_move_both_connected_shapes', [...pair, connect('r1', 'e1', 'go'), update({ ids: ['r1', 'e1'], dx: -25.5, dy: 60 })]],
  ['update_move_unlabeled_arrow_connection', [add(rect('r1', 0, 0), diamond('d1', 400, 0)), connect('r1', 'd1'), update({ ids: ['d1'], dy: 150 })]],
  ['update_move_labeled_rect_no_arrow', [add(rect('r1', 0, 0, { label: 'Move me' })), update({ ids: ['r1'], dx: 10, dy: -10 })]],
  ['update_move_only_dx', [add(rect('r1', 0, 0, { label: 'x' })), update({ ids: ['r1'], dx: 7 })]],
  ['update_move_zero', [add(rect('r1', 0, 0, { label: 'x' })), update({ ids: ['r1'] })]],
  ['update_resize_labeled_rect_rewraps', [add(rect('r1', 0, 0, { width: 300, height: 120, label: LONG })), update({ ids: ['r1'], width: 120, height: 60 })]],
  ['update_resize_labeled_rect_wider', [add(rect('r1', 0, 0, { width: 120, height: 60, label: LONG })), update({ ids: ['r1'], width: 400, height: 100 })]],
  ['update_resize_labeled_rect_width_only', [add(rect('r1', 0, 0, { label: 'Hello world' })), update({ ids: ['r1'], width: 60 })]],
  ['update_resize_labeled_rect_height_only', [add(rect('r1', 0, 0, { label: 'Hello world' })), update({ ids: ['r1'], height: 200 })]],
  ['update_resize_labeled_rect_same_size_with_move', [add(rect('r1', 0, 0, { label: 'Hello world' })), update({ ids: ['r1'], width: 160, height: 90, dx: 15, dy: 5 })]],
  ['update_resize_labeled_rect_smaller_than_text', [add(rect('r1', 0, 0, { label: 'Hello world' })), update({ ids: ['r1'], width: 20, height: 10 })]],
  ['update_resize_labeled_rect_and_move', [add(rect('r1', 0, 0, { label: 'Hello world' })), update({ ids: ['r1'], width: 220, height: 40, dx: 100, dy: 100 })]],
  ['update_resize_labeled_rect_two_line_label', [add(rect('r1', 0, 0, { label: 'a\nb' })), update({ ids: ['r1'], width: 50, height: 200 })]],
  ['update_resize_labeled_ellipse', [add(ellipse('e1', 0, 0, { width: 200, height: 120, label: LONG })), update({ ids: ['e1'], width: 100, height: 100 })]],
  ['update_resize_labeled_ellipse_small_label', [add(ellipse('e1', 0, 0, { label: 'Hi' })), update({ ids: ['e1'], width: 300, height: 200 })]],
  ['update_resize_labeled_diamond', [add(diamond('d1', 0, 0, { width: 240, height: 160, label: LONG })), update({ ids: ['d1'], width: 120, height: 90 })]],
  ['update_resize_labeled_diamond_small_label', [add(diamond('d1', 0, 0, { label: 'Hi' })), update({ ids: ['d1'], width: 300, height: 200 })]],
  ['update_resize_unlabeled_rect', [add(rect('r1', 10, 10)), update({ ids: ['r1'], width: 50, height: 70 })]],
  ['update_resize_unlabeled_ellipse_and_diamond', [add(ellipse('e1', 0, 0), diamond('d1', 300, 0)), update({ ids: ['e1', 'd1'], width: 77.5, height: 33.25 })]],
  ['update_resize_rect_with_connected_arrows', [...pair, connect('r1', 'e1', 'go'), update({ ids: ['r1'], width: 300, height: 200 })]],
  ['update_resize_ellipse_with_connected_arrow_no_label', [add(rect('r1', 0, 0), ellipse('e1', 400, 0, { label: 'Beta' })), connect('r1', 'e1'), update({ ids: ['e1'], width: 80, height: 240 })]],
  ['update_resize_diamond_with_connected_arrow', [add(rect('r1', 0, 0, { label: 'A' }), diamond('d1', 400, 0, { label: 'B' })), connect('r1', 'd1', 'x'), update({ ids: ['d1'], width: 260, height: 40 })]],
  ['update_move_chain_middle_shape', [add(rect('r1', 0, 0, { label: 'A' }), rect('r2', 300, 0, { label: 'B' }), rect('r3', 600, 0, { label: 'C' })),
    connect('r1', 'r2'), connect('r2', 'r3', 'next'), update({ ids: ['r2'], dx: 50, dy: 120 })]],
  ['update_move_plain_text', [add(text('t1', 10, 10, { text: 'Note\nover two lines' })), update({ ids: ['t1'], dx: 40, dy: 5 })]],
  ['update_move_bound_text_alone', [add(rect('r1', 0, 0, { label: 'Hello' })), update({ ids: ['r1__label'], dx: 5, dy: 5 })]],
  ['update_move_arrow_label_text_alone', [...pair, connect('r1', 'e1', 'go'), update({ ids: ['arrow-label'], dx: 5, dy: 5 })]],
  ['update_move_several_ids', [add(rect('r1', 0, 0, { label: 'A' }), ellipse('e1', 300, 0), text('t1', 0, 200, { text: 'T' }), diamond('d1', 300, 200, { label: 'D' })),
    update({ ids: ['r1', 'e1', 't1', 'd1'], dx: 11, dy: 22 })]],
  ['update_resize_text_error', [add(text('t1', 0, 0, { text: 'note' })), update({ ids: ['t1'], width: 100 })]],
  ['update_resize_text_height_error', [add(text('t1', 0, 0, { text: 'note' })), update({ ids: ['t1'], height: 100 })]],
  ['update_resize_text_with_shape_error', [add(text('t1', 0, 0, { text: 'note' }), rect('r1', 200, 0)), update({ ids: ['r1', 't1'], width: 100 })]],
  ['update_arrow_error', [add(arrow('a1', 0, 0)), update({ ids: ['a1'], dx: 10 })]],
  ['update_arrow_among_shapes_error', [add(rect('r1', 0, 0), arrow('a1', 200, 0)), update({ ids: ['r1', 'a1'], dx: 10 })]],
  ['update_unknown_id_error', [add(rect('r1', 0, 0)), update({ ids: ['r1', 'nope'], dx: 10 })]],
  ['chain_add_connect_move_resize', [add(rect('r1', 0, 0, { label: 'Start here' }), ellipse('e1', 400, 0, { label: 'Finish line' })), connect('r1', 'e1', 'then'),
    update({ ids: ['r1'], dx: 40, dy: 120 }), update({ ids: ['e1'], width: 100, height: 160 })]],
].map(([name, steps]) => ({ name, steps }));

// The bound text ids are the library's random ids; cases that need to address one use a placeholder that the page
// resolves to the id of the text bound to the named container: `<container>__label`, or `arrow-label` for the
// text bound to the first arrow of the scene.
const PLACEHOLDER_NOTE = 'Placeholders in update ids: "<id>__label" is the bound text of container <id>; "arrow-label" is the bound text of the first arrow.';

// ---- harness page ------------------------------------------------------------------------------------------------
const harness = `
import { convertToExcalidrawElements } from '@excalidraw/excalidraw';
import { diagramActions } from './src/scene.js';

const fontSet = new Set();
const fontDescriptor = Object.getOwnPropertyDescriptor(CanvasRenderingContext2D.prototype, 'font');
Object.defineProperty(CanvasRenderingContext2D.prototype, 'font', { ...fontDescriptor, set(value) { fontSet.add(value); fontDescriptor.set.call(this, value); } });
const measureCanvas = document.createElement('canvas').getContext('2d');
const measure = (font, text, kerning = 'auto') => { measureCanvas.font = font; measureCanvas.fontKerning = kerning; return measureCanvas.measureText(text).width; };

async function loadVirgil() {
  const face = new FontFace('Virgil', 'url(/Virgil-Regular.woff2) format("woff2")', { display: 'block', style: 'normal', weight: '400' });
  await face.load();
  document.fonts.add(face);
  await document.fonts.load('20px Virgil');
  const status = face.status;
  const reference = 'illiWmMwj.,!';
  const fallback = measure('20px NoSuchFamilyXyz, serif', reference);
  const virgil = measure('20px Virgil, Segoe UI Emoji', reference);
  return { status, checked: document.fonts.check('20px Virgil'), loaded: [...document.fonts].filter(f => f.family.replace(/["']/g, '') === 'Virgil' && f.status === 'loaded').length,
    virgilWidth: virgil, fallbackWidth: fallback, differsFromFallback: virgil !== fallback };
}

function fakeApi() {
  let scene = [];
  const updates = [];
  return {
    updates,
    getSceneElements: () => scene.filter(e => !e.isDeleted),
    getAppState: () => ({ selectedElementIds: {} }),
    getFiles: () => ({}),
    updateScene: ({ elements }) => { if (elements) { scene = elements; updates.push(JSON.stringify(elements.filter(e => !e.isDeleted))); } },
    scrollToContent: () => {},
  };
}

function resolveIds(ids, scene) {
  return ids.map(id => {
    if (id.endsWith('__label')) return scene.find(e => e.type === 'text' && e.containerId === id.slice(0, -7)).id;
    if (id === 'arrow-label') { const arrowId = scene.find(e => e.type === 'arrow').id; return scene.find(e => e.type === 'text' && e.containerId === arrowId).id; }
    return id;
  });
}

window.runCase = async steps => {
  const api = fakeApi();
  const actions = diagramActions(api, async candidate => candidate);
  const results = [];
  for (const step of steps) {
    const before = api.getSceneElements();
    const input = step.input.ids ? { ...step.input, ids: resolveIds(step.input.ids, before) } : step.input;
    fontSet.clear();
    const count = api.updates.length;
    try {
      await actions[step.action](structuredClone(input));
      if (api.updates.length !== count + 1) throw Error('expected exactly one updateScene call, saw ' + (api.updates.length - count));
      results.push({ elements: JSON.parse(api.updates.at(-1)), fonts: [...fontSet] });
    } catch (error) {
      results.push({ error: error.message, elements: JSON.parse(JSON.stringify(api.getSceneElements())), fonts: [...fontSet] });
    }
    results.at(-1).resolvedInput = input;
  }
  return results;
};

const CHARS = [];
for (let c = 0x20; c <= 0x7e; c++) CHARS.push(String.fromCharCode(c));
for (let c = 0xa0; c <= 0xff; c++) CHARS.push(String.fromCharCode(c));
window.virgilMetrics = () => {
  const advances = {}, missing = [];
  for (const ch of CHARS) {
    const withMono = measure('100px Virgil, monospace', ch), withSerif = measure('100px Virgil, serif', ch);
    if (withMono !== withSerif) { missing.push(ch); continue; }
    advances[ch] = measure('100px Virgil, Segoe UI Emoji', ch);
  }
  return { advances, missing };
};

window.verify = (samples, advances) => {
  const rows = [];
  for (const { text, fontSize } of samples) {
    const font = fontSize + 'px Virgil, Segoe UI Emoji';
    const lines = text.split('\\n');
    const predicted = Math.max(...lines.map(l => [...l].reduce((sum, ch) => sum + advances[ch] * fontSize / 100, 0)));
    const measured = Math.max(...lines.map(l => measure(font, l)));
    const noKerning = Math.max(...lines.map(l => measure(font, l, 'none')));
    fontSet.clear();
    const [element] = convertToExcalidrawElements([{ id: 'v', type: 'text', x: 0, y: 0, text, fontSize, fontFamily: 1 }], { regenerateIds: false });
    rows.push({ text, fontSize, measured, predicted, noKerning, elementWidth: element.width, elementHeight: element.height,
      expectedHeight: lines.length * fontSize * 1.25, fonts: [...fontSet] });
  }
  return rows;
};

// Deterministic fonts, ids and time must be installed before the library runs; the page does it in an init script.
window.harnessReady = loadVirgil();
`;

const initScript = `
(() => {
  Date.now = () => 1700000000000;
  let uuid = 0;
  window.resetUuid = () => { uuid = 0; };
  crypto.randomUUID = () => 'uuid-' + (++uuid);
  let state = 0x9e3779b9;
  const next = () => { state ^= state << 13; state >>>= 0; state ^= state >>> 17; state ^= state << 5; state >>>= 0; return state; };
  crypto.getRandomValues = array => { for (let i = 0; i < array.length; i++) array[i] = next() & 0xff; return array; };
})();
`;

const html = '<!doctype html><meta charset="utf-8"><body><script type="module" src="/harness.js"></script></body>';

// ---- run ---------------------------------------------------------------------------------------------------------
await mkdir(scratch, { recursive: true });
const bundlePath = join(scratch, 'harness.js');
await build({
  absWorkingDir: root, stdin: { contents: harness, resolveDir: root, sourcefile: 'harness.js' },
  outfile: bundlePath, bundle: true, format: 'esm', target: 'es2022', minify: false, conditions: ['production'],
  define: { 'process.env.NODE_ENV': '"production"' }, logLevel: 'error',
  plugins: [{ name: 'no-mermaid', setup(b) { b.onResolve({ filter: /^@excalidraw\/mermaid-to-excalidraw$/ }, () => ({ path: join(root, 'src/mermaid.js') })); } }],
});
const files = { '/': ['text/html', Buffer.from(html)], '/harness.js': ['text/javascript', await readFile(bundlePath)],
  '/Virgil-Regular.woff2': ['font/woff2', await readFile(FONT)] };
const server = createServer((req, res) => {
  const hit = files[new URL(req.url, 'http://127.0.0.1').pathname];
  if (!hit) { res.statusCode = 404; res.end(); return; }
  res.setHeader('Content-Type', hit[0]); res.end(hit[1]);
});
await new Promise(done => server.listen(0, '127.0.0.1', done));
const origin = `http://127.0.0.1:${server.address().port}/`;
const browser = await chromium.launch({ executablePath: '/usr/bin/google-chrome', headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
const version = browser.version();

async function freshPage() {
  const context = await browser.newContext({ viewport: { width: 800, height: 600 }, deviceScaleFactor: 1 });
  await context.addInitScript(initScript);
  const page = await context.newPage();
  const problems = [];
  page.on('pageerror', error => problems.push(error.message));
  await page.goto(origin);
  const font = await page.evaluate(() => window.harnessReady);
  if (font.status !== 'loaded' || !font.checked || font.loaded < 1 || !font.differsFromFallback) {
    throw Error('Virgil did not load: ' + JSON.stringify(font));
  }
  return { page, context, font, problems };
}

let exitCode = 0;
try {
  const golden = { virgil: null, cases: [] };
  // Metrics and verification run in their own page.
  {
    const { page, context, font, problems } = await freshPage();
    const { advances, missing } = await page.evaluate(() => window.virgilMetrics());
    const samples = [
      ['Hello', 20], ['Hello world', 16], ['Hello world', 28], ['The quick brown fox jumps over the lazy dog', 20],
      ['AVATAR Wavy To', 20], ['To Wa Yo Te', 28], ['fi fl ffi', 20], ['a\nb', 20], ['first line\nsecond line is longer\nthird', 16],
      ['iiiiiiiiii', 20], ['WWWWWWWWWW', 20], ['Server', 28], ['Yes?', 16], ['Alpha', 20], ['Beta', 28],
      ['0123456789', 20], ['!@#$%^&*()_+-=[]{}|;:\'",.<>/?`~', 20], ['  leading and trailing  ', 20], ['Café naïve', 20],
      ['Line one\nLine two\nLine three\nLine four', 28], ['a rather long arrow label', 16], ['Hello, World! 123', 28],
      ['lowercase only text here', 16], ['UPPERCASE ONLY TEXT HERE', 20], ['Mixed Case With Numbers 42', 28],
      ['x', 16], ['x y z', 20], ['This is a long label that must wrap over several lines inside the box', 20],
      ['Task\nDone?', 28], ['go', 20],
    ].map(([text, fontSize]) => ({ text, fontSize }));
    const rows = await page.evaluate(([s, a]) => window.verify(s, a), [samples, advances]);
    const wrongFont = rows.filter(r => r.fonts.some(f => !/^\d+(\.\d+)?px Virgil, Segoe UI Emoji$/.test(f)));
    if (wrongFont.length || rows.some(r => !r.fonts.length)) throw Error('The library measured with an unexpected canvas font: ' + JSON.stringify(rows.map(r => r.fonts)));
    const err = (a, b) => Math.abs(a - b);
    const verification = {
      samples: rows.map(r => ({ text: r.text, fontSize: r.fontSize, measureText: r.measured, sumOfAdvances: r.predicted, measureTextNoKerning: r.noKerning,
        libraryElementWidth: r.elementWidth, libraryElementHeight: r.elementHeight })),
      maxAbsErrorSumVsMeasureText: Math.max(...rows.map(r => err(r.measured, r.predicted))),
      maxAbsErrorSumVsLibraryElementWidth: Math.max(...rows.map(r => err(r.elementWidth, r.predicted))),
      maxAbsErrorMeasureTextVsLibraryElementWidth: Math.max(...rows.map(r => err(r.elementWidth, r.measured))),
      maxAbsErrorKerningOnVsOff: Math.max(...rows.map(r => err(r.measured, r.noKerning))),
      maxAbsErrorHeightVsLinesTimesFontSizeTimes125: Math.max(...rows.map(r => err(r.elementHeight, r.expectedHeight))),
    };
    golden.virgil = {
      unitsPerEm: 'px100',
      advances,
      lineHeight: 1.25,
      notes: 'advances[ch] is the canvas measureText width of the single character at fontSize 100 with the library canvas font "<size>px Virgil, Segoe UI Emoji" ' +
        '(Excalidraw 0.18.1 getFontString for fontFamily 1), in px; scale by fontSize/100. ' +
        'Characters outside the table (' + JSON.stringify(missing.join('')) + ' and anything beyond Latin-1) are not in Virgil and would use a fallback font, so they are not tabulated. ' +
        'Chrome ' + version + ', device scale factor 1. See "verification".',
      verification,
    };
    if (problems.length) throw Error('Page errors: ' + problems.slice(0, 3).join('; '));
    await context.close();
  }
  for (const { name, steps } of cases) {
    const { page, context, problems } = await freshPage();
    const results = await page.evaluate(runSteps => window.runCase(runSteps), steps);
    if (problems.length) throw Error(`Page errors in ${name}: ` + problems.slice(0, 3).join('; '));
    const record = [];
    results.forEach((result, i) => {
      const wrongFont = result.fonts.filter(f => !/^\d+(\.\d+)?px Virgil, Segoe UI Emoji$/.test(f));
      if (wrongFont.length) throw Error(`${name} step ${i}: unexpected canvas font ${JSON.stringify(wrongFont)}`);
      record.push({ action: steps[i].action, input: result.resolvedInput, ...(result.error === undefined ? {} : { error: result.error }), elements: result.elements });
    });
    golden.cases.push({ name, steps: record });
    await context.close();
  }
  golden.note = PLACEHOLDER_NOTE.replace('Placeholders in update ids', 'update ids in "input" are already resolved to real ids; the runner used placeholders for bound text') +
    ' Ignore seed, versionNonce, updated and the ids of bound text (library random ids; reproducible here only because the page fixes the PRNG and Date.now).';
  await mkdir(dirname(OUTPUT), { recursive: true });
  const text = '{\n"note": ' + JSON.stringify(golden.note) + ',\n"virgil": ' + JSON.stringify(golden.virgil, null, 1) + ',\n"cases": [\n' +
    golden.cases.map(c => '{"name": ' + JSON.stringify(c.name) + ', "steps": [\n' + c.steps.map(s => '{"action": ' + JSON.stringify(s.action) + ', "input": ' + JSON.stringify(s.input) +
      (s.error === undefined ? '' : ', "error": ' + JSON.stringify(s.error)) + ', "elements": [\n' + s.elements.map(e => JSON.stringify(e)).join(',\n') + '\n]}').join(',\n') + '\n]}').join(',\n') + '\n]\n}\n';
  await writeFile(OUTPUT, text);
  // The runtime table of the backend's text metrics: the advances at a font size of 100; `n` stands in for characters the face lacks.
  const { advances } = golden.virgil;
  await writeFile(join(root, 'backend/src/virgil.json'), JSON.stringify({ measuredAt: 100, fallback: advances.n, advances }));
  console.log(`wrote ${OUTPUT}: ${golden.cases.length} cases, ${text.length} bytes, Chrome ${version}`);
  console.log('max abs error sum-of-advances vs measureText:', golden.virgil.verification.maxAbsErrorSumVsMeasureText);
} catch (error) {
  console.error(error.message);
  exitCode = 1;
} finally {
  await browser.close();
  server.close();
}
process.exitCode = exitCode;
