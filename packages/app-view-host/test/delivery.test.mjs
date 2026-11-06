// MP-08 / MP-09 / MP-10 / MP-11 — supplemental authenticated-delivery regressions.
import test from 'node:test';
import assert from 'node:assert/strict';
import { AppViewCache, loadClientAppView, MAX_CHUNK_BYTES } from '../delivery.mjs';
import { packView } from '../pack.mjs';
import { base64, unbase64 } from '../bundle.mjs';

async function fixture() {
  const key = await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify']);
  const packed = await packView({ schema: 'chariox.app.v2', appContractVersion: 2,
    publisher: { id: 'com.example', keyId: 'test' }, ui: { entry: 'ui/index.html' } },
  { schema: 'chariox.app-view-source.v1', rootId: 'root', script: '/*' + 'x'.repeat(100000) + '*/', style: '' }, key);
  const bytes = unbase64(packed.bundle.files[0].body_base64);
  const descriptor = { view_id: 'client-app-' + 'a'.repeat(32), installation_id: 'installed', generation: '1',
    package_digest: 'sha256:' + '1'.repeat(64), trust: packed.trust,
    bundle: { manifest: packed.bundle.manifest, inventory: packed.bundle.inventory, signature: packed.bundle.signature } };
  const requests = [];
  const rpc = async ({ ClientAppView: command }) => {
    requests.push(command);
    if (command.Open) return { ClientAppView: { Opened: descriptor } };
    if (command.Check) return { ClientAppView: { Valid: { view_id: descriptor.view_id, generation: descriptor.generation } } };
    if (command.Close) return { ClientAppView: { Closed: { view_id: descriptor.view_id } } };
    const { offset, max_bytes } = command.Read;
    assert(max_bytes > 0 && max_bytes <= MAX_CHUNK_BYTES);
    const end = Math.min(offset + max_bytes, bytes.length);
    return { ClientAppView: { Chunk: { view_id: descriptor.view_id, offset, body_base64: base64(bytes.slice(offset, end)), eof: end === bytes.length } } };
  };
  return { rpc, descriptor, requests, bytes, kernelOwner: 'authenticated-kernel:owner', installationId: 'installed' };
}

test('serial bounded chunks verify before mounting; cache still needs a fresh open and check', async () => {
  const f = await fixture(), cache = new AppViewCache();
  const loaded = await loadClientAppView({ ...f, cache });
  assert(f.requests.filter(row => row.Read).length > 1);
  assert.deepEqual(unbase64(loaded.bundle.files[0].body_base64), f.bytes);
  await loaded.close();
  f.requests.length = 0;
  await (await loadClientAppView({ ...f, cache })).close();
  assert.deepEqual(f.requests.map(row => Object.keys(row)[0]), ['Open', 'Check', 'Close']);
  f.requests.length = 0;
  await (await loadClientAppView({ ...f, cache, kernelOwner: 'other-owner' })).close();
  assert(f.requests.some(row => row.Read));
  cache.purge(f.kernelOwner, f.installationId);
  f.requests.length = 0;
  await (await loadClientAppView({ ...f, cache })).close();
  assert(f.requests.some(row => row.Read));
});

for (const failure of ['offset', 'hash', 'eof', 'generation', 'signature', 'size']) {
  test(`${failure} rejects and closes delivery before returning a mountable bundle`, async () => {
    const f = await fixture();
    if (failure === 'signature') f.descriptor.bundle.signature.value = base64(new Uint8Array(64));
    if (failure === 'size') f.descriptor.bundle.inventory = JSON.stringify({ files: [{ path: 'ui/index.html', size: 17 * 1024 * 1024 }] });
    const rpc = async request => {
      const result = await f.rpc(request);
      const { Chunk: chunk, Valid: valid } = result.ClientAppView;
      if (chunk && failure === 'offset') chunk.offset++;
      if (chunk && failure === 'hash') chunk.body_base64 = base64(new Uint8Array(unbase64(chunk.body_base64).length));
      if (chunk && failure === 'eof') chunk.eof = !chunk.eof;
      if (valid && failure === 'generation') valid.generation = '2';
      return result;
    };
    await assert.rejects(loadClientAppView({ ...f, rpc }));
    assert(f.requests.at(-1).Close);
  });
}

test('bounded LRU copies bytes and evicts old entries', () => {
  const cache = new AppViewCache(3), bytes = new Uint8Array([1, 2]);
  cache.set('a', bytes); bytes[0] = 9;
  assert.equal(cache.get('a')[0], 1);
  const read = cache.get('a'); read[0] = 9;
  assert.equal(cache.get('a')[0], 1);
  cache.set('b', new Uint8Array([3, 4]));
  assert.equal(cache.get('a'), null);
  cache.set('large', new Uint8Array(4));
  assert.equal(cache.get('large'), null);
});
