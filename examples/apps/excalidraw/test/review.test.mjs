// MP-08/MP-09/MP-10/MP-11: real Excalidraw UI regressions with a fixture broker.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir } from 'node:fs/promises';
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
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
    const broker = { scene: null, version: null, saveAttempts: 0, fail: false, files: new Map(), writes: [], picked: null };
    await page.exposeFunction('broker', async (name, input) => {
      if (name === 'load_scene') return { scene: broker.scene, version: broker.version };
      if (name === 'save_scene') {
        broker.saveAttempts++;
        if (broker.fail) throw Error('Concurrent view changed the scene; reload.');
        assert.equal(input.version, broker.version);
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
    await page.addInitScript(`const sdk = (${createViewActions.toString()})(); window.chariox = { action: sdk.action, view: sdk.view, call: (...args) => window.broker(...args) }; window.__charioxViewAgent = sdk.agent;`);
    await page.goto(`http://127.0.0.1:${server.address().port}/index.html`);
    await page.waitForFunction(() => window.__charioxViewAgent.registrations().length === 10);
    const invoke = (name, input = {}) => page.evaluate(({ name, input }) => window.__charioxViewAgent.invoke(name, input), { name, input });
    const draw = async (x = 300) => {
      await page.locator('.excalidraw__canvas.interactive').click({ position: { x: 1000, y: 650 } });
      await page.keyboard.press('Escape'); await page.keyboard.press('r');
      await page.waitForTimeout(100);
      await page.mouse.move(x, 200); await page.mouse.down();
      await page.mouse.move(x + 100, 280, { steps: 5 }); await page.waitForTimeout(100); await page.mouse.up();
      await page.waitForTimeout(450);
    };
    await mkdir(evidence, { recursive: true });
    await run({ page, broker, invoke, draw });
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
  assert.equal(await notice.count(), 0, 'the same failure must be reported once even after another human edit');
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
  await page.screenshot({ path: join(evidence, 'oversized-recovery.png') });
}));
