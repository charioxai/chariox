import assert from 'node:assert/strict';
import { test } from 'node:test';
import { readFileSync } from 'node:fs';
import { fixture } from './fetch-helpers.js';
import { deferred, flush } from './helpers.js';
const URL = 'https://api.example.test/resource';
const contract = JSON.parse(readFileSync(new globalThis.URL('./fetch-contract.json', import.meta.url)));

test('Fetch preserves native response values and stream metadata through clone', async () => {
  const f = fixture(() => ({ chunks: ['{"ok":', 'true}'], headers: [['content-type', 'application/json']] }));
  try {
    const result = await f.fetch(URL);
    assert(result instanceof Response);
    assert(result.body instanceof ReadableStream);
    assert.equal(result.url, URL);
    assert.equal(result.redirected, false);
    assert.equal(result.bodyUsed, false);
    assert.throws(() => result.headers.set('x-change', 'no'), /immutable/);
    const clone = result.clone();
    assert.equal(clone.url, URL);
    assert.deepEqual({ status: result.status, url: result.url, redirected: result.redirected,
      type: result.type, body: await result.json() }, contract.response);
    assert.equal(await clone.text(), '{"ok":true}');
    assert.equal(result.bodyUsed, true);
    await assert.rejects(result.text(), TypeError);
    assert.equal([...f.streams.values()][0].cancelled, 1);
  } finally { f.close(); }
});

test('multipart and URLSearchParams use native serialization with bounded broker chunks', async () => {
  const f = fixture(() => ({ waitUpload: true, chunks: ['ok'] }));
  try {
    const form = new FormData();
    form.append('title', 'cafè');
    form.append('file', new Blob([Buffer.alloc(130 * 1024, 217)], { type: 'application/octet-stream' }), 'fixture.bin');
    const result = await f.fetch(URL, { method: 'POST', body: form });
    assert.equal(await result.text(), 'ok');
    const uploaded = [...f.streams.values()][0];
    assert(uploaded.writes.every(bytes => bytes.length <= 65536));
    const body = Buffer.concat(uploaded.writes);
    const parsed = await new Response(body, { headers: uploaded.params.headers }).formData();
    assert.equal(parsed.get('title'), 'cafè');
    assert.equal(parsed.get('file').name, 'fixture.bin');
    assert.deepEqual(Buffer.from(await parsed.get('file').arrayBuffer()), Buffer.alloc(130 * 1024, 217));
    await (await f.fetch(URL, { method: 'POST', body: new URLSearchParams({ x: 'a b' }) })).text();
    const second = [...f.streams.values()][1];
    assert.equal(Buffer.concat(second.writes).toString(), 'x=a+b');
    assert(new Headers(second.params.headers).get('content-type').startsWith('application/x-www-form-urlencoded'));
  } finally { f.close(); }
});

test('redirects revalidate every hop and preserve 307 bytes; 303 removes the body', async () => {
  const f = fixture((params, hop) => hop === 1 ? { status: 307, headers: [['location', '/second']] }
    : hop === 2 ? { waitUpload: true, status: 303, headers: [['location', '/final']] } : { chunks: ['done'] });
  try {
    const result = await f.fetch(URL, { method: 'POST', body: 'original' });
    assert.equal(result.url, 'https://api.example.test/final');
    assert.equal(result.redirected, true);
    assert.equal(await result.text(), 'done');
    const hops = [...f.streams.values()];
    assert.deepEqual(hops.map(hop => hop.params.method), ['POST', 'POST', 'GET']);
    assert.equal(Buffer.concat(hops[1].writes).toString(), 'original');
    assert.equal(new Headers(hops[2].params.headers).has('content-type'), false);
    assert(hops.every(hop => hop.cancelled === 1));
  } finally { f.close(); }
});

test('redirect modes, loops and nonreplayable streams fail without retrying mutations', async () => {
  for (const kind of ['loop', 'error', 'private', 'stream']) {
    const f = fixture(() => ({ status: 307, headers: [['location', kind === 'private' ? 'http://127.0.0.1/private' : '/next']] }));
    try {
      const init = kind === 'stream' ? { method: 'POST', body: new ReadableStream({ start(c) { c.enqueue(new Uint8Array([1])); c.close(); } }), duplex: 'half' }
        : kind === 'error' ? { redirect: 'error' } : {};
      await assert.rejects(f.fetch(URL, init), TypeError);
      assert.equal(f.streams.size, kind === 'loop' ? 21 : 1);
      assert([...f.streams.values()].every(hop => hop.cancelled === 1));
    } finally { f.close(); }
  }
  const manual = fixture(() => ({ status: 302, headers: [['location', '/next']], chunks: ['manual'] }));
  try { const result = await manual.fetch(URL, { redirect: 'manual' }); assert.equal(result.status, 302); assert.equal(await result.text(), 'manual'); assert.equal(manual.streams.size, 1); }
  finally { manual.close(); }
});

test('abort after headers cancels the in-flight read and exact handle once', async () => {
  const reading = deferred(); const pending = deferred();
  const f = fixture(() => ({ read() { reading.resolve(); return pending.promise; } }));
  const controller = new AbortController();
  try {
    const result = await f.fetch(URL, { signal: controller.signal });
    const body = result.text();
    await reading.promise;
    controller.abort();
    await assert.rejects(body, { name: 'AbortError' });
    await flush();
    assert.equal([...f.streams.values()][0].cancelled, 1);
    assert.equal(f.transport.sent.filter(message => message.kind === 'cancel').length, 1);
    pending.resolve({ pending: false, done: false, chunkBase64: 'eA==' });
    await flush();
    assert.equal(f.transport.sent.filter(message => message.method === 'http.read').length, 1);
  } finally { f.close(); }
});

test('unsupported authority and pre-aborted calls perform no broker request', async () => {
  const f = fixture(() => { throw new Error('must not be called'); });
  try {
    for (const options of [{ credentials: 'include' }, { dispatcher: {} }, { connectionId: 'foreign' }, { integrity: 'sha256-invalid' }, { keepalive: true }]) {
      await assert.rejects(f.fetch(URL, options), TypeError);
    }
    await assert.rejects(f.fetch(URL, { signal: AbortSignal.abort() }), { name: 'AbortError' });
    assert.equal(f.transport.sent.length, 0);
  } finally { f.close(); }
});
