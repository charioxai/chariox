// Opt-in: real sandboxed Chromium and the production host/CDP adapter.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { KernelBrowserHost } from './kernel-browser-host.mjs';
assert.ok(process.env.CHARIOX_KERNEL_BROWSER_EXECUTABLE, 'Explicit installed Chromium required; never download a browser');

async function withNative(run) {
  const root = await mkdtemp(path.join(tmpdir(), 'cx-refusal-native-'));
  const server = createServer((_req, res) => { res.setHeader('Content-Type', 'text/html'); res.end('<!doctype html><input id="field"><p>Public disposable fixture</p>'); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const url = `http://127.0.0.1:${server.address().port}/`;
  const host = new KernelBrowserHost(root);
  try {
    const opened = await host.request({ op: 'open', url });
    const tab = host.tabs.get(opened.tab_id);
    const { connection, sessionId } = await host.browser.resolvePageTarget(tab.target_id);
    await run({ host, opened, tab, connection, sessionId, url });
  } finally {
    await host.stop();
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true });
  }
}
function assertStale(reply) {
  assert.equal(reply.ok, false);
  assert.deepEqual(reply.error, { code: 'user_domain_stale_reference', message: 'User-domain request refused' });
}
test('native detached observed fill node survives production CDP normalization', () => withNative(async ({host,opened,tab,connection,sessionId,url}) => {
  const { root } = await connection.send('DOM.getDocument', {}, sessionId);
  const { nodeId } = await connection.send('DOM.querySelector', { nodeId: root.nodeId, selector: '#field' }, sessionId);
  assert.ok(nodeId);
  const { node } = await connection.send('DOM.describeNode', { nodeId }, sessionId);
  await connection.send('DOM.removeNode', { nodeId }, sessionId);
  await host.protect({ values: ['disposable-never-delivered'], targets: [], unknown: false });
  assertStale(await host.handle({ id: 1, method: 'host.secret', params: {
    tab_id: opened.tab_id, generation: opened.generation, document_id: tab.document_id,
    node_ref: `backend:${node.backendNodeId}`,
    action: { kind: 'fill', text: 'disposable-never-delivered', expected_document_url: url },
  } }));
}));
test('native navigation race survives production CDP normalization', () => withNative(async ({host,opened,connection,sessionId,url}) => {
  const nativeNavigate = host.browser.navigate.bind(host.browser);
  // Inject an actual document change between host observation and the adapter's
  // validation; the production navigate implementation still performs the call.
  host.browser.navigate = async (observed, options) => {
    await connection.send('Page.navigate', { url: url + '?replacement' }, sessionId);
    const deadline = Date.now() + 5000;
    while ((await connection.send('Page.getFrameTree', {}, sessionId)).frameTree.frame.loaderId === observed.document_id) {
      assert.ok(Date.now() < deadline, 'Fixture navigation did not commit'); await delay(10);
    }
    return nativeNavigate(observed, options);
  };
  assertStale(await host.handle({ id: 2, method: 'host.browser', params: {
    op: 'navigate', tab_id: opened.tab_id, generation: opened.generation, url: url + '?requested',
  } }));
}));
