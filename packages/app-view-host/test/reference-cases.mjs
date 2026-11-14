import assert from 'node:assert/strict';
import { join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { signReference } from '../build-reference.mjs';
import { adoptStandInSdk } from './reference-fixture.mjs';
import { processNetwork } from './process-network.mjs';
import { browserProfile } from './browser-profile.mjs';

export async function referenceCases({ chromium, launchOptions, server, evidence, builds, names = ['excalidraw', 'univer'] }) {
  const rows = [];
  for (const name of names) {
    // Only this test adapter supplies the old SDK in document memory. It does
    // not claim backend persistence, agent admission, broker access or a Room.
    const packed = await signReference(name, join(builds, name), join(evidence, 'signed-references'), adoptStandInSdk);
    for (const dpr of [1, 2]) {
      let profile, page;
      const row = { name, dpr, errors: [], console: [], remoteRequests: [], builtEntry: true, backend: 'frontend-only fixture' };
      try {
        // Use an actual renderer scale: context-only emulation did not reach
        // opaque out-of-process frames in the earlier Chrome155/Playwright run.
        profile = await browserProfile(chromium, { ...launchOptions, clientView: true, captureDirectory: join(evidence, 'process-tree', name + '-dpr' + dpr), args: [...launchOptions.args, `--force-device-scale-factor=${dpr}`],
          viewport: { width: 1440, height: 900 }, deviceScaleFactor: dpr });
        const { context } = profile;
        page = await context.newPage();
        page.on('pageerror', e => row.errors.push(e.stack));
        page.on('console', message => { if (row.console.length < 50) row.console.push(message.type() + ': ' + message.text().slice(0, 300)); });
        const requests = [];
        context.on('request', request => {
          requests.push(request.url());
          if (!request.url().startsWith(server.origin) && !/^(?:data|blob):/.test(request.url())) row.remoteRequests.push(request.url());
        });
        server.setBoundary(null); server.setPhase(`${name}-dpr${dpr}`);
        await server.open(page, packed, true);
        row.builtRequests = requests.filter(url => url.startsWith(server.origin + '/assets/'));
        const frame = await server.frame(page);
        // Excalidraw's page has no agent surface: its backend serves the actions, so it is ready when its canvas is.
        if (name === 'excalidraw') await frame.locator('.excalidraw__canvas.interactive').waitFor({ timeout: 25000 });
        else await frame.waitForFunction(() => globalThis.__charioxViewAgent?.registrations().length > 0, null, { timeout: 25000 });
        await delay(500);
        row.reportedDpr = await frame.evaluate(() => devicePixelRatio);
        row.parentDpr = await page.evaluate(() => devicePixelRatio);
        assert.equal(row.reportedDpr, dpr, 'App sees the native display DPR');
        if (name === 'excalidraw') {
          const canvas = frame.locator('.excalidraw__canvas.interactive');
          await canvas.click({ position: { x: 150, y: 180 } }); await page.keyboard.press('r');
          const bounds = await canvas.boundingBox();
          await page.mouse.move(bounds.x + 180, bounds.y + 180); await page.mouse.down();
          await page.mouse.move(bounds.x + 400, bounds.y + 310, { steps: 8 }); await page.mouse.up();
          await frame.waitForFunction(() => globalThis.__standInBackend.scene()?.elements.length > 0);
          row.nativeDrawing = await frame.evaluate(() => globalThis.__standInBackend.scene().elements.length);
          // Text typed into the rectangle is a bound label in the page's own font.
          await page.mouse.dblclick(bounds.x + 290, bounds.y + 245);
          await page.keyboard.type('Native App fonts');
          await page.keyboard.press('Escape');
          await frame.waitForFunction(() => globalThis.__standInBackend.scene()?.elements.some(element => element.text === 'Native App fonts'));
        } else {
          row.formula = await frame.evaluate(async () => {
            await globalThis.__charioxViewAgent.invoke('set_range', { range: 'A1:C2', values: [['Item', 'Qty', 'Total'], ['AV1', 7, '=B2*3']] });
            return globalThis.__charioxViewAgent.invoke('read_range', { range: 'A1:C2' });
          });
          assert.deepEqual(row.formula.value.values, [['Item', 'Qty', 'Total'], ['AV1', 7, 21]]);
          // The workbook's native cell editor must accept real keyboard input.
          const index = await frame.evaluate(() => [...document.querySelectorAll('canvas')]
            .map((canvas, index) => ({ index, area: canvas.clientWidth * canvas.clientHeight }))
            .sort((a, b) => b.area - a.area)[0].index);
          const canvas = frame.locator('canvas').nth(index);
          const bounds = await canvas.boundingBox();
          assert(bounds && bounds.width > 0);
          await canvas.click({ position: { x: 110, y: 100 } });
          await page.keyboard.press('Escape');
        }
        row.fonts = await frame.evaluate(async () => { await document.fonts.ready; return [...document.fonts].map(f => ({ family: f.family, status: f.status })); });
        row.canvases = await frame.evaluate(async () => {
          await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
          return [...document.querySelectorAll('canvas')].map(canvas => {
            const context = canvas.getContext('2d'); let inkPixels = 0;
            if (context && canvas.width && canvas.height) {
              const data = context.getImageData(0, 0, canvas.width, canvas.height).data;
              for (let i = 0; i < data.length; i += 4) if (data[i + 3] > 0 && (data[i] < 245 || data[i + 1] < 245 || data[i + 2] < 245)) inkPixels++;
            }
            return { width: canvas.width, height: canvas.height, clientWidth: canvas.clientWidth, clientHeight: canvas.clientHeight, inkPixels };
          });
        });
        row.rendered = row.canvases.some(canvas => canvas.clientWidth > 0 && canvas.width >= canvas.clientWidth * dpr && canvas.inkPixels > 20);
        assert(row.rendered, 'reference has a native canvas at the actual DPR');
        if (name === 'excalidraw') assert(row.fonts.some(font => font.family === 'Virgil' && font.status === 'loaded'), 'the embedded drawing font loaded natively');
        assert.equal(row.errors.length, 0, 'reference has no uncaught frontend errors');
        assert.equal(row.remoteRequests.length, 0, 'reference uses no remote frontend resources');
        await page.screenshot({ path: join(evidence, name + '-dpr' + dpr + '.png') });
        await writeConsole(page, join(evidence, name + '-dpr' + dpr + '-dom.txt'));
      } catch (error) {
        row.failure = error.stack;
        // What the page showed when it stopped is the first thing to read.
        if (page) { await page.screenshot({ path: join(evidence, name + '-dpr' + dpr + '-failed.png') }).catch(() => {}); await writeConsole(page, join(evidence, name + '-dpr' + dpr + '-failed-dom.txt')).catch(() => {}); row.frames = await Promise.all(page.frames().map(async frame => ({ url: frame.url().slice(0, 100), text: await frame.locator('body').innerText({ timeout: 2000 }).then(text => text.slice(0, 300), () => null) }))); }
      }
      finally { await profile?.close(); if (profile) row.processNetwork = await processNetwork(join(evidence, 'process-tree', name + '-dpr' + dpr)); }
      rows.push(row);
      console.log(name, dpr, JSON.stringify({ rendered: row.rendered, failure: row.failure?.slice(0, 180), pageErrors: row.errors.length }));
    }
  }
  return rows;
}

async function writeConsole(page, path) {
  const { writeFile } = await import('node:fs/promises');
  await writeFile(path, await page.locator('body').innerText());
}
