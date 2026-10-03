import test from 'node:test';
import assert from 'node:assert/strict';
import { AppPeer } from '../src/peer.js';
import { encodeFrame, FrameDecoder } from '../src/protocol.js';
import { streamTransport } from '../src/transport.js';
import { deferred, envelope, fakeTransport, flush, generation, request, response, TestStream } from './helpers.js';

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
  await assert.rejects(peer.request('state.get', {}), { code: 'APP_BUSY', retryable: true,
    cause: 'app_pending_capacity_full', retryAfterMs: 500,
    message: 'app_pending_capacity_full; retry after 500 ms' });
  assert.equal(transport.sent.length, 2);
  const results = Promise.allSettled([first, second]);
  transport.disconnect();
  assert.deepEqual((await results).map((result) => result.reason.code), ['DISCONNECTED', 'DISCONNECTED']);
  assert.equal(transport.closed, true);
});

test('an 80-call framed backlog completes or returns retryable APP_BUSY with capacity metadata',
  { timeout: 5000 }, async (t) => {
    const stream = new TestStream();
    const transport = streamTransport(stream);
    const running = deferred();
    let handled = 0;
    const worker = new AppPeer({ transport, generation, handleRequest: async () => {
      handled += 1;
      await running.promise;
      return 'complete';
    } });
    const callerTransport = fakeTransport();
    const caller = new AppPeer({ transport: callerTransport, generation });
    t.after(() => { running.resolve(); caller.close(); worker.close(); });
    // The kernel's 64 pending slots can arrive together after a blocked worker
    // resumes. Hold all replies while submitting the full 80-call burst.
    const settled = Promise.allSettled(Array.from({ length: 80 }, () => caller.request('tools.invoke', {})));
    assert.equal(callerTransport.sent.length, 64);
    for (const message of callerTransport.sent) stream.push(encodeFrame(message));
    await flush();
    assert.equal(handled, 16, 'the SDK execution bound stays below the pending-call bound');
    let read = 0;
    const decoder = new FrameDecoder((message) => {
      if (message.error) {
        assert.deepEqual(Object.keys(message.error).sort(), ['code', 'message', 'retryable'], 'no wire fields added');
      }
      callerTransport.receive(message);
    });
    const drain = () => { while (read < stream.frames.length) decoder.push(stream.frames[read++]); };
    drain();
    running.resolve();
    await flush();
    drain();
    const results = await settled;
    const completed = results.filter((result) => result.status === 'fulfilled');
    const errors = results.filter((result) => result.status === 'rejected').map((result) => result.reason);
    assert.equal(completed.length, 16);
    assert.equal(errors.length, 64);
    for (const error of errors) {
      assert.equal(error.code, 'APP_BUSY');
      assert.equal(error.retryable, true);
      assert.equal(error.retryAfterMs, 500);
      assert.match(error.cause, /^app_(pending|handler)_capacity_full$/);
      assert.equal(error.message, `${error.cause}; retry after 500 ms`);
    }
    assert.equal(errors.filter((error) => error.cause === 'app_handler_capacity_full').length, 48);
    assert.equal(errors.filter((error) => error.cause === 'app_pending_capacity_full').length, 16);
    assert.equal(stream.destroyed, false);
    // Refusals ran no handler; both capacities recover, with no automatic retry.
    assert.equal(callerTransport.sent.length, 64);
    const next = caller.request('tools.invoke', {});
    stream.push(encodeFrame(callerTransport.sent.at(-1)));
    await flush();
    drain();
    assert.equal(await next, 'complete');
    t.diagnostic(JSON.stringify({ calls: results.length, completed: completed.length, APP_BUSY: errors.length,
      BUSY: 0, timeouts: 0, maxHandlers: 16, maxPending: 64, recovered: true }));
  });

test('arbitrary remote APP_BUSY errors retain their message without invented capacity metadata', async () => {
  const transport = fakeTransport();
  const peer = new AppPeer({ transport, generation });
  const pending = peer.request('state.get', {});
  transport.receive(envelope({ kind: 'response', id: transport.sent[0].id,
    error: { code: 'APP_BUSY', message: 'custom App refusal', retryable: true } }));
  await assert.rejects(pending, (error) => {
    assert.equal(error.message, 'custom App refusal');
    assert.equal(error.retryable, true);
    assert.equal(error.cause, undefined);
    assert.equal(error.retryAfterMs, undefined);
    return true;
  });
  peer.close();
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
  assert.deepEqual(transport.sent.at(-1).error, { code: 'APP_BUSY', retryable: true,
    message: 'app_handler_capacity_full; retry after 500 ms' });
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

test('on a real stream a result refused while encoding writes nothing and fails only its call', async () => {
  const stream = new TestStream();
  const transport = streamTransport(stream, { maxFrameBytes: 4096, maxQueuedBytes: 8192 });
  let reads = 0;
  // Checked once as a string, then serialized as undefined: encodeFrame refuses it.
  const flipping = { get value() { reads += 1; return reads > 1 ? undefined : 'first read'; } };
  // Checked twice as a string, then serialized 70 levels deep.
  let late = 0;
  const deep = () => { let value = 'deep'; for (let index = 0; index < 70; index += 1) value = [value]; return value; };
  const changing = { get value() { late += 1; return late > 2 ? deep() : 'early read'; } };
  const reported = [];
  const peer = new AppPeer({
    transport, generation,
    handleRequest: (_method, params) => ({ flip: flipping, late: changing, large: 'x'.repeat(5000) })[params.mode] ?? params,
    onInvalidOutput: (method, params, detail) => { reported.push([method, params.mode, detail]); throw new Error('ignored'); },
  });
  for (const mode of ['flip', 'late', 'large', 'fine']) stream.push(encodeFrame(request(mode, 'tools.invoke', { mode })));
  await flush();
  await flush();
  const replies = [];
  const decoder = new FrameDecoder((message) => replies.push(message));
  for (const frame of stream.frames) decoder.push(frame);
  assert.deepEqual(replies.map((reply) => [reply.id, reply.error?.code ?? reply.result]),
    [['flip', 'INVALID_OUTPUT'], ['late', 'INVALID_OUTPUT'], ['large', 'INVALID_OUTPUT'], ['fine', { mode: 'fine' }]]);
  assert.equal(replies[0].error.message, "The App's result cannot be sent: result.value is undefined");
  assert.equal(replies[1].error.message, "The App's result cannot be sent: the message changed while it was serialized");
  assert.equal(replies[2].error.message, "The App's result cannot be sent: the message exceeds its frame size limit");
  assert.deepEqual(reported, [['tools.invoke', 'flip', 'result.value is undefined'],
    ['tools.invoke', 'late', 'the message changed while it was serialized'],
    ['tools.invoke', 'large', 'the message exceeds its frame size limit']]);
  assert.equal(stream.destroyed, false);
  peer.close();
});

test('a transport failure while answering still closes the channel', async () => {
  const transport = fakeTransport();
  transport.send = () => { transport.disconnect(new Error('write failed')); throw new Error('write failed'); };
  const peer = new AppPeer({ transport, generation, handleRequest: () => ({ fine: true }) });
  transport.receive(request('host-1', 'tools.invoke'));
  await flush();
  assert.equal(transport.closed, true);
  await assert.rejects(peer.request('state.get', {}), { code: 'DISCONNECTED' });
});

test('a transport that throws without a refusal detail closes the channel, even if it stays open', async () => {
  const transport = fakeTransport();
  transport.send = () => { throw new Error('write failed'); };
  const peer = new AppPeer({ transport, generation, handleRequest: () => ({ fine: true }) });
  transport.receive(request('host-1', 'tools.invoke'));
  await flush();
  assert.equal(transport.closed, true);
  await assert.rejects(peer.request('state.get', {}), { code: 'DISCONNECTED' });
});
