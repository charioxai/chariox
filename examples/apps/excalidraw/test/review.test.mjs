// The Excalidraw window's file handling and notices against a fake backend: copies, Open and the
// three exports go through the file tools, and what cannot be saved or exported is reported.
import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { openApp, closeEnvironment, labelled, rect, text, sceneOf, waitUntil } from './window.mjs';

after(closeEnvironment);

const idle = ms => new Promise(done => setTimeout(done, ms));
const file = (backend, name, scene) => backend.files.set(name, Buffer.from(JSON.stringify(scene)));

async function withApp(options, body) {
  const app = await openApp(options);
  try {
    await app.load();
    // Excalidraw handles keys only while it has focus: a click on empty canvas gives it focus.
    await app.page.mouse.click(1000, 750);
    await body(app);
    app.assertClean();
  } finally { await app.close(); }
}

const diagram = sceneOf([...labelled('client', 'Client', 100, 150), ...labelled('kernel', 'Kernel', 450, 150, 160, 80)]);

for (const dpr of [1, 2]) test(`copies, Open and the three exports go through the file tools at DPR${dpr}`, { timeout: 90_000 }, () => withApp({ dpr, scene: diagram }, async app => {
  const { backend, page } = app;
  const writes = name => backend.writes.filter(write => write.name === name);

  // Ctrl+S offers a copy under a name of its own, every time.
  await page.keyboard.press('Control+s');
  await waitUntil(() => backend.writes.length === 1, { what: 'the first copy' });
  await page.keyboard.press('Control+s');
  await waitUntil(() => backend.writes.length === 2, { what: 'the second copy' });
  const [first, second] = backend.writes;
  for (const write of [first, second]) {
    assert.match(write.name, /^diagram-[a-f0-9-]+\.excalidraw$/);
    assert.equal(write.offer, true, 'a copy is offered to the person');
    assert.equal(JSON.parse(write.text).elements.length, 4);
  }
  assert.notEqual(first.name, second.name);
  await app.toast.waitFor();
  assert.match(await app.toast.innerText(), /Saved copy: diagram-/);

  // Ctrl+O asks the picker; a cancelled pick changes nothing.
  await page.keyboard.press('Control+o');
  await waitUntil(() => backend.count('pick_file') === 1, { what: 'the picker' });
  await idle(500);
  assert.equal(backend.count('read_file'), 0, 'a cancelled pick reads nothing');
  assert.deepEqual(await app.ids(), ['client', 'client-text', 'kernel', 'kernel-text']);

  // The picked copy replaces what was drawn since.
  await page.mouse.click(1000, 750);
  await app.draw(300, 450);
  assert.equal((await app.elements()).length, 5);
  backend.pick(first.name);
  await page.mouse.click(1000, 750);
  await page.keyboard.press('Control+o');
  await app.until(elements => elements.length === 4, { what: 'the picked copy on the canvas' });
  assert.deepEqual(await app.ids(), ['client', 'client-text', 'kernel', 'kernel-text']);
  assert.equal(backend.count('read_file'), 1);
  assert.deepEqual(backend.live().map(element => element.id).sort(), ['client', 'client-text', 'kernel', 'kernel-text'], 'the opened diagram is saved too');

  // The menu: a copy, then the three exports.
  await app.menu('Save a copy');
  await waitUntil(() => backend.writes.filter(write => /^diagram-/.test(write.name)).length === 3, { what: 'the copy from the menu' });
  await app.menu('Export image');
  await waitUntil(() => writes('diagram.png').length === 1, { what: 'the PNG' });
  await app.menu('Export SVG');
  await waitUntil(() => writes('diagram.svg').length === 1, { what: 'the SVG' });
  const readings = writes('diagram.excalidraw').length; // reading the canvas exports JSON too
  await app.menu('Export JSON');
  await waitUntil(() => writes('diagram.excalidraw').length === readings + 1, { what: 'the JSON' });
  for (const name of ['diagram.png', 'diagram.svg', 'diagram.excalidraw']) assert.equal(writes(name).at(-1).offer, true, name);
  const png = backend.files.get('diagram.png');
  assert.equal(png.subarray(1, 4).toString(), 'PNG');
  assert(png.readUInt32BE(16) > 300 && png.readUInt32BE(20) > 50, 'a PNG of the diagram, not a blank');
  const svg = backend.files.get('diagram.svg').toString();
  assert(svg.includes('<svg') && svg.includes('Client') && svg.includes('Kernel'));
  const json = JSON.parse(backend.files.get('diagram.excalidraw').toString());
  assert.equal(json.type, 'excalidraw');
  assert.equal(json.elements.length, 4);
  assert.deepEqual(json.elements.filter(element => element.type === 'text').map(element => element.text).sort(), ['Client', 'Kernel']);
  await app.shot('exports');
  await writeEvidence(app, { png, svg, json: backend.files.get('diagram.excalidraw') });
}));

async function writeEvidence(app, files) {
  const dir = process.env.CHARIOX_EXCALIDRAW_EVIDENCE;
  for (const [kind, bytes] of Object.entries(files)) await writeFile(join(dir, `dpr${app.dpr}-diagram.${kind}`), bytes);
}

test('a picked diagram with Chinese text exports it as SVG', { timeout: 60_000 }, () => withApp({}, async app => {
  const { backend, page } = app;
  file(backend, 'cjk.excalidraw', sceneOf([text('cjk', '你好世界', 100, 100, { fontFamily: 5 })]));
  backend.pick('cjk.excalidraw');
  await page.keyboard.press('Control+o');
  await app.until(elements => elements.some(element => element.text === '你好世界'), { what: 'the picked diagram' });
  await app.menu('Export SVG');
  await waitUntil(() => backend.files.has('diagram.svg'), { what: 'the SVG' });
  assert(backend.files.get('diagram.svg').toString().includes('你好世界'));
}));

test('opening a picked diagram keeps the previous canvas recoverable with Undo', { timeout: 60_000 }, () => withApp({}, async app => {
  const { backend, page } = app;
  await app.draw(300, 200);
  await waitUntil(() => backend.live().length === 1, { what: 'the drawing to be saved' });
  const previous = backend.live().map(element => element.id);
  file(backend, 'incoming.excalidraw', sceneOf([rect('imported', 600, 300)]));
  backend.pick('incoming.excalidraw');
  await page.keyboard.press('Control+o');
  await app.until(elements => elements.length === 1 && elements[0].id === 'imported', { what: 'the picked diagram' });
  await waitUntil(() => backend.live().some(element => element.id === 'imported'), { what: 'the opened diagram to be saved' });
  await page.mouse.click(1100, 700);
  await page.keyboard.press('Control+z');
  await app.until(elements => elements.length === 1 && elements[0].id === previous[0], { what: 'Undo to restore the previous canvas' });
  await waitUntil(() => backend.live().length === 1 && backend.live()[0].id === previous[0], { what: 'Undo to be saved' });
  await page.reload();
  await app.canvas.waitFor();
  assert.deepEqual(await app.ids(), previous, 'the previous diagram is what a reload shows');
  await app.shot('open-undo-reload');
}));

test('a diagram that is too big for the broker has one dismissible notice and can be shrunk', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('existing', 100, 500)]),
}, async app => {
  const { backend, page } = app;
  const big = ('x'.repeat(1000) + '\n').repeat(200);
  await page.mouse.click(700, 300);
  await page.evaluate(value => navigator.clipboard.writeText(value), big);
  await page.keyboard.press('Control+Shift+v');
  await page.locator('.Toast, .Modal').first().waitFor();
  assert.equal(await page.locator('.Modal').count(), 0);
  assert.match(await app.toast.innerText(), /192 KiB/);
  await app.toast.locator('button').click();
  await idle(1200);
  assert.equal(await app.toast.count(), 0, 'a closed notice is not reported again');
  assert.equal(backend.count('save_scene'), 0, 'an invalid diagram must never reach the backend');

  await page.mouse.click(1000, 750);
  await page.keyboard.press('Control+a');
  await page.keyboard.press('Delete');
  await waitUntil(() => backend.saves.length === 1, { what: 'the shrunken diagram to be saved' });
  assert.deepEqual(backend.scene.elements, []);

  await page.evaluate(value => navigator.clipboard.writeText(value), big);
  await page.keyboard.press('Control+Shift+v');
  await app.toast.waitFor();
  assert.match(await app.toast.innerText(), /192 KiB/, 'a later oversized diagram is reported again');
  assert.equal(backend.saves.length, 1);
  await app.shot('oversized-recovery');
}));

test('Open and Save a copy report an automatic save that failed, every time', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([rect('existing', 100, 500)]),
}, async app => {
  const { backend, page } = app;
  backend.failSaves = 'The data directory is full.';
  await app.draw(300, 150);
  await app.toast.waitFor();
  await app.toast.locator('button').click();
  const reads = backend.count('pick_file');
  for (let attempt = 0; attempt < 2; attempt++) {
    await app.menu('Open');
    await app.toast.waitFor();
    assert.match(await app.toast.innerText(), /data directory is full/);
    await app.toast.locator('button').click();
  }
  assert.equal(backend.count('pick_file'), reads, 'Open does not ask for a file it could not save over');
  // A copy of what is on the canvas stays possible, and says what is wrong.
  await app.menu('Save a copy');
  await app.toast.waitFor();
  assert.match(await app.toast.innerText(), /Saved copy: diagram-.*reload to recover automatic saving/);
  assert.equal(JSON.parse(backend.writes.at(-1).text).elements.length, 2);
  await app.shot('open-conflict');
}));

test('a PNG that does not fit the broker is reported again on a second try', { timeout: 60_000 }, () => withApp({
  scene: sceneOf([text('noise', Array.from({ length: 55 }, (_, i) => Array.from({ length: 85 }, (_, j) => String.fromCharCode(33 + (i * 37 + j * 19) % 90)).join('')).join('\n'), 50, 50, { width: 935, height: 1375 })]),
}, async app => {
  const { backend } = app;
  for (let attempt = 0; attempt < 2; attempt++) {
    await app.menu('Export image');
    await app.toast.waitFor();
    assert.match(await app.toast.innerText(), /144 KiB/);
    await app.toast.locator('button').click();
  }
  assert.equal(backend.writes.length, 0);
  // SVG and JSON still work for it.
  await app.menu('Export SVG');
  await waitUntil(() => backend.files.has('diagram.svg'), { what: 'the SVG' });
  await app.shot('oversized-export');
}));

test('a dropped diagram goes through the picker and a dropped or pasted image is refused', { timeout: 60_000 }, () => withApp({}, async app => {
  const { backend, page } = app;
  const drop = (name, type) => page.evaluate(({ name, type }) => {
    const data = new DataTransfer();
    data.items.add(new File(['{}'], name, { type }));
    document.dispatchEvent(new DragEvent('drop', { dataTransfer: data, bubbles: true, cancelable: true }));
  }, { name, type });
  await drop('photo.png', 'image/png');
  await app.toast.waitFor();
  assert.match(await app.toast.innerText(), /Image insertion is not supported/);
  await app.toast.locator('button').click();
  assert.equal(backend.count('pick_file'), 0);
  await drop('plan.excalidraw', 'application/json');
  await waitUntil(() => backend.count('pick_file') === 1, { what: 'the picker' });
  await page.evaluate(() => {
    const data = new DataTransfer();
    data.items.add(new File(['x'], 'photo.png', { type: 'image/png' }));
    document.dispatchEvent(new ClipboardEvent('paste', { clipboardData: data, bubbles: true, cancelable: true }));
  });
  await app.toast.filter({ hasText: /Image insertion is not supported/ }).waitFor();
  assert.equal(backend.count('save_scene') <= 1, true);
}));
