import assert from 'node:assert/strict';
import test from 'node:test';
import * as browser from './browser-relay-crypto.js';
import * as native from './relay-crypto.js';

test('browser pairing fingerprints match the kernel encoded-key contract', async () => {
  // Literal shared with terminal_pairings.rs, not a locally recomputed hash.
  assert.equal(await browser.relayPublicKeyThumbprint('public-key'),
    '43a46f1d081d270130e2210a1de59f9715de033307d068edc65a335b27e95d3d');
});

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

test('paired decryption rejects an authentic envelope from a different sender', async () => {
  const client = await browser.createRelayKeypair();
  const kernel = await browser.createRelayKeypair();
  const impostor = native.encryptRelayPayload(client.publicKeyBase64, Buffer.from('forged-consent'));
  // Valid encryption to the client's public key alone is not kernel identity.
  await assert.rejects(browser.decryptRelayPayload(client.privateKey, impostor.payload,
    kernel.publicKeyBase64), { message: 'relay sender identity mismatch' });
  const legitimate = await browser.encryptRelayPayload(client.publicKeyBase64, 'source_authorized', kernel);
  assert.equal(await browser.decryptRelayPayload(client.privateKey, legitimate.payload,
    kernel.publicKeyBase64), 'source_authorized');
  // Claiming the trusted public key without its private key must still fail GCM.
  await assert.rejects(browser.decryptRelayPayload(client.privateKey,
    { ...impostor.payload, sender_public_key: kernel.publicKeyBase64 }, kernel.publicKeyBase64));
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

test('MP-08/MP-10 protocol 466 display events expose raw contiguous payload views', () => {
  const encode = (header: unknown, payload: number[]) => {
    const json = new TextEncoder().encode(JSON.stringify(header))
    const bytes = new Uint8Array(8 + json.length + payload.length)
    bytes.set([0x43, 0x58, 0x44, 0x31])
    new DataView(bytes.buffer).setUint32(4, json.length)
    bytes.set(json, 8)
    bytes.set(payload, 8 + json.length)
    return bytes
  }
  const frame = (tiles: unknown[]) => ({ event: 'kernel_browser_frame', subscription_id: 's', frame: { kind: 'tiles', moves: [[0, 1, 4, 1, -1]], tiles } })
  const event = browser.decodeDisplayEvent(encode(frame([{ format: 'webp', data: [0, 3] }, { format: 'png', data: [3, 1] }]), [1, 2, 3, 4]))
  const tiles = (event?.frame as { tiles: { data: Uint8Array }[] }).tiles
  assert.deepEqual([...tiles[0]!.data], [1, 2, 3])
  assert.deepEqual([...tiles[1]!.data], [4])
  assert.equal(browser.decodeDisplayEvent(new TextEncoder().encode('{"event":"terminal_output"}')), null)
  for (const [tiles, payload] of [
    [[{ data: [1, 2] }], [0, 1, 2]],
    [[{ data: [0, 2] }], [0, 1, 2]],
    [[{ data: [0, 4] }], [0, 1, 2]],
    [[{ data: [0, 0] }], []],
  ] as const) {
    assert.throws(() => browser.decodeDisplayEvent(encode(frame([...tiles]), [...payload])), /display/)
  }
  assert.throws(() => browser.decodeDisplayEvent(encode({ event: 'other', frame: {} }, [])), /display event/)
})


test('MP-08/MP-10/MP-11 peer96 binary relay events retain authenticated ciphertext and reject bad bounds', async () => {
  const receiver = await browser.createRelayKeypair();
  const sender = native.createRelayKeypair();
  const payload = new native.RelayClientIdentity(sender.privateKey).encrypt(receiver.publicKeyBase64, Buffer.from('{"event":"terminal_output"}'));
  const raw = Buffer.from(payload.ciphertext, 'base64');
  const header = Buffer.from(JSON.stringify({ kind:'client_event', subscription_id:'s', event_id:17, sender_public_key:payload.sender_public_key, nonce:payload.nonce }));
  const prefix = Buffer.alloc(8); prefix.write('CXR1'); prefix.writeUInt32BE(header.length, 4);
  const wire = Buffer.concat([prefix, header, raw]);
  const decoded = browser.decodeBinaryRelayEvent(wire);
  assert.equal(decoded.event_id, 17);
  assert.equal(decoded.encrypted_event.ciphertext instanceof Uint8Array, true);
  assert.deepEqual(await browser.decryptRelayEvent(receiver.privateKey, decoded.encrypted_event, sender.publicKeyBase64), { event:'terminal_output' });
  await assert.rejects(browser.decryptRelayEvent(receiver.privateKey, decoded.encrypted_event, 'wrong-sender'), /sender identity/);
  const corrupted = wire.slice(); corrupted[corrupted.length-1] = corrupted[corrupted.length-1]! ^ 1;
  await assert.rejects(browser.decryptRelayEvent(receiver.privateKey, browser.decodeBinaryRelayEvent(corrupted).encrypted_event, sender.publicKeyBase64));
  const excessive = wire.slice(); excessive.writeUInt32BE(4097,4);
  assert.throws(() => browser.decodeBinaryRelayEvent(excessive), /bounds/);
  assert.throws(() => browser.decodeBinaryRelayEvent(wire.subarray(0,8+header.length+15)), /bounds/);
});


test('MP-08/MP-10 peer96 browser negotiation retries the existing connection on a legacy relay', async () => {
  const original = globalThis.WebSocket;
  const offers: unknown[][] = [];
  class Socket {
    onopen?: () => void; onerror?: () => void; binaryType=''; protocol=''; closed=false;
    constructor(url:string, protocols?:string[]) {
      offers.push([url,protocols]);
      queueMicrotask(() => { if(protocols)this.onerror?.();else this.onopen?.() });
    }
    close(){this.closed=true}
  }
  globalThis.WebSocket = Socket as unknown as typeof WebSocket;
  try {
    const socket = await browser.connectRelaySocket('wss://example/relay');
    assert.equal(socket.protocol, '');
    assert.deepEqual(offers, [['wss://example/relay',['chariox-relay-binary-v96']],['wss://example/relay',undefined]]);
    assert.equal(socket.binaryType,'arraybuffer');
  } finally { globalThis.WebSocket=original }
});
