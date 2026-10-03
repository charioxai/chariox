import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { AppError, createAppSdk, occurrenceId } from '../src/index.js';
import { deferred, envelope, fakeTransport, flush, generation, request, response } from './helpers.js';

const roots = { package: '/isolated/package', data: '/isolated/data', temporary: '/isolated/tmp' };
function setup(declarations = {}) {
  const transport = fakeTransport();
  const sdk = createAppSdk({ transport, generation, paths: roots, declarations });
  return { transport, sdk };
}

test('only package-declared handlers register, and readiness cannot hide a missing implementation', async () => {
  const { transport, sdk } = setup({ tools: ['create_project'], incomingEvents: ['issue_created'] });
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
  const { transport, sdk } = setup({ incomingEvents: ['issue_created'] });
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

test('local health checks run before readiness ACK, default to null and preserve handler failure', async () => {
  for (const mode of ['absent', 'healthy', 'failed']) {
    const { transport, sdk } = setup();
    let called = false;
    if (mode !== 'absent') sdk.lifecycle.on('health_check', (_data, context) => {
      called = true;
      assert.equal(context.signal.aborted, false);
      if (mode === 'failed') throw new Error('private health diagnostic');
      return null;
    });
    const ready = sdk.ready();
    const report = transport.sent[0];
    assert.deepEqual(report.params.lifecycle, mode === 'absent' ? [] : ['health_check']);
    let acknowledged = false;
    const readiness = ready.then(() => { acknowledged = true; });
    transport.receive(request('health', 'lifecycle.dispatch', { event: 'health_check', data: null }));
    await flush();
    const result = transport.sent.find(item => item.id === 'health');
    assert.equal(acknowledged, false, 'A health callback cannot acknowledge activation');
    assert.equal(called, mode !== 'absent');
    if (mode === 'failed') {
      assert.deepEqual(result.error, { code: 'HANDLER_FAILED', message: 'App handler failed', retryable: false });
    } else assert.equal(result.result, null);
    assert.equal(transport.sent.length, 2, 'A local health callback performs no implicit SDK operation');
    transport.receive(response(report.id, null));
    await readiness;
    sdk.close();
  }
});

test('state transaction forwards state and occurrence together without local persistence', async () => {
  const { transport, sdk } = setup();
  const transaction = {
    schemaVersion: 1,
    checks: [{ key: 'issue', version: 3 }],
    writes: [{ key: 'issue', value: { state: 'done' } }],
    occurrences: [{ automationId: 'auto-1', occurrenceId: occurrenceId('issue-revision-4', 1000), eventVersion: 1, occurredAtMs: 1000, payload: {}, invocation: { prompt: 'Handle the issue', artifacts: [] } }],
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
  const event = { automationId: 'auto-1', occurrenceId: occurrenceId('due-1', 123456), eventVersion: 2,
    occurredAtMs: 123456, scheduleRevision: 'schedule-7', payload: { id: 'todo-1' }, invocation: { prompt: 'Handle the todo', artifacts: [] } };
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

test('automations are a parameterless read of the kernel projection', async () => {
  const { transport, sdk } = setup();
  const pending = sdk.events.automations();
  const sent = transport.sent[0];
  assert.equal(sent.method, 'events.automations');
  assert.deepEqual(sent.params, {});
  const listed = { automations: [{ automationId: 'reminders', event: 'todo_due', eventVersion: 1, state: 'paused', lastReceipt: null }] };
  transport.receive(response(sent.id, listed));
  assert.deepEqual(await pending, listed);
  sdk.close();
});

test('SDK emission matches the shared Rust event payload snapshot', async () => {
  const fixture = JSON.parse(readFileSync(new URL('./event-contract.json', import.meta.url), 'utf8'));
  const metadata = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(metadata.version, fixture.sdkVersion);
  assert.equal(fixture.minimumKernelProtocol, 294);
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
  const http = sdk.http.open({ url: 'https://api.example.test/issues', connectionId: 'connection-1', method: 'POST', hasBody: true });
  assert.equal(transport.sent[0].method, 'http.open');
  assert.equal(transport.sent[0].params.hasBody, true);
  transport.receive(response(transport.sent[0].id, { streamId: '07f5a249-a00b-4196-acdf-938265f88e0a' }));
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

test('a handler result that is not JSON fails only that call, names the value and is logged', async () => {
  const { transport, sdk } = setup({ tools: ['answer'] });
  let value;
  sdk.tools.register('answer', () => value);
  const ready = sdk.ready();
  transport.receive(response(transport.sent[0].id, null));
  await ready;
  const cycle = { list: [] };
  cycle.list.push(cycle);
  let nested = 'deep';
  for (let index = 0; index < 70; index += 1) nested = [nested];
  const cases = [
    [{ ok: true, echoed: undefined }, 'result.echoed is undefined'],
    [[1, undefined], 'result[1] is undefined'],
    [{ run() {} }, 'result.run is a function'],
    [[Symbol('s')], 'result[0] is a symbol'],
    [{ count: 10n }, 'result.count is a BigInt'],
    [{ ratio: NaN }, 'result.ratio is NaN, not a finite number'],
    [{ ratio: -Infinity }, 'result.ratio is -Infinity, not a finite number'],
    [{ id: 2 ** 53 }, 'result.id is an integer beyond the safe range; send it as a string'],
    [{ at: new Date(0) }, 'result.at is not a plain object or array'],
    [{ items: new Map() }, 'result.items is not a plain object or array'],
    [cycle, 'result.list[0] refers back to an object that contains it (a cycle)'],
    [{ 'odd key': '\ud800' }, 'result["odd key"] is a string that is not well-formed Unicode'],
    [{ ['\u0001'.repeat(64)]: undefined },
      `result${`[${JSON.stringify('\u0001'.repeat(64))}]`.slice(0, 249)}… is undefined`],
    [nested, `result${'[0]'.repeat(63)} exceeds the nesting limit of 64`],
    ['x'.repeat(1024 * 1024), 'the message exceeds its 1 MiB size limit'],
  ];
  for (const [index, [result, detail]] of cases.entries()) {
    value = result;
    const before = transport.sent.length;
    transport.receive(request(`call-${index}`, 'tools.invoke', { name: 'answer', input: null }));
    await flush();
    const [log, reply, extra] = transport.sent.slice(before);
    assert.equal(extra, undefined);
    assert.deepEqual(reply, envelope({ kind: 'response', id: `call-${index}`,
      error: { code: 'INVALID_OUTPUT', message: `The App's result cannot be sent: ${detail}`, retryable: false } }));
    assert.equal(log.method, 'log.write');
    assert.deepEqual(log.params, { level: 'error', fields: { code: 'INVALID_OUTPUT', method: 'tools.invoke', name: 'answer' },
      message: `Tool answer returned a result that cannot be sent, so that call failed with INVALID_OUTPUT: ${detail}` });
    transport.receive(response(log.id, null));
    assert.equal(transport.closed, false, detail);
  }
  value = { fine: true, list: [null, 1.5, 'Á'] };
  transport.receive(request('call-ok', 'tools.invoke', { name: 'answer', input: null }));
  await flush();
  assert.deepEqual(transport.sent.at(-1).result, value, 'The same worker keeps serving');
  sdk.close();
});

test('before readiness an invalid result still fails alone, and writes no log the kernel would refuse', async () => {
  const { transport, sdk } = setup();
  sdk.lifecycle.on('health_check', () => ({ healthy: undefined }));
  const ready = sdk.ready();
  transport.receive(request('health', 'lifecycle.dispatch', { event: 'health_check', data: null }));
  await flush();
  assert.equal(transport.sent.length, 2);
  assert.deepEqual(transport.sent[1].error, { code: 'INVALID_OUTPUT', retryable: false,
    message: "The App's result cannot be sent: result.healthy is undefined" });
  transport.receive(response(transport.sent[0].id, null));
  await ready;
  assert.equal(transport.closed, false);
  sdk.close();
});

test('an AppError the App made malformed fails its call as HANDLER_FAILED and keeps the channel', async () => {
  const { transport, sdk } = setup({ tools: ['fail'] });
  let thrown;
  sdk.tools.register('fail', () => { throw thrown; });
  const withCode = (code) => Object.assign(new AppError('DENIED', 'Request denied'), { code });
  const unreadable = new Proxy({}, { getPrototypeOf() { throw new Error('secret trap'); } });
  const cases = [withCode('has space'), withCode(''), withCode(7), withCode('x'.repeat(129)),
    Object.assign(new AppError('DENIED', 'x'), { message: 42 }), unreadable];
  for (const [index, error] of cases.entries()) {
    thrown = error;
    transport.receive(request(`call-${index}`, 'tools.invoke', { name: 'fail', input: null }));
    await flush();
    assert.deepEqual(transport.sent.at(-1), envelope({ kind: 'response', id: `call-${index}`,
      error: { code: 'HANDLER_FAILED', message: 'App handler failed', retryable: false } }));
  }
  thrown = Object.assign(new AppError('CONFLICT', 'stale \ud800 edit'), { retryable: 'yes' });
  transport.receive(request('call-kept', 'tools.invoke', { name: 'fail', input: null }));
  await flush();
  assert.deepEqual(transport.sent.at(-1).error, { code: 'CONFLICT', message: 'stale � edit', retryable: false });
  assert.equal(transport.sent.length, cases.length + 1, 'Before readiness nothing is logged');
  assert.equal(transport.closed, false);
  // Once ready, the App's log says why its own error became HANDLER_FAILED.
  const ready = sdk.ready();
  transport.receive(response(transport.sent.at(-1).id, null));
  await ready;
  thrown = withCode('has space');
  const before = transport.sent.length;
  transport.receive(request('call-logged', 'tools.invoke', { name: 'fail', input: null }));
  await flush();
  const [log, reply] = transport.sent.slice(before);
  assert.deepEqual(log.params, { level: 'error', fields: { code: 'HANDLER_FAILED', method: 'tools.invoke', name: 'fail' },
    message: 'Tool fail threw an AppError that cannot be sent, so that call failed with HANDLER_FAILED: '
      + 'its code is not 1 to 128 bytes without spaces or control characters' });
  assert.deepEqual(reply.error, { code: 'HANDLER_FAILED', message: 'App handler failed', retryable: false });
  thrown = withCode('\ud800');
  transport.receive(request('call-surrogate', 'tools.invoke', { name: 'fail', input: null }));
  await flush();
  assert.equal(transport.sent.at(-2).params.message, 'Tool fail threw an AppError that cannot be sent, so that call '
    + 'failed with HANDLER_FAILED: error.code is a string that is not well-formed Unicode');
  sdk.close();
});

test('SDK call parameters that are not JSON reject that call before sending anything', async () => {
  const { transport, sdk } = setup();
  await assert.rejects(sdk.log.write('info', 'signed in', { user: undefined }), {
    code: 'INVALID_ARGUMENT', message: 'App request parameters cannot be sent: params.fields.user is undefined',
  });
  await assert.rejects(sdk.state.transaction({ writes: [{ key: 'count', value: 1n }] }), {
    code: 'INVALID_ARGUMENT', message: 'App request parameters cannot be sent: params.writes[0].value is a BigInt',
  });
  await assert.rejects(sdk.connections.action({ connectionId: 'c', action: 'post', input: { at: new Date(0) } }), {
    code: 'INVALID_ARGUMENT', message: /params\.input\.at is not a plain object or array$/,
  });
  assert.equal(transport.sent.length, 0);
  assert.equal(transport.closed, false);
  const pending = sdk.state.get('count');
  transport.receive(response(transport.sent[0].id, { value: 1, version: 1 }));
  assert.deepEqual(await pending, { value: 1, version: 1 });
  sdk.close();
});

test('kernel-owned wakes are kernel calls, commit with state and deliver to one handler', async () => {
  const { transport, sdk } = setup();
  const received = [];
  sdk.schedule.onWake((wake) => { received.push(wake); return null; });
  assert.throws(() => sdk.schedule.onWake(() => null), (error) => error.code === 'DUPLICATE_HANDLER');
  assert.throws(() => sdk.schedule.set({ id: 'bad id', dueAtMs: 1 }), (error) => error.code === 'INVALID_ARGUMENT');
  assert.throws(() => sdk.schedule.set({ id: 'due', dueAtMs: -1 }), (error) => error.code === 'INVALID_ARGUMENT');
  const pending = sdk.schedule.set({ id: 'todo-7', dueAtMs: 5000, revision: 'r2' });
  assert.equal(transport.sent[0].method, 'schedule.set');
  assert.deepEqual(transport.sent[0].params, { id: 'todo-7', dueAtMs: 5000, revision: 'r2' });
  transport.receive(response(transport.sent[0].id, { wakes: [{ id: 'todo-7', dueAtMs: 5000, revision: 'r2' }] }));
  assert.equal((await pending).wakes.length, 1);
  const committed = sdk.state.transaction({ schemaVersion: 1, checks: [], writes: [],
    wakes: [{ op: 'set', id: 'todo-8', dueAtMs: 6000 }, { op: 'cancel', id: 'todo-7' }] });
  assert.deepEqual(transport.sent[1].params.wakes,
    [{ op: 'set', id: 'todo-8', dueAtMs: 6000, revision: '' }, { op: 'cancel', id: 'todo-7' }]);
  transport.receive(response(transport.sent[1].id, { revision: 1, receipts: [] }));
  await committed;
  transport.receive(request('host-9', 'schedule.wake', { id: 'todo-8', dueAtMs: 6000, revision: '', overdue: true }));
  await flush();
  assert.deepEqual(received, [{ id: 'todo-8', dueAtMs: 6000, revision: '', overdue: true }]);
  assert.equal(transport.sent.at(-1).kind, 'response');
  sdk.close();
});

test('a due wake without a registered handler is a typed failure', async () => {
  const { transport, sdk } = setup();
  transport.receive(request('host-1', 'schedule.wake', { id: 'todo-1', dueAtMs: 1, revision: '' }));
  await flush();
  assert.equal(transport.sent[0].error.code, 'METHOD_NOT_FOUND');
  sdk.close();
});

test('Apps receive AppError and the occurrence identity helper', async () => {
  const { sdk } = setup();
  assert.equal(sdk.AppError, AppError);
  assert.equal(sdk.events.occurrenceId('todo:1:2', 1000), occurrenceId('todo:1:2', 1000));
  sdk.close();
});

test('log messages are limited in UTF-8 bytes, like the kernel', () => {
  const { sdk } = setup();
  assert.throws(() => sdk.log.write('info', '\u{1F600}'.repeat(1100)), { code: 'LIMIT_EXCEEDED' });
  assert.throws(() => sdk.log.write('trace', 'x'), { code: 'INVALID_ARGUMENT' });
  sdk.close();
});

test('a migration step writes only its target schema version and reports completion', async () => {
  const { transport, sdk } = setup();
  assert.throws(() => sdk.migration({ from: 1, to: 3 }), TypeError);
  const context = sdk.migration({ from: 1, to: 2 });
  assert.equal(context.from, 1);
  assert.equal(context.to, 2);
  assert.throws(() => context.state.transaction({ schemaVersion: 1, checks: [], writes: [] }), { code: 'INVALID_ARGUMENT' });
  assert.equal(transport.sent.length, 0);
  const read = context.state.get('project');
  assert.deepEqual([transport.sent[0].method, transport.sent[0].params], ['state.get', { key: 'project' }]);
  transport.receive(response(transport.sent[0].id, null));
  assert.equal(await read, null);
  const writes = [{ key: 'project', value: { v: 2 } }];
  const committed = context.state.transaction({ checks: [{ key: 'project', version: null }], writes });
  assert.equal(transport.sent[1].method, 'state.transaction');
  assert.deepEqual(transport.sent[1].params, { checks: [{ key: 'project', version: null }], writes, schemaVersion: 2 });
  transport.receive(response(transport.sent[1].id, { revision: 1, receipts: [] }));
  await committed;
  const step = sdk.migrationStep(2);
  assert.deepEqual([transport.sent[2].method, transport.sent[2].params], ['migration.step', { to: 2 }]);
  transport.receive(response(transport.sent[2].id, null));
  assert.equal(await step, null);
  sdk.close();
});

test('pickFile waits on a pending kernel reference and resolves with grants, or rejects a decline', async () => {
  const { transport, sdk } = setup();
  const picked = sdk.host.pickFile({ accept: ['.md'] });
  await flush();
  assert.equal(transport.sent[0].method, 'host.pick_file');
  assert.deepEqual(transport.sent[0].params, { accept: ['.md'] });
  transport.receive(response(transport.sent[0].id, { operationId: 'file-pick-1', state: 'pending', grantIds: [], expiresAtMs: 1 }));
  await new Promise(resolve => setTimeout(resolve, 300));
  assert.equal(transport.sent[1].method, 'host.pick_file_status');
  assert.deepEqual(transport.sent[1].params, { operationId: 'file-pick-1' });
  // A retryable refusal (a busy kernel) keeps polling instead of failing.
  transport.receive(envelope({ kind: 'response', id: transport.sent[1].id, error: { code: 'APP_BUSY', message: 'busy', retryable: true } }));
  await new Promise(resolve => setTimeout(resolve, 600));
  assert.equal(transport.sent[2].method, 'host.pick_file_status');
  transport.receive(response(transport.sent[2].id, { operationId: 'file-pick-1', state: 'granted', grantIds: ['grant-1'], expiresAtMs: 2 }));
  assert.deepEqual(await picked, { grantIds: ['grant-1'] });

  const declined = sdk.host.pickFile({});
  await flush();
  transport.receive(response(transport.sent[3].id, { operationId: 'file-pick-2', state: 'declined', grantIds: [], expiresAtMs: 1 }));
  await assert.rejects(declined, { code: 'DECLINED' });
  sdk.close();
});

test('connections list and act through the kernel with the exact wire fields', async () => {
  const { transport, sdk } = setup();
  const listed = sdk.connections.list();
  await flush();
  assert.equal(transport.sent[0].method, 'connections.list');
  assert.deepEqual(transport.sent[0].params, {});
  transport.receive(response(transport.sent[0].id, { connections: [{ generatorId: 'dev.chariox.slack', connectionId: 'c1', actions: ['notification.reply'] }] }));
  assert.equal((await listed).connections[0].connectionId, 'c1');
  const acted = sdk.connections.action({ connectionId: 'c1', action: 'notification.reply', input: { text: 'On it' }, context: { channel_id: 'C1' } });
  await flush();
  assert.equal(transport.sent[1].method, 'connections.action');
  assert.deepEqual(transport.sent[1].params, { connectionId: 'c1', action: 'notification.reply', input: { text: 'On it' }, context: { channel_id: 'C1' } });
  transport.receive(response(transport.sent[1].id, { accepted: true, result: { ts: '1.3' }, idempotencyKey: 'app:x:y' }));
  assert.equal((await acted).accepted, true);
  sdk.close();
});

test('copy and link helpers return pending human offers on the existing SDK channel', async () => {
  const { transport, sdk } = setup();
  for (const [call, method, params] of [
    [() => sdk.host.writeClipboard('copy\ntext'), 'host.clipboard_write', { text: 'copy\ntext' }],
    [() => sdk.host.openLink('https://example.org/a?x=%20'), 'host.open_link', { url: 'https://example.org/a?x=%20' }],
  ]) {
    const result = call();
    const sent = transport.sent.at(-1);
    assert.equal(sent.method, method);
    assert.deepEqual(sent.params, params);
    const pending = { operationId: 'host-offer', state: 'pending', expiresAtMs: 2000000000000 };
    transport.receive(response(sent.id, pending));
    assert.deepEqual(await result, pending);
  }
  assert.equal('readClipboard' in sdk.host, false);
  sdk.close();
});
