import assert from 'node:assert/strict';
import test from 'node:test';
import * as browser from './browser-relay-crypto.js';
import * as native from './relay-crypto.js';

test('a paired browser sender keeps its identity across consent requests', async () => {
  const sender = await browser.createRelayKeypair();
  const recipient = native.createRelayKeypair();
  const first = await browser.encryptRelayPayload(recipient.publicKeyBase64, 'prepare', sender);
  const second = await browser.encryptRelayPayload(recipient.publicKeyBase64, 'approve', sender);

  assert.equal(first.payload.sender_public_key, sender.publicKeyBase64);
  assert.equal(second.payload.sender_public_key, sender.publicKeyBase64);
  assert.notEqual(first.payload.nonce, second.payload.nonce);
  assert.equal(native.decryptRelayPayload(recipient.privateKey, first.payload), 'prepare');
  assert.equal(native.decryptRelayPayload(recipient.privateKey, second.payload), 'approve');
});

test('browser relay private keys cannot be exported for storage or logging', async () => {
  const sender = await browser.createRelayKeypair();
  assert.equal(Buffer.from(sender.publicKeyBase64, 'base64').length, 65);
  assert.equal(sender.privateKey.extractable, false);
  await assert.rejects(crypto.subtle.exportKey('jwk', sender.privateKey));
  await assert.rejects(crypto.subtle.exportKey('pkcs8', sender.privateKey));
});

test('ordinary requests keep independent ephemeral sender identities', async () => {
  const recipient = native.createRelayKeypair();
  const first = await browser.encryptRelayPayload(recipient.publicKeyBase64, 'first');
  const second = await browser.encryptRelayPayload(recipient.publicKeyBase64, 'second');
  assert.notEqual(first.payload.sender_public_key, second.payload.sender_public_key);
  assert.equal(native.decryptRelayPayload(recipient.privateKey, first.payload), 'first');
  assert.equal(native.decryptRelayPayload(recipient.privateKey, second.payload), 'second');
});

test('concurrent paired requests use fresh nonces and decrypt native replies', async () => {
  const sender = await browser.createRelayKeypair();
  const recipient = native.createRelayKeypair();
  const requests = await Promise.all(Array.from({ length: 32 }, (_, index) =>
    browser.encryptRelayPayload(recipient.publicKeyBase64, `request-${index}`, sender)));

  assert.equal(new Set(requests.map(({ payload }) => payload.nonce)).size, 32);
  for (const [index, { keypair, payload }] of requests.entries()) {
    assert.equal(payload.sender_public_key, sender.publicKeyBase64);
    assert.equal(Buffer.from(payload.nonce, 'base64').length, 12);
    assert.equal(native.decryptRelayPayload(recipient.privateKey, payload), `request-${index}`);
    const reply = native.encryptRelayPayload(payload.sender_public_key, Buffer.from(`reply-${index}`));
    assert.equal(await browser.decryptRelayPayload(keypair.privateKey, reply.payload), `reply-${index}`);
  }
});

test('browser encryption and native decryption preserve the existing relay envelope', async () => {
  const recipient = native.createRelayKeypair();
  const encrypted = await browser.encryptRelayPayload(recipient.publicKeyBase64, 'fixture: café ✓');
  assert.deepEqual(Object.keys(encrypted.payload).sort(), ['ciphertext','nonce','sender_public_key']);
  assert.equal(native.decryptRelayPayload(recipient.privateKey,encrypted.payload),'fixture: café ✓');
});

test('native encryption and browser decryption use the same relay contract', async () => {
  const recipient = await browser.createRelayKeypair();
  const encrypted = native.encryptRelayPayload(recipient.publicKeyBase64,Buffer.from('fixture-response'));
  assert.equal(await browser.decryptRelayPayload(recipient.privateKey,encrypted.payload),'fixture-response');
});

test('ciphertext modification and wrong recipient keys are rejected', async () => {
  const recipient = await browser.createRelayKeypair();
  const encrypted = await browser.encryptRelayPayload(recipient.publicKeyBase64,'fixture-secret');
  const bytes = Buffer.from(encrypted.payload.ciphertext,'base64');
  bytes[0] = bytes[0]! ^ 1;
  await assert.rejects(browser.decryptRelayPayload(recipient.privateKey,{
    ...encrypted.payload,ciphertext:bytes.toString('base64'),
  }));
  const other = await browser.createRelayKeypair();
  await assert.rejects(browser.decryptRelayPayload(other.privateKey,encrypted.payload));
});
