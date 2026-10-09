// Opt-in MP-08/MP-11: real Chromium, every frame incl. isolated ones, DPR 1 and 2.
// Oracle: CDP viewport pixels. Protected content (magenta) is fully covered;
// ordinary content (cyan: consent dialog, cross-site captcha frame, article)
// stays visible. Desktop placement is proven by the Xvfb drill.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { decodePng } from './kernel-browser-pixels.mjs';
import { fenceBrowserCapture, measureBrowserProtection } from './browser-protection-regions.mjs';
import { VAULT_VALUE, census, launchChromium, openFixture, serveFixture } from './browser-protection-fixture.mjs';
const executable = process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE;
assert.ok(executable, 'Explicit installed Chromium required; never download a browser');

async function withFixture(dpr, run, query = '') {
  const root = await mkdtemp(path.join(tmpdir(), 'cx-protection-'));
  const fixture = await serveFixture();
  const chromium = await launchChromium({ executable, dpr, root });
  try {
    const opened = await openFixture(chromium.browser, fixture.url + query);
    const shot = async () => decodePng((await opened.connection.send('Page.captureScreenshot', { format: 'png' }, opened.sessionId)).data, dpr);
    await run({ ...chromium, ...opened, shot });
  } finally { await chromium.close(); await fixture.close(); await rm(root, { recursive: true, force: true }); }
}
const policy = (values = []) => ({ values, targets: [], unknown: false });

// Vault policy: whole-page DOMSnapshot path (echoes, markers incl. frame owners).
// No policy on a fields-only page: selector-search path, no DOMSnapshot.
for (const dpr of [1, 2]) for (const [label, values, query] of [['snapshot', [VAULT_VALUE], ''], ['search', [], '?novault&nomarkers']]) {
  test(`DPR ${dpr} ${label}: every protected frame region is covered; ordinary content stays visible`, () => withFixture(dpr, async ({ browser, connection, sessionId, shot }) => {
    for (const scroll of [0, 300]) {
      await connection.send('Runtime.evaluate', { expression: `scrollTo(0, ${scroll})` }, sessionId);
      let snapshots = 0;
      const send = connection.send.bind(connection);
      connection.send = (method, ...rest) => { if (method === 'DOMSnapshot.captureSnapshot') snapshots++; return send(method, ...rest); };
      const { pages } = await measureBrowserProtection(browser, policy(values)).finally(() => { connection.send = send; });
      assert.equal(snapshots > 0, label === 'snapshot', `${label} path`);
      assert.equal(pages.length, 1);
      const [page] = pages;
      assert.equal(page.dpr, dpr);
      const pixels = await shot();
      assert.deepEqual(page.viewport, [pixels.width, pixels.height]);
      const before = census(pixels), after = census(pixels, page.regions);
      assert.ok(before.magenta > 1000 * dpr * dpr, 'fixture shows protected pixels');
      assert.equal(after.magenta, 0, `scroll ${scroll}: protected pixels remain`);
      // Every frame is inspected; only the scaled, mirrored and rotated frames are withheld whole
      // (the selector search places the rotated in-process frame's field from its transformed quad).
      assert.ok(page.withheld.every(reason => reason === 'transformed_frame') && (scroll || page.withheld.length === (values.length ? 3 : 2)), String(page.withheld));
      // Ordinary content incl. the cross-site captcha frame is not withheld.
      assert.ok(after.cyan > before.cyan * 0.95, `scroll ${scroll}: ordinary content masked (${after.cyan}/${before.cyan})`);
    }
  }, query));
}

// The canvas draws the value and removes its script: no DOM string echoes it.
for (const dpr of [1, 2]) test(`DPR ${dpr}: Vault echoes and opaque media are protected only while values are registered`, () => withFixture(dpr, async ({ browser, connection, sessionId, shot }) => {
  const { result } = await connection.send('Runtime.evaluate', { returnByValue: true,
    expression: `[...document.scripts].some(script => script.text.includes(${JSON.stringify(VAULT_VALUE)})) ? null : ['drawn', 'media'].map(id => document.getElementById(id).getBoundingClientRect()).map(b => [b.left, b.top, b.width, b.height])` }, sessionId);
  assert.ok(result.value, 'the drawing script is gone');
  const [drawn, media] = result.value.map(box => box.map(v => v * dpr));
  const pixels = await shot();
  const without = (await measureBrowserProtection(browser, policy())).pages[0];
  const withValue = (await measureBrowserProtection(browser, policy([VAULT_VALUE]))).pages[0];
  const crop = ([x, y, w, h]) => ({ width: w, pixels: Buffer.concat(Array.from({ length: h }, (_, row) => pixels.pixels.subarray(((y + row) * pixels.width + x) * 4, ((y + row) * pixels.width + x + w) * 4))) });
  const inBox = ([x, y, w, h], regions) => regions.map(([rx, ry, rw, rh]) => [rx - x, ry - y, rw, rh]);
  assert.ok(census(crop(drawn), inBox(drawn, without.regions)).magenta > 50 * dpr * dpr, 'the drawn value is ordinary without a Vault value');
  assert.ok(census(crop(media), inBox(media, without.regions)).cyan === media[2] * media[3], 'media stay visible without a Vault value');
  assert.ok(census(pixels, without.regions).magenta > 0, 'the echo paragraph is ordinary without a Vault value');
  assert.equal(census(pixels, withValue.regions).magenta, 0, 'echoes and the canvas-drawn value are masked');
  assert.equal(census(crop(media), inBox(media, withValue.regions)).cyan, 0, 'every medium is masked while a value is registered');
}));

test('a captcha frame stays visible; its region is not protected', () => withFixture(1, async ({ browser, connection, sessionId }) => {
  const { result } = await connection.send('Runtime.evaluate', { returnByValue: true, expression: 'JSON.stringify(document.getElementById("captcha").getBoundingClientRect())' }, sessionId);
  const box = JSON.parse(result.value);
  const { pages } = await measureBrowserProtection(browser, policy());
  const overlap = pages[0].regions.filter(([x, y, w, h]) => x < box.right && x + w > box.left && y < box.bottom && y + h > box.top);
  assert.deepEqual(overlap, []);
}));

test('layout moved between measurement and capture re-measures, then fails closed', () => withFixture(1, async ({ browser, connection, sessionId }) => {
  const calls = [];
  const once = await fenceBrowserCapture(browser, policy(), async protection => {
    calls.push(protection);
    if (calls.length === 1) await connection.send('Runtime.evaluate', { expression: 'scrollBy(0, 120)' }, sessionId);
    return calls.length;
  });
  assert.equal(once, 2, 'the moved capture is discarded and re-captured under a fresh measurement');
  assert.notDeepEqual(calls[1].pages[0].regions, calls[0].pages[0].regions);
  const moving = [];
  const result = await fenceBrowserCapture(browser, policy(), async protection => {
    moving.push(protection);
    if (protection) await connection.send('Runtime.evaluate', { expression: 'scrollBy(0, 40)' }, sessionId);
    return protection;
  });
  assert.equal(result, null, 'continuously moving layout is captured fail-closed');
  assert.deepEqual(moving.map(Boolean), [true, true, true, false]);
}));

test('unknown protection fails closed', () => withFixture(1, async ({ browser }) => {
  await assert.rejects(measureBrowserProtection(browser, { values: [], targets: [], unknown: true }));
  assert.equal(await fenceBrowserCapture(browser, { values: [], targets: [], unknown: true }, async protection => protection), null);
}));

test('a visible browser-internal page (settings, extensions, DevTools) fails closed', () => withFixture(1, async ({ browser, connection, sessionId }) => {
  await connection.send('Page.navigate', { url: 'chrome://version/' }, sessionId);
  for (let i = 0; i < 50 && (await connection.send('Page.getFrameTree', {}, sessionId)).frameTree.frame.url !== 'chrome://version/'; i++) await new Promise(resolve => setTimeout(resolve, 50));
  await assert.rejects(measureBrowserProtection(browser, policy()), /browser-internal page/);
}));
