import test from 'node:test';
import assert from 'node:assert/strict';
import { createAppSdk } from '../src/index.js';
import { fakeTransport, generation, response } from './helpers.js';

const paths = { package: '/isolated/package', data: '/isolated/data', temporary: '/isolated/tmp' };
const bytes = value => Buffer.byteLength(JSON.stringify(value));
const event = (id = 'event') => ({ automationId: 'automation', occurrenceId: id, eventVersion: 1,
  occurredAtMs: 1000, payload: { text: '' }, invocation: { prompt: 'Handle this event', artifacts: [] } });
function setup() {
  const transport = fakeTransport();
  return { transport, sdk: createAppSdk({ transport, generation, paths, declarations: { tools: [], incomingEvents: [] } }) };
}
async function acknowledge(transport, pending) {
  const sent = transport.sent.at(-1);
  transport.receive(response(sent.id, null));
  await pending;
}

test('outgoing-only Apps become ready without an incoming handler and legacy declarations fail closed', async () => {
  const { sdk, transport } = setup();
  assert.throws(() => sdk.events.register('outgoing', () => {}), { code: 'UNDECLARED_HANDLER' });
  await acknowledge(transport, sdk.ready());
  assert.deepEqual(transport.sent[0].params, { tools: [], events: [], lifecycle: [] });
  await acknowledge(transport, sdk.events.emit(event()));
  sdk.close();
  assert.throws(() => createAppSdk({ transport: fakeTransport(), generation, paths,
    declarations: { tools: [], events: ['ambiguous'] } }), TypeError);
});

test('invocations and artifact metadata require complete strict fields without ambient access', async () => {
  const { sdk, transport } = setup();
  for (const invocation of [undefined, null, {}, { prompt: 'work' },
    { prompt: '', artifacts: [] }, { prompt: '\u0085', artifacts: [] },
    { prompt: 'work', artifacts: [], owner: 'forged' }, { prompt: 'work', artifacts: null }]) {
    assert.throws(() => sdk.events.emit({ ...event(), invocation }), { code: 'INVALID_ARGUMENT' });
  }
  const artifact = { name: 'Context', mediaType: 'text/plain', reference: 'file:///not-authorized' };
  for (const patch of [{ sizeBytes: null }, { sizeBytes: -1 }, { sizeBytes: Number.MAX_SAFE_INTEGER + 1 },
    { digest: null }, { digest: 'sha256:wrong' }, { attachmentId: 'forged' }, { reference: '' },
    { mediaType: 'é'.repeat(1025) }]) {
    assert.throws(() => sdk.events.emit({ ...event(), invocation: { prompt: 'work', artifacts: [{ ...artifact, ...patch }] } }),
      { code: 'INVALID_ARGUMENT' });
  }
  assert.equal(transport.sent.length, 0);
  const input = event();
  input.invocation.artifacts = [{ ...artifact, sizeBytes: 0, digest: `sha256:${'a'.repeat(64)}` }];
  await acknowledge(transport, sdk.events.emit(input));
  assert.deepEqual(transport.sent[0].params, input, 'reference remains metadata in the kernel request');
  sdk.close();
});

test('payload and prompt limits count UTF8 and JSON escapes rather than character length', async () => {
  const { sdk, transport } = setup();
  const input = event();
  const overhead = bytes({ text: '' });
  input.payload.text = `${'é'.repeat(Math.floor((65536 - overhead) / 2))}x`;
  input.invocation.prompt = 'é'.repeat(32768);
  assert.equal(bytes(input.payload), 65536);
  await acknowledge(transport, sdk.events.emit(input));
  for (const invalid of [
    { ...input, payload: { text: `${input.payload.text}x` } },
    { ...input, invocation: { prompt: `${input.invocation.prompt}x`, artifacts: [] } },
    { ...event(), payload: { text: '\u000b'.repeat(16384) } },
  ]) assert.throws(() => sdk.events.emit(invalid), { code: 'LIMIT_EXCEEDED' });
  sdk.close();
});

test('invocation bytes and artifact count have independent exact ceilings', async () => {
  const { sdk, transport } = setup();
  const input = event();
  input.invocation = { prompt: 'x', artifacts: Array.from({ length: 32 }, () => ({
    name: 'x'.repeat(2048), mediaType: 'x'.repeat(2048), reference: 'x'.repeat(2048),
  })) };
  input.invocation.prompt = 'x'.repeat(1 + 256 * 1024 - bytes(input.invocation));
  assert.ok(input.invocation.prompt.length <= 65536);
  assert.equal(bytes(input.invocation), 256 * 1024);
  await acknowledge(transport, sdk.events.emit(input));
  assert.throws(() => sdk.events.emit({ ...input, invocation: { ...input.invocation, prompt: `${input.invocation.prompt}x` } }),
    { code: 'LIMIT_EXCEEDED' });
  assert.throws(() => sdk.events.emit({ ...input, invocation: { ...input.invocation, artifacts: [...input.invocation.artifacts, input.invocation.artifacts[0]] } }),
    { code: 'LIMIT_EXCEEDED' });
  sdk.close();
});

test('combined batch bounds include invocation bytes across different automation IDs', async () => {
  const { sdk, transport } = setup();
  const occurrences = Array.from({ length: 16 }, (_, index) => {
    const input = event(`event-${index}`);
    input.automationId = `automation-${index}`;
    input.payload.text = 'x'.repeat(32768 - bytes(input.invocation) - bytes(input.payload));
    return input;
  });
  assert.equal(occurrences.reduce((sum, input) => sum + bytes(input.payload) + bytes(input.invocation), 0), 512 * 1024);
  await acknowledge(transport, sdk.state.transaction({ schemaVersion: 0, checks: [], writes: [], occurrences }));
  const over = occurrences.map(input => structuredClone(input));
  over[0].payload.text += 'x';
  for (const invalid of [over, Array.from({ length: 17 }, (_, i) => event(`many-${i}`))]) {
    assert.throws(() => sdk.state.transaction({ schemaVersion: 0, checks: [], writes: [], occurrences: invalid }),
      { code: 'LIMIT_EXCEEDED' });
  }
  assert.throws(() => sdk.state.transaction({ schemaVersion: 0, checks: [], writes: [], occurrences: [event(), event()] }),
    { code: 'INVALID_ARGUMENT' });
  assert.equal(transport.sent.length, 1);
  sdk.close();
});

test('payload container and node limits stop overbroad input before serialization', async () => {
  const { sdk, transport } = setup();
  let payload = null;
  for (let depth = 0; depth < 32; depth++) payload = [payload];
  await acknowledge(transport, sdk.events.emit({ ...event(), payload }));
  assert.throws(() => sdk.events.emit({ ...event(), payload: [payload] }), { code: 'LIMIT_EXCEEDED' });
  await acknowledge(transport, sdk.events.emit({ ...event(), payload: Array(16383).fill(0) }));
  assert.throws(() => sdk.events.emit({ ...event(), payload: Array(16384).fill(0) }), { code: 'LIMIT_EXCEEDED' });
  sdk.close();
});
