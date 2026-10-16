// MP-08/MP-10/MP-11: supplementary 539/147 observer checks in real Chromium.
// These credential-free fixtures do not replace the real hosted acceptance drill.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import path from 'node:path';
import { mirror2ObserverExpression } from './kernel-browser-mirror2-observer.mjs';

assert(process.env.MIRROR_LAB_NODE_MODULES && path.isAbsolute(process.env.MIRROR_LAB_NODE_MODULES));
const { chromium } = createRequire(path.join(process.env.MIRROR_LAB_NODE_MODULES, 'x.js'))('playwright-core');
const launch = () => chromium.launch({ executablePath: process.env.MIRROR_LAB_CHROME ?? '/opt/google/chrome/chrome', headless: true });
async function install(page) {
  await page.evaluate(mirror2ObserverExpression());
  await page.evaluate(() => { window.observer = globalThis.__charioxMirror2.get('completion-test'); });
}

test('MP-08/MP-11 long SVG geometry survives without admitting long metadata', async () => {
  const browser = await launch();
  try {
    const page = await browser.newPage();
    const d = 'M0 0L10 0L10 10L0 10Z'.repeat(80), points = '0,0 10,0 10,10 0,10 '.repeat(80);
    await page.setContent(`<svg><path id="logo" d="${d}" data-metadata="${d}"/><polygon id="polygon" points="${points}"/><path id="oversized" d="${'M'.repeat(65537)}"/></svg><div id="html" d="${d}"></div>`);
    await install(page);
    const packet = await page.evaluate(() => window.observer.snapshot());
    const attrs = id => packet.nodes.find(node => node.attrs?.id === id)?.attrs;
    assert.equal(attrs('logo')?.d, d);
    assert.equal(attrs('polygon')?.points, points);
    assert.equal(attrs('logo')?.['data-metadata'], undefined);
    assert.equal(attrs('oversized')?.d, undefined);
    assert.equal(attrs('html')?.d, undefined);
  } finally { await browser.close(); }
});

test('MP-10 completion is false throughout a bounded initial continuation', async () => {
  const browser = await launch();
  try {
    const page = await browser.newPage();
    await page.setContent('<body><header hidden>' + '<span>hidden</span>'.repeat(12000) + '</header><p>Final viewport</p>');
    await install(page);
    let packet = await page.evaluate(() => window.observer.snapshot()), drains = 0;
    assert.equal(packet.snapshot_complete, false);
    while (await page.evaluate(() => window.observer.pending())) {
      assert.equal(packet.snapshot_complete, false);
      packet = await page.evaluate(() => window.observer.drain());
      assert(++drains < 100);
    }
    assert.equal(packet.snapshot_complete, true);
    assert(drains > 0);
  } finally { await browser.close(); }
});

for (const betweenCredits of [false, true]) test(`MP-10 native completion wakes an empty credit, betweenCredits=${betweenCredits}`, async () => {
  let response;
  const server = createServer((_request, res) => {
    response = res;
    res.setHeader('Content-Type', 'text/html');
    res.write('<!doctype html><html><head></head><body></body></html>');
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await launch();
  try {
    const page = await browser.newPage();
    await page.goto('http://127.0.0.1:' + server.address().port, { waitUntil: 'commit' });
    await page.waitForFunction(() => document.body);
    await install(page);
    assert.equal((await page.evaluate(() => window.observer.snapshot())).snapshot_complete, false);
    let waiting;
    if (!betweenCredits) {
      waiting = page.evaluate(() => window.observer.waitDrain(2000));
      await new Promise(resolve => setTimeout(resolve, 50));
    }
    response.end();
    // No DOM mutation follows the snapshot. Completion before the next credit
    // must remain unreported until that credit observes it.
    if (betweenCredits) await page.waitForFunction(() => document.readyState === 'complete');
    const started = Date.now();
    let packet = await (waiting ?? page.evaluate(() => window.observer.waitDrain(2000)));
    for (let n = 0; n < 3 && !packet.snapshot_complete; n++) packet = await page.evaluate(() => window.observer.waitDrain(2000));
    assert.equal(packet.snapshot_complete, true);
    assert(Date.now() - started < 1500, 'completion wakes before the idle timeout');
  } finally {
    response?.end();
    await browser.close();
    await new Promise(resolve => server.close(resolve));
  }
});

test('MP-11 hidden subtree stylesheet owners stay in the initial cascade', async () => {
  const browser = await launch();
  try {
    const page = await browser.newPage();
    await page.setContent('<body><header hidden><style>main{color:rgb(12,34,56)}</style><span>Hidden</span></header><main>Visible destination</main>');
    await install(page);
    const first = await page.evaluate(() => window.observer.snapshot());
    assert(first.nodes.some(r => r.tag === 'style' && r.css.includes('12, 34, 56')), 'global CSS remains available before the destination paints');
    assert(first.nodes.some(r => r.text === 'Visible destination'));
  } finally { await browser.close(); }
});


test('MP-11 relational CSS keeps the hidden structure needed by the visible viewport', async () => {
 const browser=await launch();try{
  const page=await browser.newPage();
  await page.setContent('<style>main{display:none}body:has(header .ready) main{display:block}</style><header hidden><span class="ready">state</span></header><main>Visible destination</main>');
  await install(page);const first=await page.evaluate(()=>window.observer.snapshot());
  assert(first.nodes.some(r=>r.attrs?.class==='ready'),'omitting hidden descendants would change the visible selector result');
 }finally{await browser.close();}
});

// MP-08/MP-10/MP-11: source-native failures permit the same fallback, never a missing live font.
test('MP-08/MP-10/MP-11 native font failure metadata wakes and excludes a nonfailed matching face',async()=>{
 const browser=await launch();try{
  const page=await browser.newPage();await page.setContent('<body>Native fallback metadata</body>');await install(page);
  await page.evaluate(()=>{window.nativeFont=new FontFace('NativeFailed','url(data:font/woff2;base64,AA==)');document.fonts.add(window.nativeFont)});
  assert.deepEqual((await page.evaluate(()=>window.observer.snapshot())).font_failures,[]);
  const waiting=page.evaluate(()=>window.observer.waitDrain(2000));await page.evaluate(async()=>{await window.nativeFont.load().catch(()=>{})});
  let packet=await waiting;for(let n=0;n<3&&!packet.font_failures?.length;n++)packet=await page.evaluate(()=>window.observer.drain());
  const fixture=JSON.parse(await (await import('node:fs/promises')).readFile(new URL('../../src/local/api/tests/protocol_shapes/browser-mirror2-font-failures-539.json',import.meta.url),'utf8'));
  assert.deepEqual(packet.font_failures,[fixture.descriptor_sha256]);
  await page.evaluate(()=>document.fonts.add(new FontFace('NativeFailed','url(data:font/woff2;base64,AAAA)')));
  assert.deepEqual((await page.evaluate(()=>window.observer.drain())).font_failures,[],'an unloaded matching face could still supply glyphs, so failure is not certified');
 }finally{await browser.close()}
});
