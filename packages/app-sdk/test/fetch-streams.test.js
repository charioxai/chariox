import assert from 'node:assert/strict';
import { test } from 'node:test';
import { gzipSync, deflateSync, brotliCompressSync } from 'node:zlib';
import { fixture } from './fetch-helpers.js';
import { deferred, flush } from './helpers.js';
import { limitBytes } from '../src/fetch-response.js';
const URL = 'https://api.example.test/stream';

test('gzip, deflate, Brotli and chained encodings decode across broker chunk boundaries', async () => {
  const plain = Buffer.from('data: cafè\n\n'.repeat(40));
  for (const [encoding, compressed] of [
    ['gzip', gzipSync(plain)], ['deflate', deflateSync(plain)], ['br', brotliCompressSync(plain)],
    ['gzip, br', brotliCompressSync(gzipSync(plain))],
  ]) {
    const f = fixture(() => ({ headers: [['content-encoding', encoding], ['content-length', String(compressed.length)]],
      chunks: [...compressed].map(byte => Buffer.from([byte])) }));
    try {
      const result = await f.fetch(URL);
      assert.equal(result.headers.get('content-encoding'), encoding);
      assert.deepEqual(Buffer.from(await result.arrayBuffer()), plain);
      assert.equal([...f.streams.values()][0].cancelled, 1);
    } finally { f.close(); }
  }
});

test('invalid compression and decoded byte overflow cancel upstream instead of buffering indefinitely', async () => {
  for (const encoding of ['gzip', 'unsupported', 'gzip, gzip, gzip, gzip']) {
    const f = fixture(() => ({ headers: [['content-encoding', encoding]], chunks: ['bad compressed bytes'] }));
    try {
      await assert.rejects(async () => (await f.fetch(URL)).text());
      await flush();
      assert.equal([...f.streams.values()][0].cancelled, 1);
    } finally { f.close(); }
  }
  let cancelled = false;
  const source = new ReadableStream({ pull(c) { c.enqueue(new Uint8Array(5)); }, cancel() { cancelled = true; } }, { highWaterMark: 0 });
  await assert.rejects(new Response(source.pipeThrough(limitBytes(8))).arrayBuffer(), /byte limit/);
  await flush();
  assert(cancelled);
});

test('unconsumed responses retain bounded pulls; explicit body cancellation stops further reads', async () => {
  const f = fixture(() => ({ read(stream) { stream.read++; return { pending: false, done: false, chunkBase64: 'eA==' }; } }));
  try {
    const result = await f.fetch(URL);
    await flush(); await flush();
    const stream = [...f.streams.values()][0];
    assert(stream.read <= 2, `unbounded read-ahead: ${stream.read}`);
    await result.body.cancel();
    const reads = stream.read;
    await flush();
    assert.equal(stream.read, reads);
    assert.equal(stream.cancelled, 1);
  } finally { f.close(); }
});

test('upload backpressure retains one broker write and abort settles a blocked upload', async () => {
  const blocked = deferred(); const entered = deferred(); const head = deferred();
  const f = fixture(() => ({ headersWait: head.promise, write() { entered.resolve(); return blocked.promise; }, chunks: ['ok'] }));
  const abort = new AbortController();
  let pulls = 0;
  const body = new ReadableStream({ pull(c) { pulls++; c.enqueue(new Uint8Array(65536)); } }, { highWaterMark: 0 });
  try {
    const pending = f.fetch(URL, { method: 'POST', body, duplex: 'half', signal: abort.signal });
    await entered.promise;
    await flush();
    assert.equal(f.transport.sent.filter(message => message.method === 'http.write').length, 1);
    assert(pulls <= 2);
    abort.abort();
    await assert.rejects(pending, { name: 'AbortError' });
    await flush();
    assert.equal([...f.streams.values()][0].cancelled, 1);
    blocked.resolve(); head.resolve();
    await flush();
    assert.equal(f.transport.sent.filter(message => message.method === 'http.write').length, 1);
  } finally { f.close(); }
});

test('HEAD and null-body response statuses release handles without reading body chunks', async () => {
  for (const [method, status] of [['HEAD', 200], ['GET', 204], ['GET', 205], ['GET', 304]]) {
    const f = fixture(() => ({ status, chunks: ['must not be consumed'] }));
    try {
      const result = await f.fetch(URL, { method });
      assert.equal(result.body, null);
      assert.equal(await result.text(), '');
      assert.equal(f.transport.sent.filter(message => message.method === 'http.read').length, 0);
      assert.equal([...f.streams.values()][0].cancelled, 1);
    } finally { f.close(); }
  }
});

test('a completed early response cancels a blocked upload without turning its body into an error', async () => {
  const writing = deferred(); const blocked = deferred();
  const f = fixture(() => ({ headersWait: writing.promise, chunks: ['accepted'], write() { writing.resolve(); return blocked.promise; } }));
  try {
    const result = await f.fetch(URL, { method: 'POST', body: 'upload' });
    assert.equal(await result.text(), 'accepted');
    assert.equal([...f.streams.values()][0].cancelled, 1);
    assert.equal(f.transport.sent.filter(message => message.kind === 'cancel').length, 1);
    blocked.resolve();
    await flush();
    assert.equal(f.transport.sent.filter(message => message.method === 'http.write').length, 1);
  } finally { f.close(); }
});

test('a declared content-length mismatch cancels once and never sends caller-controlled framing', async () => {
  for (const length of ['2', '20']) {
    const f = fixture(params => { assert.equal(new Headers(params.headers).has('content-length'), false); return { waitUpload: true }; });
    try {
      await assert.rejects(f.fetch(URL, { method: 'POST', body: 'four', headers: { 'content-length': length } }), /content-length/);
      assert.equal([...f.streams.values()][0].cancelled, 1);
      assert(f.transport.sent.filter(message => message.method === 'http.write').length <= 1);
    } finally { f.close(); }
  }
});
