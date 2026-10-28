// MP-08/MP-09/MP-10/MP-11: real Excalidraw UI regressions with a fixture broker.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { resolve, extname, join } from 'node:path';
import { chromium } from 'playwright-core';
import { createViewActions } from '../../../../apps/kernel/slice-linux-docker/docker/browser-app-view-actions.mjs';

const output = process.env.CHARIOX_EXCALIDRAW_BUNDLE;
const evidence = process.env.CHARIOX_EXCALIDRAW_EVIDENCE;
assert(output && evidence, 'set external bundle and evidence paths');

async function fixture(run) {
  const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.woff2': 'font/woff2' };
  const server = createServer(async (req, res) => {
    try {
      const file = resolve(output, 'ui', '.' + new URL(req.url, 'http://localhost').pathname);
      if (!file.startsWith(resolve(output, 'ui') + '/')) throw Error('invalid path');
      res.setHeader('Content-Type', types[extname(file)] ?? 'application/octet-stream');
      res.end(await readFile(file));
    } catch { res.statusCode = 404; res.end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ executablePath: process.env.CHARIOX_TEST_CHROME ?? '/usr/bin/google-chrome', headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  try {
    const context = await browser.newContext({ viewport: { width: 1440, height: 900 } });
    const page = await context.newPage();
    page.setDefaultTimeout(5000);
    const broker = { scene: null, version: null, saveAttempts: 0, saves: [], fail: false, files: new Map(), writes: [], callers: [], picked: null };
    await context.exposeFunction('broker', async (name, input, token) => {
      broker.callers.push({name,token:token??null});
      if (name === 'load_scene') return { scene: JSON.parse(JSON.stringify(broker.scene, (_, value) =>
        value && !Array.isArray(value) && typeof value === 'object' ? Object.fromEntries(Object.entries(value).sort(([a], [b]) => a.localeCompare(b))) : value)), version: broker.version };
      if (name === 'save_scene') {
        broker.saveAttempts++;
        broker.saves.push(input);
        if (broker.fail) throw Error('Concurrent view changed the scene; reload.');
        if (input.version !== broker.version) throw Error('Concurrent view changed the scene; reload.');
        broker.scene = input.scene; broker.version = String(Number(broker.version) + 1);
        return { version: broker.version };
      }
      if (name === 'write_file') {
        broker.writes.push(input);
        broker.files.set(input.name, input.text);
        return { name: input.name, bytes: input.text.length, grantId: input.offer ? input.name : null };
      }
      if (name === 'pick_file') return { grants: broker.picked ? [broker.picked] : [] };
      if (name === 'read_file') return { name: input.name ?? input.grantId, scene: JSON.parse(broker.files.get(input.name ?? input.grantId)) };
      throw Error('unknown tool: ' + name);
    });
    await context.addInitScript(`const sdk = (${createViewActions.toString()})((...args) => window.broker(...args)); window.chariox = { action: sdk.action, view: sdk.view, call: (...args) => window.broker(...args) }; window.__charioxViewAgent = sdk.agent;`);
    await page.goto(`http://127.0.0.1:${server.address().port}/index.html`);
    await page.waitForFunction(() => window.__charioxViewAgent.registrations().length === 10);
    const invoke = (name, input = {}) => page.evaluate(({ name, input }) => window.__charioxViewAgent.invoke(name, input, 'agent-test'), { name, input });
    const draw = async (x = 300) => {
      await page.locator('.excalidraw__canvas.interactive').click({ position: { x: 1000, y: 650 } });
      await page.keyboard.press('Escape'); await page.keyboard.press('r');
      await page.waitForTimeout(100);
      await page.mouse.move(x, 200); await page.mouse.down();
      await page.mouse.move(x + 100, 280, { steps: 5 }); await page.waitForTimeout(100); await page.mouse.up();
      await page.waitForTimeout(450);
    };
    await mkdir(evidence, { recursive: true });
    const openView = async () => {
      const other = await context.newPage();
      other.setDefaultTimeout(5000);
      await other.goto(page.url());
      await other.waitForFunction(() => window.__charioxViewAgent.registrations().length === 10);
      return other;
    };
    await run({ page, broker, invoke, draw, openView });
  } finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
}

test('a failed action then a human edit reports once without trapping the canvas', { timeout: 30_000 }, () => fixture(async ({ page, broker, invoke, draw }) => {
  await draw();
  const before = broker.saveAttempts;
  broker.fail = true;
  const failed = await invoke('add_elements', { elements: [{ id: 'failed', type: 'rectangle', x: 500, y: 300 }] });
  assert.equal(failed.ok, false);
  assert.equal(broker.saveAttempts, before + 1);
  await draw(500);
  assert.equal(await page.locator('.Modal').count(), 0, 'a save failure must not block the canvas with a modal');
  const notice = page.locator('.Toast');
  await notice.waitFor();
  assert.match(await notice.innerText(), /Concurrent view/);
  await notice.locator('button').click();
  const changes = await page.evaluate(() => window.__charioxViewAgent.changes(0));
  await page.waitForTimeout(1100);
  assert.equal(await notice.count(), 0, 'closing the notice must not re-report the failure');
  assert.equal(broker.saveAttempts, before + 1, 'failed persistence must not start another broker save');
  const idle = await page.evaluate(() => window.__charioxViewAgent.changes(0));
  assert.equal(idle.value.latest, changes.value.latest, 'reporting and closing must not re-arm saves');
  await draw(700);
  await notice.waitFor();
  assert.match(await notice.innerText(), /Concurrent view/, 'a new edit after dismissal must report the active failure again');
  await notice.locator('button').click();
  const summary = await invoke('scene_summary');
  assert.equal(summary.value.total, 3, 'a person can keep editing after the failure');
  broker.fail = false;
  await page.reload();
  await page.waitForFunction(() => window.__charioxViewAgent.registrations().length === 10);
  await draw(900);
  assert(broker.saveAttempts > before + 1, 'reload/load_scene must recover automatic saving');
  await page.screenshot({ path: join(evidence, 'failure-recovery.png') });
}));

test('an oversized human paste has one dismissible notice and can be removed', { timeout: 30_000 }, () => fixture(async ({ page, broker }) => {
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);
  await page.locator('.excalidraw__canvas.interactive').click({ position: { x: 500, y: 400 } });
  await page.evaluate(() => navigator.clipboard.writeText(('x'.repeat(1000) + '\n').repeat(200)));
  await page.keyboard.press('Control+Shift+v');
  const notice = page.locator('.Toast');
  await page.locator('.Toast, .Modal').first().waitFor();
  assert.equal(await page.locator('.Modal').count(), 0);
  assert.match(await notice.innerText(), /192 KiB/);
  const attempts = broker.saveAttempts;
  await notice.locator('button').click();
  await page.waitForTimeout(1100);
  assert.equal(await notice.count(), 0);
  assert.equal(broker.saveAttempts, attempts, 'an invalid scene must never reach the broker');
  await page.locator('.excalidraw__canvas.interactive').click({ position: { x: 1000, y: 650 } });
  await page.keyboard.press('Control+a'); await page.keyboard.press('Delete');
  await page.waitForTimeout(500);
  assert(broker.saveAttempts > attempts, 'shrinking the scene must recover saving without reloading');
  await page.evaluate(() => navigator.clipboard.writeText(('x'.repeat(1000) + '\n').repeat(200)));
  await page.keyboard.press('Control+Shift+v');
  await notice.waitFor();
  assert.match(await notice.innerText(), /192 KiB/, 'a later oversized scene must report again after recovery');
  await page.screenshot({ path: join(evidence, 'oversized-recovery.png') });
}));

test('Open reports a sticky conflict on every explicit attempt', { timeout: 30_000 }, () => fixture(async ({ page, broker, draw }) => {
  await draw(); broker.fail = true; await draw(500);
  const notice = page.locator('.Toast');
  await notice.waitFor();
  for (let attempt = 0; attempt < 2; attempt++) {
    await notice.locator('button').click();
    await page.getByTestId('main-menu-trigger').click();
    await page.getByText('Open', { exact: true }).click();
    await notice.waitFor();
    assert.match(await notice.innerText(), /Concurrent view/);
  }
  await page.screenshot({ path: join(evidence, 'repeated-open-conflict.png') });
}));

test('a second oversized PNG export reports again', { timeout: 30_000 }, () => fixture(async ({ page, invoke }) => {
  await invoke('add_elements', { elements: [{ id: 'noise', type: 'text', x: 50, y: 50,
    text: Array.from({ length: 55 }, (_, i) => Array.from({ length: 85 }, (_, j) => String.fromCharCode(33 + (i * 37 + j * 19) % 90)).join('')).join('\n') }] });
  const notice = page.locator('.Toast');
  for (let attempt = 0; attempt < 2; attempt++) {
    await page.getByTestId('main-menu-trigger').click();
    await page.getByText('Export image', { exact: true }).click();
    await notice.waitFor();
    assert.match(await notice.innerText(), /144 KiB/);
    await notice.locator('button').click();
  }
  await page.screenshot({ path: join(evidence, 'repeated-oversized-export.png') });
}));

test('opening a picked diagram keeps the previous canvas recoverable with Undo', { timeout: 30_000 }, () => fixture(async ({ page, broker, invoke, draw }) => {
  await draw();
  const previous = (await invoke('scene_summary')).value.elements.map(element => element.id);
  const incoming = structuredClone(broker.scene);
  incoming.elements = incoming.elements.map(element => ({ ...element, id: 'imported', x: 600 }));
  broker.files.set('incoming.excalidraw', JSON.stringify(incoming));
  broker.picked = 'incoming.excalidraw';
  await page.keyboard.press('Control+o');
  await page.waitForFunction(() => window.__charioxViewAgent.state().then(result => result.value?.elements.some(element => element.id === 'imported')));
  await page.waitForTimeout(400);
  await page.locator('.excalidraw__canvas.interactive').click({ position: { x: 1100, y: 650 } });
  await page.keyboard.press('Control+z');
  await page.waitForTimeout(500);
  assert.deepEqual((await invoke('scene_summary')).value.elements.map(element => element.id), previous, 'Open must retain the previous diagram in native Undo');
  assert.deepEqual(broker.scene.elements.map(element => element.id), previous, 'Undo must restore the durable scene too');
  await page.reload();
  await page.waitForFunction(() => window.__charioxViewAgent.registrations().length === 10);
  assert.deepEqual((await invoke('scene_summary')).value.elements.map(element => element.id), previous);
  await page.screenshot({ path: join(evidence, 'open-undo-reload.png') });
}));

test('UI saves offer distinct copies that a person can reopen through Open', { timeout: 30_000 }, () => fixture(async ({ page, broker, invoke, draw }) => {
  await draw();
  await page.keyboard.press('Control+s'); await page.waitForTimeout(200);
  await page.keyboard.press('Control+s'); await page.waitForTimeout(200);
  await page.keyboard.press('Control+s'); await page.waitForTimeout(200);
  assert.equal(broker.writes.length, 3);
  const [first, second] = broker.writes;
  assert.equal(first.offer, true, 'Ctrl+S must offer the copy through files.export');
  assert.equal(second.offer, true);
  assert.notEqual(first.name, second.name, 'each unnamed save needs a distinct filename');
  assert.equal(new Set(broker.writes.map(write => write.name)).size, 3);
  assert(broker.writes.every(write => write.offer));
  assert.equal(broker.files.size, 3, 'offered copies remain available to their recipient');
  const saved = (await invoke('scene_summary')).value.elements;
  await draw(600);
  broker.picked = first.name;
  await page.keyboard.press('Control+o'); await page.waitForTimeout(500);
  assert.deepEqual((await invoke('scene_summary')).value.elements.map(element => element.id), saved.map(element => element.id), 'the exported copy reopens through the human picker');
  const named = await invoke('save_file', { name: 'explicit' });
  assert.equal(named.ok, true);
  assert.equal(named.value.name, 'explicit.excalidraw', 'agent-supplied private names retain their semantics');
  assert.equal(broker.writes.at(-1).offer, false);
  await page.screenshot({ path: join(evidence, 'save-copy-open.png') });
}));

test('selection, hover and scroll do not flood view.changed', { timeout: 30_000 }, () => fixture(async ({ page, broker, invoke, draw }) => {
  await draw();
  const id = (await invoke('scene_summary')).value.elements[0].id;
  const initial = await page.evaluate(() => window.__charioxViewAgent.changes(0));
  const attempts = broker.saveAttempts;
  for (let i = 0; i < 270; i++) await invoke('select_elements', { ids: i % 2 ? [id] : [] });
  for (let i = 0; i < 10; i++) {
    await page.mouse.move(500 + i * 10, 450);
    await page.mouse.wheel(0, 10);
  }
  await page.waitForTimeout(500);
  const afterUI = await page.evaluate(since => window.__charioxViewAgent.changes(since), initial.value.latest);
  assert.equal(afterUI.value.latest, initial.value.latest, 'only serialized scene changes may notify agents');
  assert.equal(afterUI.value.dropped, false, 'UI-only changes must not exhaust the 256-entry buffer');
  assert.equal(broker.saveAttempts, attempts);
  await draw(700);
  const edited = await page.evaluate(since => window.__charioxViewAgent.changes(since), initial.value.latest);
  assert(edited.value.latest > initial.value.latest, 'a real canvas edit must still notify');
  const settled = edited.value.latest;
  await page.waitForTimeout(600);
  assert.equal((await page.evaluate(since => window.__charioxViewAgent.changes(since), settled)).value.latest, settled, 'persisting an already announced scene must not notify twice');
}));

test('an unchanged loaded scene neither saves nor notifies', { timeout: 30_000 }, () => fixture(async ({ page, broker, draw }) => {
  await draw();
  const attempts = broker.saveAttempts, version = broker.version;
  const original = structuredClone(broker.scene);
  await page.reload();
  await page.waitForFunction(() => window.__charioxViewAgent.registrations().length === 10);
  await page.waitForTimeout(700);
  await writeFile(join(evidence, 'loaded-scenes.json'), JSON.stringify({ original, restored: broker.scene }));
  assert.equal(broker.saveAttempts, attempts, 'restoring an unchanged scene must not write');
  assert.equal(broker.version, version, 'an idle view must not bump the state version');
  assert.equal((await page.evaluate(() => window.__charioxViewAgent.changes(0))).value.latest, 0, 'initial restoration must not announce an edit');
  await page.screenshot({ path: join(evidence, 'unchanged-load.png') });
}));

test('opening an idle second view does not conflict with the editor', { timeout: 30_000 }, () => fixture(async ({ page, broker, draw, openView }) => {
  await draw();
  const attempts = broker.saveAttempts;
  const idle = await openView();
  await idle.waitForTimeout(700);
  const afterIdle = broker.saveAttempts;
  assert.equal(afterIdle, attempts, 'an idle second view must leave the saved version unchanged');
  assert.equal((await idle.evaluate(() => window.__charioxViewAgent.changes(0))).value.latest, 0);
  await page.bringToFront();
  await draw(600);
  await writeFile(join(evidence, 'two-view-saves.json'), JSON.stringify({ attempts, afterIdle, saves: broker.saves }));
  assert.equal(broker.saveAttempts, attempts + 1, 'only the editor should save');
  assert.equal(broker.scene.elements.length, 2, 'the editor must persist its next edit');
  assert.equal(await page.locator('.Toast').count(), 0, 'opening a view must not cause a conflict');
  await page.screenshot({ path: join(evidence, 'idle-view-editor.png') });
}));

test('selection and hover do not parse or normalize an unchanged scene', { timeout: 30_000 }, () => fixture(async ({ page, broker, draw }) => {
  await draw();
  await page.keyboard.press('Escape');
  await page.keyboard.press('v');
  await page.waitForTimeout(500);
  const before = broker.saveAttempts;
  const changes = await page.evaluate(() => window.__charioxViewAgent.changes(0));
  await page.evaluate(() => {
    const parse = JSON.parse, stringify = JSON.stringify;
    window.sceneWork = { parses: 0, normalizations: 0 };
    JSON.parse = function (...args) {
      const value = parse.apply(this, args);
      if (value?.type === 'excalidraw' && Array.isArray(value.elements) && value.appState && value.files) window.sceneWork.parses++;
      return value;
    };
    JSON.stringify = function (value, replacer, ...args) {
      if (value?.type === 'excalidraw' && typeof replacer === 'function') window.sceneWork.normalizations++;
      return stringify.call(this, value, replacer, ...args);
    };
  });
  // Select and deselect the existing rectangle, then hover over the canvas.
  await page.mouse.click(300, 240);
  await page.keyboard.press('Escape');
  await page.mouse.move(600, 400, { steps: 15 });
  await page.waitForTimeout(500);
  const work = await page.evaluate(() => window.sceneWork);
  await writeFile(join(evidence, 'unchanged-scene-work.json'), JSON.stringify(work));
  assert.equal(work.parses, 0, 'unchanged canvas events must skip full scene parsing');
  assert.equal(work.normalizations, 0, 'unchanged canvas events must skip key sorting and serialization');
  assert.equal(broker.saveAttempts, before);
  assert.equal((await page.evaluate(() => window.__charioxViewAgent.changes(0))).value.latest, changes.value.latest);
}));


test('agent persistence uses its caller while a later human save stays human', {timeout:30000}, () => fixture(async ({broker,invoke,draw}) => {
  broker.callers.length=0;
  assert.equal((await invoke('add_elements',{elements:[{id:'scoped',type:'rectangle',x:50,y:50}]})).ok,true);
  assert.equal((await invoke('save_file',{name:'scoped'})).ok,true);
  assert(broker.callers.some(call=>call.name==='save_scene'&&call.token==='agent-test'));
  assert(broker.callers.some(call=>call.name==='write_file'&&call.token==='agent-test'));
  broker.callers.length=0;
  await draw(600);
  assert(broker.callers.some(call=>call.name==='save_scene'&&call.token===null));
}));
