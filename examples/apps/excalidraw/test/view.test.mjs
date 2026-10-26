// MP-08/MP-10/MP-11: real Excalidraw/A1 browser regression. The storage adapter
// is a fixture; this does not claim the kernel/provider acceptance legs.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { join, resolve, extname } from 'node:path';
import { chromium } from 'playwright-core';
import { createViewActions } from '../../../../apps/kernel/slice-linux-docker/docker/browser-app-view-actions.mjs';

const output = process.env.CHARIOX_EXCALIDRAW_BUNDLE;
const evidence = process.env.CHARIOX_EXCALIDRAW_EVIDENCE;
assert(output && evidence, 'set CHARIOX_EXCALIDRAW_BUNDLE and CHARIOX_EXCALIDRAW_EVIDENCE outside the repository');
const manifest = JSON.parse(await readFile(new URL('../schemas/ui-actions.json', import.meta.url)));

test('canvas, declarations, persistence, native shortcuts and three exports round trip at DPR1 and DPR2', { timeout: 120_000 }, async () => {
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
  const origin = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ executablePath: process.env.CHARIOX_TEST_CHROME ?? '/usr/bin/google-chrome', headless: true, args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  try {
    for (const dpr of [1, 2]) {
      const context = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: dpr });
      await context.grantPermissions(['clipboard-read', 'clipboard-write']);
      const page = await context.newPage();
      const errors = [], network = [], calls = [];
      page.on('pageerror', error => errors.push(error.message));
      await context.route('**/*', route => {
        if (route.request().url().startsWith(origin)) return route.continue();
        network.push(route.request().url());
        return route.abort();
      });
      let scene = null, version = null, failSave = false, holdSave = null, saveStarted;
      const files = new Map();
      await page.exposeFunction('broker', async (name, input) => {
        calls.push(name);
        if (name === 'load_scene') return { scene, version };
        if (name === 'save_scene') {
          if (holdSave && input.scene.elements.some(element => element.id === 'stale-action')) { saveStarted(); await holdSave; }
          if (failSave) { failSave = false; throw Error('Concurrent view changed the scene; reload.'); }
          assert.equal(input.version, version);
          scene = input.scene; version = String(Number(version) + 1);
          return { version };
        }
        if (name === 'write_file') {
          const bytes = input.base64 ? Buffer.from(input.base64, 'base64') : Buffer.from(input.text);
          files.set(input.name, bytes);
          return { name: input.name, bytes: bytes.length, grantId: input.offer ? 'export-fixture' : null };
        }
        if (name === 'read_file') return { name: input.name, scene: JSON.parse(files.get(input.name).toString()) };
        if (name === 'pick_file') return { grants: [] };
        throw Error('unknown backend tool: ' + name);
      });
      await page.addInitScript(`const sdk = (${createViewActions.toString()})(); window.chariox = { action: sdk.action, view: sdk.view, call: (...args) => window.broker(...args) }; window.__charioxViewAgent = sdk.agent;`);
      await page.goto(origin + '/index.html');
      await page.waitForFunction(() => window.__charioxViewAgent.registrations().length === 10);
      const invoke = async (name, input = {}) => {
        const result = await page.evaluate(async ({ name, input }) => window.__charioxViewAgent.invoke(name, input), { name, input });
        assert.equal(result.ok, true, JSON.stringify(result)); return result.value;
      };
      const registrations = await page.evaluate(() => window.__charioxViewAgent.registrations());
      assert.deepEqual(registrations, manifest.uiActions);
      const parallel = await page.evaluate(() => Promise.all(['parallel-a', 'parallel-b'].map(id => window.__charioxViewAgent.invoke('add_elements', { elements: [{ id, type: 'rectangle', x: 20, y: 20 }] }))));
      assert(parallel.every(result => result.ok));
      assert.equal((await invoke('scene_summary')).total, 2, 'overlapping actions must retain both additions');
      await invoke('delete_elements', { ids: ['parallel-a', 'parallel-b'] });
      await page.waitForSelector('.excalidraw__canvas.interactive');
      // A person draws using Excalidraw's real toolbar/keys and pointer path.
      await page.keyboard.press('r');
      await page.mouse.move(80, 160); await page.mouse.down(); await page.mouse.move(240, 250, { steps: 5 }); await page.waitForTimeout(100); await page.mouse.up();
      await page.waitForFunction(() => window.__charioxViewAgent.state().then(r => r.value?.total > 0));
      await invoke('add_elements', { elements: [
        { id: 'client', type: 'rectangle', x: 300, y: 200, label: 'Client' },
        { id: 'kernel', type: 'ellipse', x: 650, y: 200, label: 'Kernel' },
      ] });
      await invoke('connect_elements', { from: 'client', to: 'kernel', label: 'Actions' });
      const drawn = await invoke('scene_summary');
      assert.equal(drawn.elements.filter(e => e.type === 'text').length, 3);
      assert(drawn.elements.some(e => e.type === 'arrow' && e.from === 'client' && e.to === 'kernel'));
      await invoke('update_elements', { ids: ['client'], dx: 30, dy: 10, width: 190 });
      await invoke('group_elements', { ids: ['client', 'kernel'] });
      const moved = await invoke('select_elements', { ids: ['client', 'kernel'] });
      assert.equal(moved.elements.find(e => e.id === 'client').x, 330);
      assert.equal(moved.elements.find(e => e.id === 'client').width, 190);
      assert(moved.elements.find(e => e.type === 'arrow').x > 520, 'arrow must start outside the moved box and its label');
      assert.deepEqual(moved.elements.find(e => e.id === 'client').groups, moved.elements.find(e => e.id === 'kernel').groups);
      await invoke('save_file', { name: 'roundtrip' });
      for (const format of ['json', 'svg', 'png']) await invoke('export_file', { format, name: 'proof' });
      const json = JSON.parse(files.get('proof.excalidraw'));
      assert.equal(json.elements.length, moved.total);
      assert.deepEqual(json.elements.filter(e => e.type === 'text').map(e => e.text).sort(), ['Actions', 'Client', 'Kernel']);
      const svg = files.get('proof.svg').toString();
      assert(svg.includes('<svg') && svg.includes('Client') && svg.includes('Kernel'));
      const png = files.get('proof.png');
      assert.equal(png.subarray(1, 4).toString(), 'PNG');
      assert(png.readUInt32BE(16) > 300 && png.readUInt32BE(20) > 50);
      const cjk = structuredClone(json);
      const text = cjk.elements.find(element => element.type === 'text');
      cjk.elements = [{ ...text, id: 'cjk', text: '你好世界', originalText: '你好世界', fontFamily: 5, containerId: null, boundElements: null, groupIds: [], x: 100, y: 100 }];
      files.set('cjk.excalidraw', Buffer.from(JSON.stringify(cjk)));
      await invoke('open_file', { name: 'cjk' });
      await invoke('export_file', { format: 'svg', name: 'cjk' });
      assert(files.get('cjk.svg').toString().includes('你好世界'));
      await page.locator('.excalidraw__canvas.interactive').click({ position: { x: 50, y: 450 } });
      await page.evaluate(() => navigator.clipboard.writeText('flowchart LR\nA --> B'));
      await page.keyboard.press('Control+v');
      await page.waitForFunction(() => window.__charioxViewAgent.state().then(result => result.value.elements.some(element => element.text?.includes('flowchart LR'))));
      await invoke('open_file', { name: 'roundtrip' });
      await page.evaluate(() => document.fonts.ready);
      await page.waitForTimeout(400);
      await invoke('delete_elements', { ids: ['client'] });
      const deleted = await invoke('scene_summary');
      assert(!deleted.elements.some(e => e.id === 'client' || e.containerId === 'client'));
      assert.equal(deleted.elements.find(e => e.type === 'arrow').from, null);
      await invoke('open_file', { name: 'roundtrip' });
      assert.equal((await invoke('scene_summary')).total, moved.total);
      await page.keyboard.press('Control+s');
      await page.waitForTimeout(500);
      assert([...files.keys()].some(name => /^diagram-[a-f0-9-]+\.excalidraw$/.test(name)), 'Ctrl+S must use the broker with a unique copy name');
      await page.keyboard.press('Control+o');
      await page.waitForTimeout(200);
      assert(calls.includes('pick_file'), 'Ctrl+O must use the broker');
      await page.reload();
      await page.waitForFunction(() => window.__charioxViewAgent.registrations().length === 10);
      assert.equal((await invoke('scene_summary')).total, moved.total);
      assert.equal((await page.evaluate(() => window.__charioxViewAgent.changes(0))).ok, true);
      await mkdir(evidence, { recursive: true });
      await page.screenshot({ path: join(evidence, `canvas-dpr${dpr}.png`) });
      for (const name of ['proof.excalidraw', 'proof.svg', 'proof.png']) await writeFile(join(evidence, `dpr${dpr}-${name}`), files.get(name));
      await writeFile(join(evidence, `browser-dpr${dpr}.json`), JSON.stringify({ errors, network, calls, drawn, moved, png: { width: png.readUInt32BE(16), height: png.readUInt32BE(20) } }, null, 2));
      let release;
      await page.locator('.excalidraw__canvas.interactive').click({ position: { x: 50, y: 450 } });
      await page.keyboard.press('Escape');
      await page.keyboard.press('v');
      await invoke('select_elements', { ids: ['client'] });
      await page.waitForTimeout(500);
      holdSave = new Promise(resolve => { release = resolve; });
      const saving = new Promise(resolve => { saveStarted = resolve; });
      await page.evaluate(() => { window.pendingAction = window.__charioxViewAgent.invoke('add_elements', { elements: [{ id: 'stale-action', type: 'rectangle', x: 30, y: 30 }] }); });
      await saving;
      await page.keyboard.press('Delete');
      await page.waitForFunction(total => window.__charioxViewAgent.state().then(result => result.value.total === total - 2), moved.total);
      await page.waitForTimeout(200);
      holdSave = null; release();
      const raced = await page.evaluate(() => window.pendingAction);
      assert.equal(raced.ok, false, 'a pending action must preserve a concurrent human edit');
      const afterHuman = await invoke('scene_summary');
      assert.equal(afterHuman.total, moved.total - 2);
      assert(!afterHuman.elements.some(element => element.id === 'client'));
      assert(!afterHuman.elements.some(element => element.id === 'stale-action'));
      // A failed receipt must leave the visible scene unchanged. Private copies
      // remain available so a person can recover edits after a CAS conflict.
      await page.waitForTimeout(500);
      const beforeFailure = await invoke('scene_summary');
      failSave = true;
      const failed = await page.evaluate(() => window.__charioxViewAgent.invoke('add_elements', { elements: [{ id: 'must-not-appear', type: 'rectangle', x: 10, y: 10 }] }));
      assert.equal(failed.ok, false);
      assert.equal((await invoke('scene_summary')).total, beforeFailure.total, 'failed action must not alter the canvas');
      const recovered = await invoke('save_file', { name: 'recovery' });
      assert.match(recovered.automaticSaveError, /Concurrent view/);
      assert.equal(JSON.parse(files.get('recovery.excalidraw')).elements.length, beforeFailure.total);
      assert.deepEqual(errors, []);
      assert.deepEqual(network, [], 'all fonts, workers and code must load from App assets');
      await context.close();
    }
  } finally { await browser.close(); await new Promise(resolve => server.close(resolve)); }
});
