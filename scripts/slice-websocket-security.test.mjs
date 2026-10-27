import assert from 'node:assert/strict';
import { once } from 'node:events';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import test from 'node:test';

// Install only the locked ws package into an external disposable directory, then
// set CHARIOX_TEST_WS_ROOT to that directory. Never install provider CLIs for this drill.
const root = process.env.CHARIOX_TEST_WS_ROOT;
const skip = root ? false : 'set CHARIOX_TEST_WS_ROOT to an external locked ws installation';

async function fixture(t, options = {}) {
  const require = createRequire(resolve(root, 'package.json'));
  const manifest = JSON.parse(await readFile(new URL('../apps/kernel/slice-linux-docker/toolchain/package.json', import.meta.url)));
  assert.equal(require('ws/package.json').version, manifest.dependencies.ws);
  const {WebSocket, WebSocketServer} = require('ws');
  const server = new WebSocketServer({host: '127.0.0.1', port: 0, ...options});
  let client;
  t.after(async () => {
    client?.terminate();
    for (const socket of server.clients) socket.terminate();
    await new Promise(resolve => server.close(resolve));
  });
  await once(server, 'listening', {signal: AbortSignal.timeout(2000)});
  const connected = once(server, 'connection', {signal: AbortSignal.timeout(2000)});
  client = new WebSocket(`ws://127.0.0.1:${server.address().port}`);
  client.on('error', () => {});
  const opened = once(client, 'open', {signal: AbortSignal.timeout(2000)});
  const [peer] = await connected;
  await opened;
  return {client, peer};
}

test('locked slice WebSocket preserves text and binary messages', {skip, timeout: 5000}, async t => {
  const {client, peer} = await fixture(t);
  peer.on('message', (data, binary) => peer.send(data, {binary}));
  for (const value of ['browser command', Buffer.from([0, 1, 255])]) {
    const reply = once(client, 'message', {signal: AbortSignal.timeout(2000)});
    client.send(value);
    const [data, binary] = await reply;
    assert.deepEqual(data, Buffer.from(value));
    assert.equal(binary, Buffer.isBuffer(value));
  }
});

test('locked slice WebSocket rejects fragment amplification within a bounded input', {skip, timeout: 5000}, async t => {
  const {client, peer} = await fixture(t, {maxFragments: 4});
  const errors = [];
  peer.on('error', error => errors.push(error.code));
  const closed = new Promise(resolve => client.once('close', code => resolve(code)));
  // Six one-byte fragments prove the configured bound without reproducing OOM.
  for (let index = 0; index < 6; index++) client.send('x', {fin: index === 5});
  // ws reports this structural limit as policy violation, not payload size.
  assert.equal(await closed, 1008);
  assert.deepEqual(errors, ['WS_ERR_TOO_MANY_BUFFERED_PARTS']);
});
