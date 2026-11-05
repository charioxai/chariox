import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createPrivateKey, createPublicKey } from 'node:crypto';
import { base64, canonical, digest, encodeView, signingBytes, verifyView } from '../bundle.mjs';
import { packView } from '../pack.mjs';

const manifest = { schema: 'chariox.app.v2', appContractVersion: 2, publisher: { id: 'com.example', keyId: 'test' }, ui: { entry: 'ui/index.html' } };
const source = { schema: 'chariox.app-view-source.v1', rootId: 'root', script: 'globalThis.executed = true;', style: '' };
const keyPair = await crypto.subtle.generateKey('Ed25519', true, ['sign', 'verify']);

test('browser signing subject and signature agree with the real Rust package golden vector', async () => {
  const vector = JSON.parse(await readFile(new URL('../../app-package/tests/vectors/v2/package.json', import.meta.url)));
  const expected = vector.cases[0].expected;
  const manifest = JSON.parse(expected.manifestBytes), inventory = JSON.parse(expected.inventoryBytes);
  const subject = signingBytes(manifest, inventory);
  assert.equal(new TextDecoder().decode(subject), expected.signingSubject);
  assert.equal(await digest(subject), expected.contentDigest);
  // Public test seed from the Rust vector producer, never a product identity.
  const privateKey = createPrivateKey({ key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), Buffer.alloc(32, 7)]), format: 'der', type: 'pkcs8' });
  const publicBytes = createPublicKey(privateKey).export({ format: 'der', type: 'spki' }).subarray(-32);
  const publicKey = await crypto.subtle.importKey('raw', publicBytes, 'Ed25519', false, ['verify']);
  assert(await crypto.subtle.verify('Ed25519', publicKey, Buffer.from(expected.signatureValue, 'base64'), subject));
});

test('only the authenticated publisher, signing subject and exact signed entry are executable', async () => {
  const packed = await packView(manifest, source, keyPair);
  assert.deepEqual(await verifyView(packed.bundle, packed.trust), source);
  const cases = [
    ['entry bytes', p => { p.bundle.files[0].body_base64 = base64(encodeView({ ...source, script: 'globalThis.attacker = true;' })); }],
    ['signature', p => { p.bundle.signature.value = base64(new Uint8Array(64)); }],
    ['publisher', p => { p.bundle.manifest = canonical({ ...manifest, publisher: { ...manifest.publisher, id: 'attacker' } }); }],
    ['self-asserted key', p => { p.bundle.publicKey = p.trust.publicKey; p.trust.publicKey = base64(new Uint8Array(32)); }],
    ['inventory', p => { const i = JSON.parse(p.bundle.inventory); i.files[0].digest = 'sha256:' + '0'.repeat(64); p.bundle.inventory = canonical(i); }],
    ['missing entry', p => { p.bundle.files = []; }],
    ['extra entry', p => { p.bundle.files.push(p.bundle.files[0]); }],
    ['duplicate control key', p => { p.bundle.manifest = p.bundle.manifest.replace('"appContractVersion":2', '"appContractVersion":1,"appContractVersion":2'); }],
  ];
  for (const [name, corrupt] of cases) {
    const p = structuredClone(packed); corrupt(p);
    await assert.rejects(verifyView(p.bundle, p.trust), undefined, name);
  }
  await assert.rejects(verifyView(packed.bundle, null), /authenticated/);
});

test('source carrier cannot terminate a raw-text HTML element', () => {
  const bytes = encodeView({ ...source, script: 'console.log("</script><script>alert(1)</script>");', style: 'body:before{content:"</style><img src=x>"}' });
  const text = new TextDecoder().decode(bytes);
  assert.equal((text.match(/<script/g) ?? []).length, 1);
  assert.equal((text.match(/<\/script/g) ?? []).length, 1);
});

test('reference-size source verifies without regular-expression stack exhaustion', async () => {
  const large = { ...source, script: '// ' + 'x'.repeat(4 * 1024 * 1024) };
  const packed = await packView(manifest, large, keyPair);
  assert.equal((await verifyView(packed.bundle, packed.trust)).script.length, large.script.length);
});

test('canonical controls reject malformed Unicode and bound JSON recursion like the package verifier', () => {
  for (const value of ['\ud800', '\udfff', { '\ud800': true }, NaN, Infinity, 2 ** 53]) {
    assert.throws(() => canonical(value), /interoperable|limit/);
  }
  let nested = null;
  for (let i = 0; i < 34; i++) nested = [nested];
  assert.throws(() => canonical(nested), /limit/);
  assert.equal(canonical({ '\u{1f600}': 'ok', '\r': '\u20ac' }), '{"\\r":"€","😀":"ok"}');
});

test('the producer rejects a source whose encoded carrier exceeds the delivery budget', () => {
  assert.throws(() => encodeView({ ...source, script: 'x'.repeat(13 * 1024 * 1024) }), /limit/);
});

test('a full signed inventory may include portable license names outside the delivered entry', async () => {
  const packed = await packView(manifest, source, keyPair,
    new Map([['licenses/@scope/LICENSE (MIT).txt', new TextEncoder().encode('MIT notice')]]));
  assert.equal(packed.bundle.files.length, 1);
  assert.deepEqual(await verifyView(packed.bundle, packed.trust), source);
});
