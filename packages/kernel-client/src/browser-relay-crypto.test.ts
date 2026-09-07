import assert from 'node:assert/strict';
import test from 'node:test';
import * as browser from './browser-relay-crypto.js';
import * as native from './relay-crypto.js';

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
