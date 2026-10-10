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
