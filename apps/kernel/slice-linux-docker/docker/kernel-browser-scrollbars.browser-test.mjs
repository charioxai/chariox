// A page in the kernel browser lays out without a scrollbar gutter, as the mirror and a Mac show it. 2026-11-13 (elpais.com):
// a classic 15 px gutter made a banner sized to 100vw overflow the window. 2026-11-17 (nasa.gov, MDN at DPR1 and DPR2): the
// overlay-scrollbar flag does nothing in headless Chrome and a page's own scrollbar skin still took room headed, so the
// viewer laid the page out 15 px wider than the kernel and text re-wrapped. Real Chromium and the real host;
// CHARIOX_KERNEL_BROWSER_EXECUTABLE names the Chromium, CHARIOX_KERNEL_BROWSER_HEADLESS=1 runs it headless.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

const PAGES = {
  '/plain': '<div style="height:3000px">tall</div><div id="inner" style="height:100px;overflow:scroll"><div style="height:400px"></div></div>',
  '/skinned': '<style>::-webkit-scrollbar{width:12px;height:12px}::-webkit-scrollbar-thumb{background:#888}</style><div style="height:3000px">tall</div><div id="inner" style="height:100px;overflow:scroll"><div style="height:400px"></div></div>',
  '/thin': '<style>*{scrollbar-width:thin}</style><div style="height:3000px">tall</div><div id="inner" style="height:100px;overflow:scroll"><div style="height:400px"></div></div>',
};

const gutters = `(() => { const inner = document.getElementById('inner');
  return { root: innerWidth - document.documentElement.clientWidth, inner: inner.offsetWidth - inner.clientWidth }; })()`;

const listen = server => new Promise(resolve => server.listen(0, '127.0.0.1', resolve));

test('a page in the kernel browser has no scrollbar gutter, whatever scrollbar style it asks for', async () => {
  assert(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, 'explicit installed Chromium required');
  const root = await mkdtemp(path.join(tmpdir(), 'kernel-scrollbars-'));
  const server = createServer((request, response) => {
    response.setHeader('Content-Type', 'text/html');
    response.end(`<!doctype html><title>${request.url}</title>${PAGES[request.url] ?? ''}`);
  });
  await listen(server);
  const origin = `http://127.0.0.1:${server.address().port}`;
  const appStorageRoot = path.join(root, 'app-storage'); await mkdir(appStorageRoot);
  const host = new KernelBrowserHost(path.join(root, 'browser'), { appStorageRoot });
  try {
    const opened = await host.request({ op: 'open', url: `${origin}/plain` });
    const binding = { tab_id: opened.tab_id, generation: opened.generation };
    const measure = async () => {
      const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(opened.tab_id).target_id);
      return (await connection.send('Runtime.evaluate', { expression: gutters, returnByValue: true }, sessionId)).result.value;
    };
    for (const page of Object.keys(PAGES)) {
      // The tab keeps its setting across navigations, so each page after the first comes through navigate.
      if (page !== '/plain') await host.request({ op: 'navigate', ...binding, url: `${origin}${page}` });
      assert.deepEqual(await measure(), { root: 0, inner: 0 }, `${page}: no scrollbar gutter`);
    }
    const second = await host.request({ op: 'open', url: `${origin}/skinned` });
    const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(second.tab_id).target_id);
    assert.deepEqual((await connection.send('Runtime.evaluate', { expression: gutters, returnByValue: true }, sessionId)).result.value, { root: 0, inner: 0 }, 'a new tab has no scrollbar gutter');
  } finally {
    await host.stop();
    if (host.chromium.browserUser) await rm(host.chromium.browserUser.root, { recursive: true, force: true });
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
});
