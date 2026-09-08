// Run only through scripts/test-app-fetch-clients.mjs: dependencies are pinned
// in this directory but installed in private outside-repository scratch.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { gzipSync } from 'node:zlib';
import { fixture } from '../fetch-helpers.js';
const root = process.env.CHARIOX_FETCH_CLIENT_FIXTURES;
if (!root || !path.isAbsolute(root)) throw new Error('Expected explicit external fixture installation');
const load = createRequire(path.join(root, 'package.json'));
for (const [name, version] of [['@octokit/request', '10.0.16'], ['eventsource-parser', '4.1.0']]) {
  assert.equal(JSON.parse(readFileSync(path.join(root, 'node_modules', name, 'package.json'))).version, version);
}
const { request } = await import(pathToFileURL(load.resolve('@octokit/request')));
const { EventSourceParserStream } = await import(pathToFileURL(load.resolve('eventsource-parser/stream')));

test('pinned Octokit request.fetch handles JSON, query serialization, redirects and streamed results', async () => {
  const f = fixture((params, sequence) => {
    if (sequence === 1) {
      assert.equal(params.url, 'https://api.github.com/repos/chariox/fixture/issues?state=open&per_page=2');
      return { status: 302, headers: [['location', '/final']] };
    }
    if (sequence === 2) return { chunks: [gzipSync(Buffer.from('[{"id":42}]'))], headers: [['content-type', 'application/json'], ['content-encoding', 'gzip']] };
    return { chunks: [Buffer.from([0, 255]), 'tail'], headers: [['content-type', 'application/octet-stream']] };
  });
  try {
    const result = await request('GET /repos/{owner}/{repo}/issues', { owner: 'chariox', repo: 'fixture', state: 'open', per_page: 2, request: { fetch: f.fetch } });
    assert.deepEqual(result.data, [{ id: 42 }]);
    assert.equal(result.url, 'https://api.github.com/final');
    const stream = await request('GET /download', { request: { fetch: f.fetch, parseSuccessResponseBody: false } });
    assert(stream.data instanceof ReadableStream);
    assert.deepEqual(Buffer.from(await new Response(stream.data).arrayBuffer()), Buffer.from([0, 255, ...Buffer.from('tail')]));
  } finally { f.close(); }
});

test('pinned SSE parser receives fragmented UTF-8 records with its own explicit buffer bound', async () => {
  const contents = Buffer.from(': heartbeat\n\nid:42\nevent:issue\ndata: café\ndata: next\n\ndata: final\n\n');
  const f = fixture(() => ({ headers: [['content-type', 'text/event-stream']], chunks: [...contents].map(byte => Buffer.from([byte])) }));
  try {
    const response = await f.fetch('https://api.example.test/events');
    const events = response.body.pipeThrough(new TextDecoderStream()).pipeThrough(new EventSourceParserStream({ maxBufferSize: 8192, onError: 'terminate' }));
    const found = [];
    for await (const event of events) found.push(event);
    assert.equal(found.length, 2);
    assert.equal(found[0].id, '42');
    assert.equal(found[0].event, 'issue');
    assert.equal(found[0].data, 'café\nnext');
    assert.equal(found[1].data, 'final');
    assert.equal([...f.streams.values()][0].cancelled, 1);
  } finally { f.close(); }
});

test('pinned Octokit binary upload retains client length checks while the kernel owns framing', async () => {
  const f = fixture(params => {
    assert.equal(new Headers(params.headers).has('content-length'), false);
    return { waitUpload: true, status: 201, chunks: ['{"id":7}'], headers: [['content-type', 'application/json']] };
  });
  try {
    const body = Buffer.from([0, 255, 128, 1]);
    const result = await request('POST https://uploads.github.com/fixture', {
      data: body, headers: { 'content-type': 'application/octet-stream', 'content-length': body.length }, request: { fetch: f.fetch },
    });
    assert.equal(result.status, 201);
    assert.deepEqual(result.data, { id: 7 });
    assert.deepEqual(Buffer.concat([...f.streams.values()][0].writes), body);
  } finally { f.close(); }
});
