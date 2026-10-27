import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { encodeFrame, FrameDecoder, MAX_FRAME_BYTES, validateMessage } from '../src/protocol.js';
import { streamTransport } from '../src/transport.js';
import { envelope, response, TestStream } from './helpers.js';

test('shared Rust and JavaScript wire corpus agrees on every envelope', () => {
  const corpus = JSON.parse(readFileSync(new URL('./wire-vectors.json', import.meta.url), 'utf8'));
  for (const entry of corpus.cases) {
    if (entry.valid) assert.doesNotThrow(() => validateMessage(entry.message, entry.sender), entry.name);
    else assert.throws(() => validateMessage(entry.message, entry.sender), { code: 'PROTOCOL_ERROR' }, entry.name);
  }
  for (const entry of corpus.rawCases) {
    const payload = Buffer.from(entry.json);
    const frame = Buffer.alloc(payload.length + 4);
    frame.writeUInt32BE(payload.length);
    payload.copy(frame, 4);
    const decoder = new FrameDecoder((message) => validateMessage(message, entry.sender));
    if (entry.valid) assert.doesNotThrow(() => decoder.push(frame), entry.name);
    else assert.throws(() => decoder.push(frame), { code: 'PROTOCOL_ERROR' }, entry.name);
  }
});

test('JSON depth is exactly bounded and large integers never silently round', () => {
  let nested = null;
  for (let index = 0; index < 63; index += 1) nested = [nested];
  assert.doesNotThrow(() => encodeFrame(response('deep', nested)), 'Envelope plus63 arrays is64 containers');
  assert.throws(() => encodeFrame(response('too-deep', [nested])), /nesting/);
  for (const value of [NaN, Infinity, -Infinity, 9007199254740992, 1n, '\ud800']) {
    assert.throws(() => encodeFrame(response('bad', value)), { code: 'PROTOCOL_ERROR' });
  }
  assert.throws(() => encodeFrame(response('bad', { '\udfff': null })), /Unicode/);
});

test('fragmented and coalesced frames preserve Unicode, null and message identity', () => {
  const messages = [response('a', null), response('b', { text: 'Á 🚀' })];
  const output = [];
  const decoder = new FrameDecoder((value) => output.push(value));
  const input = Buffer.concat(messages.map((message) => encodeFrame(message)));
  for (let index = 0; index < input.length; index += 3) decoder.push(input.subarray(index, index + 3));
  decoder.end();
  assert.deepEqual(output, messages);
});

test('oversized length is rejected from its header before reading or allocating a body', () => {
  const decoder = new FrameDecoder(() => assert.fail('No oversized message may dispatch'));
  const header = Buffer.alloc(4);
  header.writeUInt32BE(MAX_FRAME_BYTES + 1);
  assert.throws(() => decoder.push(header), { code: 'PROTOCOL_ERROR' });
  assert.throws(() => decoder.push(encodeFrame(response('a', null))), /closed/);
});

test('UTF-8 corruption and premature EOF permanently poison framing', () => {
  const decoder = new FrameDecoder(() => assert.fail('Invalid UTF-8 must not be delivered'));
  assert.throws(() => decoder.push(Buffer.from([0, 0, 0, 1, 255])), /UTF-8/);
  for (const partial of [Buffer.from([0, 0]), encodeFrame(response('a', null)).subarray(0, 9)]) {
    const cut = new FrameDecoder(() => assert.fail('Truncated frames must not dispatch'));
    cut.push(partial);
    assert.throws(() => cut.end(), /Truncated/);
  }
});

test('closed wire shapes reject ambiguous success, unknown fields and unsafe deadlines', () => {
  const cases = [
    { ...response('a', null), error: { code: 'ERROR', message: 'ambiguous' } },
    { ...response('a', null), actor: 'human' },
    envelope({ kind: 'response', id: 'a' }),
    envelope({ kind: 'request', id: 'a', method: 'state.get', params: {}, deadline_ms: 1.5 }),
    envelope({ kind: 'request', id: 'a', method: 'state.get', params: {}, deadline_ms: Number.MAX_SAFE_INTEGER + 1 }),
    envelope({ kind: 'cancel', id: '' }),
  ];
  for (const message of cases) assert.throws(() => validateMessage(message), { code: 'PROTOCOL_ERROR' });
  assert.deepEqual(validateMessage(response('a', null)).result, null);
  assert.throws(() => encodeFrame(response('a', undefined)), { code: 'PROTOCOL_ERROR' });
});

test('transport rejects write saturation without growing an unbounded stream buffer', () => {
  const stream = new TestStream({ stallWrites: true });
  const transport = streamTransport(stream, { maxFrameBytes: 512, maxQueuedBytes: 520 });
  let reason;
  transport.subscribe(() => {}, (error) => { reason = error; });
  transport.send(response('a', 'x'.repeat(350)));
  assert.throws(() => transport.send(response('b', 'y'.repeat(350))), { code: 'BACKPRESSURE' });
  assert.equal(stream.destroyed, true);
  assert.equal(reason.code, 'BACKPRESSURE');
  assert.throws(() => transport.send(response('c', null)), { code: 'DISCONNECTED' });
});

test('partial frame has an absolute assembly deadline, including a header-only stall', async () => {
  const stream = new TestStream();
  const transport = streamTransport(stream, { frameTimeoutMs: 15 });
  const closed = new Promise((resolve) => transport.subscribe(() => assert.fail('No message'), resolve));
  stream.push(Buffer.from([0, 0, 1, 0]));
  const error = await closed;
  assert.match(error.message, /frame deadline/);
  assert.equal(stream.destroyed, true);
});

test('stalled writes close the channel instead of reusing partially written framing', async () => {
  const stream = new TestStream({ stallWrites: true });
  const transport = streamTransport(stream, { frameTimeoutMs: 15 });
  const closed = new Promise((resolve) => transport.subscribe(() => {}, resolve));
  transport.send(response('a', null));
  const error = await closed;
  assert.match(error.message, /write deadline/);
  assert.equal(stream.destroyed, true);
});
