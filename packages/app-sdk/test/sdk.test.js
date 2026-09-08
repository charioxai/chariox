import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { AppError, createAppSdk } from '../src/index.js';
import { deferred, envelope, fakeTransport, flush, generation, request, response } from './helpers.js';

const roots = { package: '/isolated/package', data: '/isolated/data', temporary: '/isolated/tmp' };
function setup(declarations = {}) {
  const transport = fakeTransport();
  const sdk = createAppSdk({ transport, generation, paths: roots, declarations });
  return { transport, sdk };
}

test('only package-declared handlers register, and readiness cannot hide a missing implementation', async () => {
  const { transport, sdk } = setup({ tools: ['create_project'], events: ['issue_created'] });
  assert.throws(() => sdk.tools.register('undeclared', () => {}), { code: 'UNDECLARED_HANDLER' });
  await assert.rejects(sdk.ready(), { code: 'MISSING_HANDLER' });
  sdk.tools.register('create_project', () => {});
  assert.throws(() => sdk.tools.register('create_project', () => {}), { code: 'DUPLICATE_HANDLER' });
  sdk.events.register('issue_created', () => {});
  const ready = sdk.ready();
  assert.deepEqual(transport.sent[0].params, { tools: ['create_project'], events: ['issue_created'], lifecycle: [] });
  transport.receive(response(transport.sent[0].id, null));
  await ready;
  await assert.rejects(sdk.ready(), { code: 'ALREADY_READY' });
  assert.throws(() => sdk.lifecycle.on('resume', () => {}), { code: 'ALREADY_READY' });
  sdk.close();
});

test('App occurrences use acknowledged delivery and retain occurrence identity across replays', async () => {
  const { transport, sdk } = setup({ events: ['issue_created'] });
  const received = [];
  sdk.events.register('issue_created', (event) => { received.push(event); return { accepted: true }; });
  const params = { name: 'issue_created', occurrence_id: 'occ-1', payload: { issue: 'LIN-3' } };
  transport.receive(request('host-1', 'events.deliver', params));
  await flush();
  assert.equal(transport.sent[0].kind, 'response');
  transport.receive(request('host-2', 'events.deliver', params));
  await flush();
  assert.equal(received.length, 2, 'Delivery is at least once; durable deduplication belongs to kernel state');
  assert.deepEqual(received.map((event) => event.occurrenceId), ['occ-1', 'occ-1']);
  transport.receive(envelope({ kind: 'event', name: 'issue_created', data: params }));
  assert.equal(transport.closed, true, 'Fire-and-forget wire events cannot bypass receipt handling');
  assert.equal(received.length, 2);
  sdk.close();
});

test('incoming tool identity uses trusted context; parameters cannot override it', async () => {
  const { transport, sdk } = setup({ tools: ['create_project'] });
  sdk.tools.register('create_project', (input, context) => {
    assert.equal(context.actor.kind, 'agent');
    assert.equal(context.actor.id, 'agent-real');
    assert.equal(input.actor.id, 'human-forged');
    assert.equal(Object.isFrozen(context), true);
    return { created: true };
  });
  transport.receive(request('host-1', 'tools.invoke', {
    name: 'create_project', input: { actor: { kind: 'human', id: 'human-forged' } },
  }, { context: { actor: { kind: 'agent', id: 'agent-real' } } }));
  await flush();
  assert.deepEqual(transport.sent[0].result, { created: true });
  sdk.close();
});

test('lifecycle callbacks cannot overlap, including a callback ignoring cancellation', async () => {
  const { transport, sdk } = setup();
  const busy = deferred();
  sdk.lifecycle.on('suspend', () => busy.promise);
  sdk.lifecycle.on('resume', () => assert.fail('Resume cannot overlap unfinished suspension'));
  transport.receive(request('host-1', 'lifecycle.dispatch', { event: 'suspend' }));
  await flush();
  transport.receive(envelope({ kind: 'cancel', id: 'host-1' }));
  transport.receive(request('host-2', 'lifecycle.dispatch', { event: 'resume' }));
  await flush();
  assert.equal(transport.sent.find((item) => item.id === 'host-2').error.code, 'BUSY');
  busy.resolve(null);
  await flush();
  sdk.close();
});

test('state transaction forwards state and occurrence together without local persistence', async () => {
  const { transport, sdk } = setup();
  const transaction = {
    schemaVersion: 1,
    checks: [{ key: 'issue', version: 3 }],
    writes: [{ key: 'issue', value: { state: 'done' } }],
    occurrences: [{ automationId: 'auto-1', occurrenceId: 'issue-revision-4', eventVersion: 1, occurredAtMs: 1000, payload: {} }],
  };
  const pending = sdk.state.transaction(transaction);
  assert.deepEqual(transport.sent[0].params, transaction);
  assert.equal(transport.sent[0].method, 'state.transaction');
  transport.receive(response(transport.sent[0].id, { revision: 4, receipts: [] }));
  assert.deepEqual(await pending, { revision: 4, receipts: [] });
  sdk.close();
});

test('versioned occurrences preserve replay time and schedule revision and reject incomplete envelopes', async () => {
  const { transport, sdk } = setup();
  const event = { automationId: 'auto-1', occurrenceId: 'due-1', eventVersion: 2,
    occurredAtMs: 123456, scheduleRevision: 'schedule-7', payload: { id: 'todo-1' } };
  for (const patch of [{ eventVersion: 0 }, { eventVersion: 1.5 }, { eventVersion: 0x100000000 },
    { occurredAtMs: undefined }, { occurredAtMs: -1 }, { occurredAtMs: Number.MAX_SAFE_INTEGER + 1 },
    { scheduleRevision: 'bad revision' }, { owner: 'forged' }]) {
    const invalid = { ...event, ...patch };
    assert.throws(() => sdk.events.emit(invalid), { code: 'INVALID_ARGUMENT' });
    assert.throws(() => sdk.state.transaction({ schemaVersion: 1, checks: [], writes: [], occurrences: [invalid] }),
      { code: 'INVALID_ARGUMENT' });
  }
  assert.equal(transport.sent.length, 0);
  for (let attempt = 0; attempt < 2; attempt++) {
    const pending = sdk.events.emit(event);
    const sent = transport.sent.at(-1);
    assert.deepEqual(sent.params, event);
    transport.receive(response(sent.id, { receiptId: 'receipt-1', state: 'accepted' }));
    assert.deepEqual(await pending, { receiptId: 'receipt-1', state: 'accepted' });
  }
  sdk.close();
});

test('SDK emission matches the shared Rust event payload snapshot', async () => {
  const fixture = JSON.parse(readFileSync(new URL('./event-contract.json', import.meta.url), 'utf8'));
  const metadata = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(metadata.version, fixture.sdkVersion);
  assert.equal(fixture.minimumKernelProtocol, 290);
  const { transport, sdk } = setup();
  const pending = sdk.events.emit(fixture.occurrence);
  assert.deepEqual(transport.sent[0].params, fixture.occurrence);
  transport.receive(response(transport.sent[0].id, { receiptId: 'receipt-1', state: 'accepted' }));
  await pending;
  sdk.close();
});

test('file operations reject obvious escapes and forward only private paths and bounded bytes', async () => {
  const { transport, sdk } = setup();
  for (const path of ['/etc/passwd', '../other', 'data/../other', 'C:\\secret', 'a//b']) {
    assert.throws(() => sdk.files.atomicReplace(path, 'bad'), { code: 'INVALID_ARGUMENT' });
  }
  assert.throws(() => sdk.files.atomicReplace('ok', new Uint8Array(512 * 1024 + 1)), /512 KiB/);
  const write = sdk.files.atomicReplace('documents/issue.json', 'text');
  assert.deepEqual(transport.sent[0].params, { path: 'documents/issue.json', contentsBase64: 'dGV4dA==' });
  transport.receive(response(transport.sent[0].id, { bytesWritten: 4 }));
  await write;
  sdk.close();
});

test('HTTP and human validation are kernel calls; user approval does not hold a request open', async () => {
  const { transport, sdk } = setup();
  assert.throws(() => sdk.http.request({ url: 'file:///etc/passwd' }), { code: 'INVALID_ARGUMENT' });
  assert.throws(() => sdk.http.request({ url: 'https://user:pass@example.com/' }), { code: 'INVALID_ARGUMENT' });
  const http = sdk.http.request({ url: 'https://api.example.test/issues', connectionId: 'connection-1', body: 'hello' });
  assert.equal(transport.sent[0].method, 'http.request');
  assert.equal(transport.sent[0].params.bodyBase64, 'aGVsbG8=');
  transport.receive(response(transport.sent[0].id, { status: 200, headers: [], bodyBase64: '', url: 'https://api.example.test/issues' }));
  await http;
  const validation = sdk.validation.request({ action: 'pay', parameters: { amount: 10 } });
  transport.receive(response(transport.sent[1].id, { operationId: 'operation-1', state: 'pending' }));
  assert.deepEqual(await validation, { operationId: 'operation-1', state: 'pending' });
  assert.equal(transport.sent.length, 2);
  sdk.close();
});

test('no transcript, raw kernel request, credentials or approval-resolution API is exposed', () => {
  const { sdk } = setup();
  for (const key of ['conversation', 'transcript', 'kernel', 'credentials', 'request']) assert.equal(key in sdk, false);
  assert.equal('approve' in sdk.validation, false);
  assert.equal(Object.isFrozen(sdk), true);
  assert.equal(Object.isFrozen(sdk.paths), true);
  sdk.close();
});

test('unclassified handler exceptions never send private exception contents across IPC', async () => {
  const { transport, sdk } = setup({ tools: ['fail'] });
  sdk.tools.register('fail', () => { throw new Error('secret-token in /home/user/private'); });
  transport.receive(request('host-1', 'tools.invoke', { name: 'fail', input: null }));
  await flush();
  assert.deepEqual(transport.sent[0].error, { code: 'HANDLER_FAILED', message: 'App handler failed', retryable: false });
  sdk.close();
  assert.equal(new AppError('DENIED', 'Request denied').retryable, false);
});
