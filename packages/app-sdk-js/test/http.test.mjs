import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createFetch } from '../src/http.js';
import { ResourceScope } from '../src/core.js';

function response() {
  let closed = 0;
  const stream = (async function* () {
    try { yield new TextEncoder().encode('Todo 🍋'); yield new TextEncoder().encode('!'); }
    finally { closed++; }
  })();
  return { stream, closed: () => closed };
}

test('fetch forwards only the broker envelope and streams UTF-8', async () => {
  const source = response();
  const scope = new ResourceScope();
  const fetch = createFetch({ async fetch(req, body) {
    assert.deepEqual(req, { url: 'https://example.com/', method: 'GET', headers: [['accept', 'text/plain']], connectionId: null, operationId: null });
    assert.equal(body, null);
    return [{ status: 200, headers: [['x-test', 'yes']], url: req.url }, source.stream];
  } }, scope);
  const result = await fetch('https://example.com/', { headers: { Accept: 'text/plain' } });
  assert.equal(await result.text(), 'Todo 🍋!');
  assert.equal(result.headers.get('X-Test'), 'yes');
  assert.equal(source.closed(), 1);
  await scope.close();
  assert.equal(source.closed(), 1);
  await assert.rejects(result.text(), /already been consumed/);
});

test('breaking a response loop or exceeding a buffer limit drops the body', async () => {
  for (const limit of [null, 1]) {
    const source = response();
    const scope = new ResourceScope();
    const fetch = createFetch({ fetch: async () => [{ status: 200, headers: [], url: 'https://example.com/' }, source.stream] }, scope);
    const result = await fetch('https://example.com/');
    if (limit === null) { for await (const _ of result.body) break; }
    else await assert.rejects(result.bytes({ limit }), e => e.code === 'APP_HTTP_LIMIT');
    assert.equal(source.closed(), 1);
    await scope.close();
  }
});

test('pending raw import gets QuickJS cancel and late response cleanup', async () => {
  let finish, cancelled = 0, dropped = 0;
  let started;
  const opened = new Promise(resolve => { started = resolve; });
  const pending = new Promise(resolve => { finish = resolve; });
  pending.cancel = () => { cancelled++; };
  const scope = new ResourceScope();
  const fetch = createFetch({ fetch: () => { started(); return pending; } }, scope);
  const result = fetch('https://example.com/');
  const rejection = assert.rejects(result, e => e.code === 'CANCELLED');
  await opened;
  await scope.close();
  assert.equal(cancelled, 1);
  finish([{ status: 200, headers: [], url: 'https://example.com/' }, { async return() { dropped++; } }]);
  await rejection;
  assert.equal(dropped, 1);
});

test('aborting one fetch leaves its sibling and invocation alive', async () => {
  const scope = new ResourceScope();
  const dropped = [];
  const fetch = createFetch({ fetch: async request => [{ status: 200, headers: [], url: request.url }, { async return() { dropped.push(request.url); } }] }, scope);
  const controller = new AbortController();
  await fetch('https://example.com/a', { signal: controller.signal });
  await fetch('https://example.com/b');
  controller.abort('cancel request A');
  await Promise.resolve(); await Promise.resolve();
  assert.equal(scope.signal.aborted, false);
  assert.deepEqual(dropped, ['https://example.com/a']);
  await fetch('https://example.com/c');
  await scope.close();
  assert.deepEqual(dropped.sort(), ['https://example.com/a', 'https://example.com/b', 'https://example.com/c']);
});
