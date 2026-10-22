// MP-08/MP-10/MP-11: supplementary 539/147 observer checks in real Chromium.
// These credential-free fixtures do not replace the real hosted acceptance drill.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import path from 'node:path';
import { readFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { mirror2ObserverExpression } from './kernel-browser-mirror2-observer.mjs';

assert(process.env.MIRROR_LAB_NODE_MODULES && path.isAbsolute(process.env.MIRROR_LAB_NODE_MODULES));
const { chromium } = createRequire(path.join(process.env.MIRROR_LAB_NODE_MODULES, 'x.js'))('playwright-core');
const launch = () => chromium.launch({ executablePath: process.env.MIRROR_LAB_CHROME ?? '/opt/google/chrome/chrome', headless: true });
async function install(page) {
  await page.evaluate(mirror2ObserverExpression());
  await page.evaluate(() => { window.observer = globalThis.__charioxMirror2.get('completion-test'); });
}

test('MP-08/MP-10/MP-11 native loaded-font requirements wake credits and ignore unused failures', async () => {
  const bytes = await readFile(process.env.MIRROR_LAB_FONT ?? '/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf');
  let response;
  const server = createServer((_request, res) => { response = res; res.setHeader('Access-Control-Allow-Origin', '*'); res.setHeader('Content-Type', 'font/ttf'); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const browser = await launch();
  try {
    const page = await browser.newPage();
    await page.setContent('<body>Native fonts</body>'); await install(page);
    const first = await page.evaluate(() => window.observer.snapshot());
    assert.deepEqual((first.ops ?? []).filter(op => op.op === 'fonts'), [{ op: 'fonts', id: first.root, required: [] }]);
    await page.evaluate(url => { window.requiredFont = new FontFace('NativeRequired', `url(${url})`); document.fonts.add(window.requiredFont); void window.requiredFont.load(); }, `http://127.0.0.1:${server.address().port}/font`);
    await page.waitForFunction(() => document.fonts.status === 'loading');
    assert((await page.evaluate(() => window.observer.drain())).ops.some(op => op.op === 'fonts' && op.required === null));
    const waiting = page.evaluate(() => window.observer.waitDrain(2000));
    for (let n = 0; !response && n < 100; n++) await new Promise(resolve => setTimeout(resolve, 10));
    assert(response); response.end(bytes);
    await page.evaluate(() => window.requiredFont.loaded);
    let packet = await waiting;
    const signature = createHash('sha256').update(JSON.stringify(['nativerequired','normal','400','100%','U+0-10FFFF'])).digest('hex');
    for (let n = 0; n < 3 && !packet.ops.some(op => op.op === 'fonts' && op.required?.includes(signature)); n++) packet = await page.evaluate(() => window.observer.drain());
    assert(packet.ops.some(op => op.op === 'fonts' && JSON.stringify(op.required) === JSON.stringify([signature])));
    await page.evaluate(async () => { const unused = new FontFace('UnusedNative', 'url(data:font/woff2;base64,AA==)'); document.fonts.add(unused); await unused.load().catch(() => {}); });
    const last = await page.evaluate(() => window.observer.snapshot());
    assert.deepEqual(last.ops.find(op => op.op === 'fonts').required, [signature]);
  } finally { response?.end(); await browser.close(); await new Promise(resolve => server.close(resolve)); }
});

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
  const fixture=JSON.parse(await (await import('node:fs/promises')).readFile(new URL('../../src/local/api/tests/protocol_shapes/browser-mirror2-font-failures-550.json',import.meta.url),'utf8'));
  assert.deepEqual(packet.font_failures,[fixture.descriptor_sha256]);
  await page.evaluate(()=>document.fonts.add(new FontFace('NativeFailed','url(data:font/woff2;base64,AAAA)')));
  assert.deepEqual((await page.evaluate(()=>window.observer.drain())).font_failures,[],'an unloaded matching face could still supply glyphs, so failure is not certified');
 }finally{await browser.close()}
});

// MP-08/MP-10/MP-11 (r73): the viewer must fall through a font stack exactly where the native page did. Names a stack
// lists that the native machine does not have travel as `absent`; installed families and the page's own faces never do.
test('MP-08/MP-10/MP-11 the fonts op names stack families the native machine lacks', async () => {
  const browser = await launch();
  try {
    const page = await browser.newPage();
    await page.setContent(String.raw`<style>@font-face{font-family:"Page Face";src:url(data:font/woff2;base64,AA==)}</style><body style="font-family:-apple-system,'Chariox Absent One','Page Face',Arial,serif"><p style="font-family:'Chariox Absent Two',sans-serif">text</p><p style="font-family:'Evil\&quot;};x{',monospace">x</p></body>`);
    await install(page);
    const first = await page.evaluate(() => window.observer.snapshot());
    const op = first.ops.find(entry => entry.op === 'fonts');
    assert.deepEqual(op.absent, ['-apple-system', 'chariox absent one', 'chariox absent two']);
    const again = await page.evaluate(() => window.observer.drain());
    assert(!again.ops.some(entry => entry.op === 'fonts'), 'an unchanged absent set is not re-sent');
  } finally { await browser.close(); }
});

// r74: the scan must follow the document: a class/style change or newly inserted content naming an absent family after the
// document settled is reported, not left to the viewer's own installed family.
test('MP-08/MP-10/MP-11 a font stack that changes after completion reports its absent families', async () => {
  const browser = await launch();
  try {
    const page = await browser.newPage();
    await page.setContent('<body style="font-family:Arial,serif"><p>settled</p></body>');
    await install(page);
    const first = await page.evaluate(() => window.observer.snapshot());
    assert(!first.ops.find(entry => entry.op === 'fonts').absent, 'a settled system-font page names nothing');
    await page.evaluate(() => { document.body.style.fontFamily = '"Chariox Late One", serif'; const p = document.createElement('p'); p.style.fontFamily = '"Chariox Late Two", sans-serif'; p.textContent = 'later'; document.body.append(p); });
    let absent = null;
    for (let n = 0; n < 8 && !absent; n++) absent = (await page.evaluate(() => window.observer.waitDrain(1500))).ops.find(entry => entry.op === 'fonts' && entry.absent)?.absent;
    assert.deepEqual(absent, ['chariox late one', 'chariox late two']);
  } finally { await browser.close(); }
});

// MP-08/MP-10/MP-11: CSSOM updates bypass MutationObserver; the CDP sheet-change bridge must refresh stacks too.
for (const adopted of [false, true]) test(`MP-10 absent families follow stylesheet-only edits (adopted=${adopted})`, async () => {
  const browser = await launch();
  try {
    const page = await browser.newPage();
    await page.setContent('<style>p{font-family:system-ui}</style><p>Native fallback</p>');
    if (adopted) await page.evaluate(() => { const sheet = new CSSStyleSheet(); sheet.replaceSync('p{font-family:system-ui}'); document.adoptedStyleSheets = [sheet]; });
    await install(page);
    assert(!(await page.evaluate(() => window.observer.snapshot())).ops.find(op => op.op === 'fonts').absent);
    await page.evaluate(adopted => {
      if (adopted) document.adoptedStyleSheets[0].replaceSync('p{font-family:"chariox css-only absent",system-ui}');
      else document.styleSheets[0].cssRules[0].style.fontFamily = '"chariox css-only absent",system-ui';
      window.observer.sheetChanged(adopted ? 'adopted' : document.querySelector('style'));
    }, adopted);
    let absent;
    for (let n = 0; n < 4 && !absent; n++) absent = (await page.evaluate(() => window.observer.waitDrain(700))).ops.find(op => op.op === 'fonts' && op.absent)?.absent;
    assert.deepEqual(absent, ['chariox css-only absent']);
  } finally { await browser.close(); }
});

// r83 (MIRROR-FONT-ABSENT-REFRESH): a face only a script added (`document.fonts.add(new FontFace(...))`) has no stylesheet rule.
// Chrome reports its source over CDP: a URL is mirrored as a synthetic @font-face, bytes the viewer cannot have are neither
// required (they would hold the handover) nor left to a viewer family of the same name.
test('MP-08/MP-10/MP-11 script-added font faces are mirrored like stylesheet faces', async () => {
  const bytes = await readFile(process.env.MIRROR_LAB_FONT ?? '/usr/share/fonts/truetype/liberation/LiberationSerif-Regular.ttf');
  const server = createServer((_request, res) => { res.setHeader('Access-Control-Allow-Origin', '*'); res.setHeader('Content-Type', 'font/ttf'); res.end(bytes); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const { MirrorCssEvents } = await import('./kernel-browser-mirror2-css.mjs');
  const browser = await launch();
  try {
    const page = await browser.newPage();
    await page.setContent('<body style="font-family:ScriptUrl,ScriptBytes,serif">script faces</body>');
    const session = await page.context().newCDPSession(page);
    const listeners = new Set();
    for (const name of ['CSS.styleSheetAdded', 'CSS.styleSheetRemoved', 'CSS.styleSheetChanged', 'CSS.fontsUpdated']) session.on(name, params => { for (const listener of listeners) listener({ sessionId: 's1', method: name, params }); });
    let contextId;
    session.on('Runtime.executionContextCreated', ({ context }) => { if (context.auxData?.isDefault) contextId = context.id; });
    await session.send('Runtime.enable');
    const connection = { subscribe: listener => { listeners.add(listener); return () => listeners.delete(listener); }, send: (method, params) => session.send(method, params) };
    await install(page);
    const first = await page.evaluate(() => window.observer.snapshot());
    const css = new MirrorCssEvents();
    await css.watch({ connection, sessionId: 's1', contextId }, 'frame', 'stream');
    const origin = `http://127.0.0.1:${server.address().port}`;
    await page.evaluate(async origin => {
      const fromUrl = new FontFace('ScriptUrl', `url(${origin}/a.ttf)`, { weight: '700' }); document.fonts.add(fromUrl); await fromUrl.load();
      const buffer = await (await fetch(`${origin}/b.ttf`)).arrayBuffer();
      const fromBytes = new FontFace('ScriptBytes', buffer); document.fonts.add(fromBytes); await fromBytes.load();
    }, origin);
    const digest = descriptor => createHash('sha256').update(JSON.stringify(descriptor)).digest('hex');
    const urlFace = digest(['scripturl', 'normal', '700', '100%', 'U+0-10FFFF']), bytesFace = digest(['scriptbytes', 'normal', '400', '100%', 'U+0-10FFFF']);
    const ops = [], resources = [];
    for (let n = 0; n < 12 && !(ops.some(op => op.op === 'adopted') && ops.some(op => op.op === 'fonts' && op.absent)); n++) {
      const packet = await page.evaluate(() => window.observer.waitDrain(700)); ops.push(...packet.ops); resources.push(...(packet.resources ?? []));
    }
    const sheets = ops.filter(op => op.op === 'adopted').flatMap(op => op.sheets).join('\n');
    assert.match(sheets, /@font-face\{font-family:"ScriptUrl";[^}]*font-weight:700;[^}]*src:url\("mr:r\d+"\)\}/, 'the URL face is mirrored as a font-face rule');
    assert(resources.some(resource => resource.kind === 'font' && resource.url === `${origin}/a.ttf`), 'its URL is requested as a font resource');
    assert(!/ScriptBytes/.test(sheets), 'bytes the viewer cannot have are not described');
    const fonts = ops.filter(op => op.op === 'fonts').at(-1);
    assert(fonts.required.includes(urlFace), 'the URL face is a requirement the viewer can meet');
    assert(!fonts.required.includes(bytesFace), 'the bytes face would hold the handover forever');
    assert(fonts.absent.includes('scriptbytes'), 'its family is shadowed so the stack falls through');
    assert(!fonts.absent.includes('scripturl'));
    assert(first.root);
  } finally { await browser.close(); server.close(); }
});
