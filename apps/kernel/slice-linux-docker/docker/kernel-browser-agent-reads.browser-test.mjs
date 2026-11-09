// MP-08/MP-09/MP-10/MP-11: opt-in native Chromium regression for the agent-only reads (region screenshot, save, scroll to a node,
// wait), supplementary to the kernel/provider drill. CHARIOX_KERNEL_BROWSER_EXECUTABLE names the Chromium.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, rm, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { KernelBrowserHost } from './kernel-browser-host.mjs';
import { encodePng } from './kernel-browser-pixels.mjs';

const sizeOf = data => { const header = Buffer.from(data.slice(0, 44), 'base64'); return { width: header.readUInt32BE(16), height: header.readUInt32BE(20) }; };
const LOGO = encodePng(40, 30, Buffer.alloc(40 * 30 * 4, 200));
const PAGE = `<!doctype html><title>reads</title><body style="margin:0">
<h1 id="top" style="margin:0;height:120px;background:#cde">Top</h1>
<img id="logo" src="/logo.png" width="40" height="30">
<div style="height:2400px"></div>
<p id="far" style="margin:0;height:80px;background:#fdc">Far down</p>
<div style="height:600px"></div>
<script>setTimeout(()=>{const p=document.createElement('p');p.textContent='Appeared later';document.body.append(p)},600);
setTimeout(()=>history.pushState({},'','/pushed/state'),900)</script>`;

test('native region screenshot, save, scroll to node and wait on a real page', async () => {
  assert(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, 'explicit installed Chromium required');
  const root = await mkdtemp(path.join(tmpdir(), 'kernel-reads-native-'));
  const workspace = path.join(root, 'workspace'); await mkdir(workspace);
  const logoRequests = [];
  const server = createServer((request, response) => {
    if (request.url === '/logo.png') {
      logoRequests.push(Boolean(/session=abc/.test(request.headers.cookie ?? '')));
      if (!/session=abc/.test(request.headers.cookie ?? '')) { response.statusCode = 403; return response.end('no session'); }
      response.setHeader('Content-Type', 'image/png'); return response.end(Buffer.from(LOGO, 'base64'));
    }
    response.setHeader('Content-Type', 'text/html'); response.setHeader('Set-Cookie', 'session=abc; Path=/'); response.end(PAGE);
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const appStorageRoot = path.join(root, 'app-storage'); await mkdir(appStorageRoot);
  const host = new KernelBrowserHost(path.join(root, 'browser'), { appStorageRoot });
  const agent = { focused_agent: true, observed_by: 'agent:reads-native' };
  try {
    const opened = await host.request({ op: 'open', url: `${origin}/`, ...agent });
    const binding = { tab_id: opened.tab_id, generation: opened.generation };
    let snapshot;
    for (let count = 0; count < 40; count++) {
      snapshot = (await host.request({ op: 'snapshot', ...binding, ...agent })).snapshot;
      if (snapshot.dom_nodes.some(node => node.attributes?.id === 'far')) break;
      await delay(100);
    }
    const node = id => snapshot.dom_nodes.find(candidate => candidate.attributes?.id === id).node_ref;
    const state = await host.request({ op: 'state', ...agent });
    const scale = state.viewport?.device_scale_factor ?? 1;
    const evaluate = async expression => { const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(opened.tab_id).target_id); return (await connection.send('Runtime.evaluate', { expression, returnByValue: true }, sessionId)).result.value; };

    // 1: region, element, full page.
    const clip = await host.request({ op: 'screenshot', ...binding, ...agent, clip: { x: 0, y: 0, width: 200, height: 100 } });
    assert.deepEqual(sizeOf(clip.data_base64), { width: 200 * scale, height: 100 * scale });
    const element = await host.request({ op: 'screenshot', ...binding, ...agent, node_ref: node('far') });
    assert.equal(await evaluate('Math.round(scrollY)') > 1000, true, 'the element was scrolled into view');
    assert.deepEqual(element.clip.height, 80);
    assert.deepEqual(sizeOf(element.data_base64).height, 80 * scale);
    const page = await host.request({ op: 'screenshot', ...binding, ...agent, full_page: true });
    assert(page.page_height >= 3100, `page_height ${page.page_height}`);
    assert.deepEqual(sizeOf(page.data_base64), { width: page.width, height: page.height });
    if (process.env.CHARIOX_READS_EVIDENCE) {
      await writeFile(path.join(process.env.CHARIOX_READS_EVIDENCE, 'native-clip.png'), Buffer.from(clip.data_base64, 'base64'));
      await writeFile(path.join(process.env.CHARIOX_READS_EVIDENCE, 'native-element.png'), Buffer.from(element.data_base64, 'base64'));
      await writeFile(path.join(process.env.CHARIOX_READS_EVIDENCE, 'native-full-page.png'), Buffer.from(page.data_base64, 'base64'));
    }

    // 2: save through the page session, by node and by URL.
    const roots = [workspace];
    const byNode = await host.request({ op: 'save', ...binding, ...agent, node_ref: node('logo'), path: 'saved/logo.png', _save_roots: roots });
    const byUrl = await host.request({ op: 'save', ...binding, ...agent, url: `${origin}/logo.png`, path: 'saved/again.png', _save_roots: roots });
    const expected = Buffer.from(LOGO, 'base64');
    for (const [saved, file] of [[byNode, 'saved/logo.png'], [byUrl, 'saved/again.png']]) {
      assert.deepEqual([saved.path, saved.bytes, saved.mime_type, saved.sha256], [file, expected.length, 'image/png', createHash('sha256').update(expected).digest('hex')]);
      assert.deepEqual(await readFile(path.join(workspace, file)), expected);
    }
    assert(logoRequests.length >= 2 && logoRequests.every(Boolean), 'the page cookie went with every fetch');
    await assert.rejects(host.request({ op: 'save', ...binding, ...agent, url: `${origin}/logo.png`, path: '../escape.png', _save_roots: roots }), { code: 'user_domain_not_granted' });

    // 3: scroll to a node.
    await evaluate('scrollTo(0, 0)');
    await host.request({ op: 'input', ...binding, ...agent, document_id: snapshot.document_id, input: { kind: 'scroll', target: node('far') } });
    assert.equal(await evaluate('Math.round(document.getElementById("far").getBoundingClientRect().top) < innerHeight'), true);

    // 4: wait.
    const later = await host.request({ op: 'wait', ...binding, ...agent, text: 'appeared LATER', timeout_ms: 5000 });
    assert.equal(later.matched, true);
    const pushed = await host.request({ op: 'wait', ...binding, ...agent, url_contains: '/pushed/', timeout_ms: 5000 });
    assert.equal(pushed.matched, true);
    const never = await host.request({ op: 'wait', ...binding, ...agent, text: 'never on this page', timeout_ms: 700 });
    assert.equal(never.matched, false);
    console.log('agent reads native:', JSON.stringify({ scale, clip: sizeOf(clip.data_base64), element: element.clip, page: { width: page.width, height: page.height, page_height: page.page_height, mime: page.mime_type }, saved: [byNode.bytes, byUrl.bytes], waited: [later.waited_ms, pushed.waited_ms, never.waited_ms] }));
  } finally {
    await host.stop();
    if (host.chromium.browserUser) await rm(host.chromium.browserUser.root, { recursive: true, force: true });
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
});
