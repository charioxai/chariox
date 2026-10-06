// Exercise production CDP action/navigation normalization, not stubbed browser methods.
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { BrowserCdpClient, BrowserControllerError } from './browser-controller-cdp.mjs';
import { KernelBrowserHost } from './kernel-browser-host.mjs';

async function withAdapter(run) {
  const root = await mkdtemp(path.join(os.tmpdir(), 'cx-refusal-cdp-'));
  const wire = { document: 'observed', resolveError: null, calls: [],
    isOpen: () => true, subscribe: () => () => {}, close: async () => {},
    async send(method) {
      this.calls.push(method);
      if (method === 'Target.getTargets') return { targetInfos: [{ targetId: 'target', type: 'page', url: 'https://fixture.invalid' }] };
      if (method === 'Target.attachToTarget') return { sessionId: 'session' };
      if (method === 'Page.getFrameTree') return { frameTree: { frame: { id: 'frame', loaderId: this.document } } };
      if (method === 'DOMSnapshot.captureSnapshot') return {strings:['frame'],documents:[{frameId:0,nodes:{backendNodeId:[42]}}]};
      if (method === 'Page.createIsolatedWorld') return {executionContextId:7};
      if (method === 'DOM.resolveNode' && this.resolveError) throw this.resolveError;
      return {}; // A detached backend node resolves to no object.
    },
  };
  const browser = new BrowserCdpClient({ connectionFactory: async () => wire });
  const host = new KernelBrowserHost(root);
  // Host admission has already selected the observed tab. The browser adapter,
  // document validation, backend-node resolution and normalization remain real.
  host.browser = browser; host.generation = 1;
  host.start = async () => {};
  host.target = async () => ({ tab_id: 'host-tab', target_id: 'target', document_id: 'observed' });
  try { await run(host, wire); } finally { await browser.close(); await rm(root, { recursive: true, force: true }); }
}
const fill = { method: 'host.secret', id: 1, params: {
  tab_id: 'host-tab', generation: 1, document_id: 'observed', node_ref: 'backend:42',
  action: { kind: 'fill', text: 'disposable-fixture', expected_document_url: 'https://fixture.invalid' },
} };
const stale = { code: 'user_domain_stale_reference', message: 'User-domain request refused' };

test('detached secure-fill node keeps trusted provenance through real performAction normalization', () => withAdapter(async (host, wire) => {
  const reply = await host.handle(fill);
  assert.ok(wire.calls.includes('DOM.resolveNode'));
  assert.deepEqual(reply.error, stale);
}));
test('changed navigation document keeps trusted provenance through real navigate normalization', () => withAdapter(async (host, wire) => {
  wire.document = 'replacement';
  const reply = await host.handle({ id: 2, method: 'host.browser', params: {
    op: 'navigate', tab_id: 'host-tab', generation: 1, url: 'https://fixture.invalid/next',
  } });
  assert.ok(wire.calls.includes('Page.getFrameTree'));
  assert.deepEqual(reply.error, stale);
}));
for (const forged of [
  Object.assign(new Error('lookalike'), { code: 'stale_element_reference' }),
  new BrowserControllerError('stale_element_reference', 'lookalike'),
]) test('untrusted matching code is not a stale-reference refusal: ' + forged.name, () => withAdapter(async (host, wire) => {
  wire.resolveError = forged;
  const reply = await host.handle(fill);
  assert.ok(wire.calls.includes('DOM.resolveNode'));
  assert.equal(reply.error.code, 'kernel_browser_failed');
}));
