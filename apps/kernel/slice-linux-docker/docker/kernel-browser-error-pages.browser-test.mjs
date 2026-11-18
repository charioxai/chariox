// A Chrome error page (an HTTP error without a body, a refused connection) is a page: navigate commits it, the display
// keeps encoding it, and the browser keeps answering. Real Chromium and the real host; CHARIOX_KERNEL_BROWSER_EXECUTABLE
// names the Chromium.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

const EMPTY_BODY_DELAY_MS = 300;

const loaded = async (host, tabId, url, label) => {
  for (let attempt = 0; attempt < 50; attempt++) {
    const { tabs } = await host.request({ op: 'state' });
    if (tabs.some(tab => tab.tab_id === tabId && tab.url === url)) return;
    await delay(100);
  }
  assert.fail(label);
};

const listen = server => new Promise(resolve => server.listen(0, '127.0.0.1', resolve));

test('a failed load is a page: navigate commits it and the browser keeps answering', async () => {
  assert(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, 'explicit installed Chromium required');
  const root = await mkdtemp(path.join(tmpdir(), 'kernel-error-pages-'));
  const server = createServer((request, response) => {
    // The answer comes after the display has started encoding the page that is being left.
    const empty = status => setTimeout(() => { response.statusCode = status; response.end(); }, EMPTY_BODY_DELAY_MS);
    if (request.url === '/empty-500') return empty(500);
    if (request.url === '/empty-404') return empty(404);
    if (request.url === '/no-content') { response.statusCode = 204; return response.end(); }
    response.setHeader('Content-Type', 'text/html'); response.end(`<!doctype html><title>${request.url}</title><p>page ${request.url}</p>`);
  });
  await listen(server);
  const origin = `http://127.0.0.1:${server.address().port}`;
  const refused = createServer(); await listen(refused);
  const refusedOrigin = `http://127.0.0.1:${refused.address().port}`;
  await new Promise(resolve => refused.close(resolve));
  const appStorageRoot = path.join(root, 'app-storage'); await mkdir(appStorageRoot);
  const host = new KernelBrowserHost(path.join(root, 'browser'), { appStorageRoot });
  let pumping = true, pump;
  try {
    const opened = await host.request({ op: 'open', url: `${origin}/ok` });
    const binding = { tab_id: opened.tab_id, generation: opened.generation };
    const display = await host.request({ op: 'display_subscribe', ...binding, codecs: ['png', 'vp09.00.10.08'], bitrate: 4_000_000, device_scale_factor: 1 });
    await host.request({ op: 'display_attach', subscription_id: display.subscription_id, generation: opened.generation });
    // A viewer asks for frames all the time, also while a navigation is in flight.
    pump = (async () => {
      let sequence = 0;
      while (pumping) {
        const frame = await host.request({ op: 'screenshot', display_subscription_id: display.subscription_id, generation: opened.generation, after_sequence: sequence, wait_ms: 100 }).catch(() => ({}));
        if (Number.isInteger(frame.sequence)) sequence = frame.sequence;
        await delay(30);
      }
    })();

    for (const failing of ['/empty-500', '/empty-404', `${refusedOrigin}/`]) {
      const url = failing.startsWith('/') ? `${origin}${failing}` : failing;
      const navigated = await host.request({ op: 'navigate', ...binding, url });
      assert(navigated.tabs.some(tab => tab.tab_id === opened.tab_id), `${url}: navigate answers with the tab`);
      await delay(1_000);
      // The wedge is a state read that times out until the kernel restarts: three reads in a row must answer.
      for (let read = 0; read < 3; read++) {
        const state = await host.request({ op: 'state' });
        assert(state.tabs.some(tab => tab.tab_id === opened.tab_id), `${url}: state answers`);
      }
      await host.request({ op: 'navigate', ...binding, url: `${origin}/ok` });
      await loaded(host, opened.tab_id, `${origin}/ok`, `${url}: the next navigate loads a page`);
      const second = await host.request({ op: 'open', url: `${origin}/second` });
      assert.equal(typeof second.tab_id, 'string', `${url}: a new tab opens`);
      await host.request({ op: 'close', tab_id: second.tab_id, generation: second.generation });
    }

    // A load Chrome aborts keeps the page that is showing: that stays a failure.
    await assert.rejects(host.request({ op: 'navigate', ...binding, url: `${origin}/no-content` }), /navigation failed: net::ERR_ABORTED/);
  } finally {
    pumping = false; await pump;
    await host.stop();
    if (host.chromium.browserUser) await rm(host.chromium.browserUser.root, { recursive: true, force: true });
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
});
