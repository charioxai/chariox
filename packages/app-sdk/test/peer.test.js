import test from 'node:test';
import assert from 'node:assert/strict';
import { AppPeer } from '../src/peer.js';
import { deferred, envelope, fakeTransport, flush, generation, request, response } from './helpers.js';

test('out-of-order replies resolve only their own request without invoking App handlers', async () => {
  const transport = fakeTransport();
  const peer = new AppPeer({ transport, generation, handleRequest: () => assert.fail('Responses cannot invoke tools') });
  const first = peer.request('state.get', { key: 'first' });
  const second = peer.request('state.get', { key: 'second' });
  transport.receive(response(transport.sent[1].id, 2));
  transport.receive(response(transport.sent[0].id, 1));
  assert.deepEqual(await Promise.all([first, second]), [1, 2]);
  assert.equal(Object.hasOwn(transport.sent[0], 'context'), false);
  peer.close();
});

test('pending limit rejects before sending and disconnect releases every request', async () => {
  const transport = fakeTransport();
  const peer = new AppPeer({ transport, generation, limits: { maxPending: 2 } });
  const first = peer.request('state.get', {});
  const second = peer.request('state.get', {});
  await assert.rejects(peer.request('state.get', {}), { code: 'BUSY' });
  assert.equal(transport.sent.length, 2);
  const results = Promise.allSettled([first, second]);
  transport.disconnect();
  assert.deepEqual((await results).map((result) => result.reason.code), ['DISCONNECTED', 'DISCONNECTED']);
  assert.equal(transport.closed, true);
});

test('abort sends cancellation, frees outgoing capacity and ignores a late success', async () => {
  const transport = fakeTransport();
  const peer = new AppPeer({ transport, generation, limits: { maxPending: 1 } });
  const abort = new AbortController();
  const pending = peer.request('http.request', {}, { signal: abort.signal });
  const rejected = assert.rejects(pending, { code: 'CANCELLED' });
  abort.abort();
  await rejected;
  assert.equal(transport.sent[1].kind, 'cancel');
  transport.receive(response(transport.sent[0].id, 'late'));
  const next = peer.request('state.get', {});
  transport.receive(response(transport.sent[2].id, 'fresh'));
  assert.equal(await next, 'fresh');
  peer.close();
});

test('generation mismatch invalidates pending operations and stops the channel', async () => {
  const transport = fakeTransport();
  const peer = new AppPeer({ transport, generation });
  const pending = peer.request('state.get', {});
  const rejected = assert.rejects(pending, { code: 'PROTOCOL_ERROR' });
  transport.receive({ ...response(transport.sent[0].id, 'bad'), generation: 'older' });
  await rejected;
  assert.equal(transport.closed, true);
});

test('a handler ignoring cancellation retains capacity; its late result cannot settle twice', async () => {
  const transport = fakeTransport();
  const running = deferred();
  let context;
  const peer = new AppPeer({ transport, generation, limits: { maxHandlers: 1 }, handleRequest: (_method, _params, ctx) => {
    context = ctx;
    return running.promise;
  } });
  transport.receive(request('host-1', 'tools.invoke', {}, { context: { agent_id: 'agent-1' } }));
  await flush();
  transport.receive(envelope({ kind: 'cancel', id: 'host-1' }));
  assert.equal(context.signal.aborted, true);
  assert.equal(context.agent_id, 'agent-1');
  transport.receive(request('host-2', 'tools.invoke'));
  assert.equal(transport.sent.at(-1).error.code, 'BUSY');
  running.resolve('late effect');
  await flush();
  assert.equal(transport.sent.filter((message) => message.id === 'host-1').length, 1);
  peer.close();
});

test('cancellation arriving before callback dispatch prevents starting effects', async () => {
  const transport = fakeTransport();
  const peer = new AppPeer({ transport, generation, handleRequest: () => assert.fail('Cancelled handler cannot start') });
  transport.receive(request('host-1', 'tools.invoke'));
  transport.receive(envelope({ kind: 'cancel', id: 'host-1' }));
  await flush();
  assert.equal(transport.sent.length, 1);
  assert.equal(transport.sent[0].error.code, 'CANCELLED');
  peer.close();
});

test('incoming and outgoing deadlines settle once and preserve unknown external outcomes', async () => {
  const transport = fakeTransport();
  const peer = new AppPeer({ transport, generation, handleRequest: () => new Promise(() => {}), limits: { maxDeadlineMs: 15 } });
  const pending = peer.request('http.request', {});
  transport.receive(request('host-1', 'tools.invoke'));
  await assert.rejects(pending, { code: 'DEADLINE_EXCEEDED' });
  await new Promise((resolve) => setTimeout(resolve, 10));
  assert.equal(transport.sent.some((message) => message.id === 'host-1' && message.error?.code === 'DEADLINE_EXCEEDED'), true);
  assert.equal(transport.sent.filter((message) => message.kind === 'request').length, 1, 'No automatic retry of unknown HTTP outcome');
  peer.close();
});

test('a repeated active inbound identity closes rather than invoking twice', async () => {
  const transport = fakeTransport();
  let called = 0;
  const peer = new AppPeer({ transport, generation, handleRequest: () => { called += 1; return new Promise(() => {}); } });
  transport.receive(request('host-1', 'tools.invoke'));
  await flush();
  transport.receive(request('host-1', 'tools.invoke'));
  assert.equal(called, 1);
  assert.equal(transport.closed, true);
  peer.close();
});
