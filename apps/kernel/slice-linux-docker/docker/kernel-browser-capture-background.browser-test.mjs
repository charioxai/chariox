// Row 558: a QR capture reads a picture of the element (Page.captureScreenshot). On a Tab that is not the shown one and holds no
// background lease, Chrome renders nothing: the screenshot times out and every retry then fails with "screenshot presentation did
// not restore" (browser_controller_internal). An agent's Tab holds a lease only for 30 s after its last use, so the capture failed
// whenever the agent paused that long between its last browser call and the capture. captureSecret must hold the Tab itself.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, rm } from 'node:fs/promises';
import path from 'node:path';
import { KernelBrowserHost } from './kernel-browser-host.mjs';
import { HostChromium } from './kernel-browser-process.mjs';

assert(process.env.CHARIOX_MDACCESS_DRILL_ROOT, 'explicit disposable root required');
const PAGE = '<!doctype html><title>qr</title><svg id="code" width="120" height="120" viewBox="0 0 12 12"><rect width="12" height="12" fill="#fff"/><rect x="1" y="1" width="3" height="3"/><rect x="8" y="1" width="3" height="3"/><rect x="1" y="8" width="3" height="3"/><rect x="5" y="5" width="2" height="2"/></svg>';

async function withTabs(run) {
  const base = process.env.CHARIOX_MDACCESS_DRILL_ROOT;
  const root = await mkdtemp(path.join(base, 'qrhold-')), apps = await mkdtemp(path.join(base, 'qrhold-apps-'));
  const server = createServer((request, response) => { response.setHeader('Content-Type', 'text/html'); response.end(PAGE + `<!-- ${request.url} -->`); });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const host = new KernelBrowserHost(root, { appStorageRoot: apps, chromium: new HostChromium(root, { appStorageRoot: apps, environment: process.env }) });
  const wait = async (tab, expression) => { for (let i = 0; i < 200; i++) { if (await evaluate(tab, expression).catch(() => false)) return; await new Promise(resolve => setTimeout(resolve, 25)); } };
  const evaluate = async (tab, expression) => {
    const { connection, sessionId } = await host.browser.resolvePageTarget(host.tabs.get(tab.tab_id).target_id);
    const reply = await connection.send('Runtime.evaluate', { expression, returnByValue: true }, sessionId);
    return reply.result.value;
  };
  const open = async url => { const tab = await host.request({ op: 'open', url }); await wait(tab, "document.readyState === 'complete' && location.protocol === 'http:'"); return tab; };
  const qr = async tab => {
    const stored = host.tabs.get(tab.tab_id);
    const { connection, sessionId } = await host.browser.resolvePageTarget(stored.target_id);
    const doc = await connection.send('DOM.getDocument', { depth: 0 }, sessionId);
    const found = await connection.send('DOM.querySelector', { nodeId: doc.root.nodeId, selector: '#code' }, sessionId);
    const node = (await connection.send('DOM.describeNode', { nodeId: found.nodeId }, sessionId)).node.backendNodeId;
    // The kernel's own entry: host.handle, as the Rust side calls it for chariox.vault.capture_browser (read=qr).
    const reply = await host.handle({ id: 1, method: 'host.secret', params: { op: 'capture', tab_id: tab.tab_id, generation: host.generation, document_id: stored.document_id,
      node_ref: `backend:${node}`, reveal_node_ref: null, read: 'qr', expected_document_url: await evaluate(tab, 'location.href') } });
    if (!reply.ok) throw new Error(`${reply.error?.code}: ${reply.error?.message}`);
    return reply.result.value;
  };
  try { await run({ host, open, qr, origin, evaluate }); }
  finally {
    await host.stop();
    if (host.chromium.browserUser) await rm(host.chromium.browserUser.root, { recursive: true, force: true });
    await new Promise(resolve => server.close(resolve));
    await rm(root, { recursive: true, force: true }); await rm(apps, { recursive: true, force: true });
  }
}
const isPng = value => typeof value === 'string' && Buffer.from(value, 'base64').subarray(0, 4).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47]));

test('row 558: a QR capture works on a Tab behind the shown one that holds no background lease', () => withTabs(async ({ open, qr, origin, evaluate }) => {
  const behind = await open(`${origin}/qr`);
  await open(`${origin}/other`); // the shown Tab is now the second one
  assert.equal(await evaluate(behind, 'document.visibilityState'), 'hidden', 'the Tab under test is not the shown one');
  assert.ok(isPng(await qr(behind)), 'the QR picture is returned as a PNG');
}));

test('row 558: a QR capture works after the Tab\'s 30 s background lease expired', () => withTabs(async ({ host, open, qr, origin }) => {
  const behind = await open(`${origin}/qr`);
  await open(`${origin}/other`);
  const stored = host.tabs.get(behind.tab_id);
  await host.holdBackground(stored); await host.releaseBackground(stored.target_id); // what the idle timer does 30 s after the agent's last call
  assert.ok(isPng(await qr(behind)), 'the QR picture is returned as a PNG');
  assert.ok(isPng(await qr(behind)), 'and again: a first failure must not poison the next capture');
}));
