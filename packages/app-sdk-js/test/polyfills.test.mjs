import { test } from 'node:test';
import assert from 'node:assert/strict';
import { TextEncoder, TextDecoder } from '../src/encoding.js';
import { AbortController } from '../src/abort.js';

test('UTF-8 encoding matches the platform including lone surrogates and partial destinations', () => {
  for (const text of ['', 'Todo 🍋 日本語', '\ud800', '\udc00', '\ud800x']) {
    assert.deepEqual(new TextEncoder().encode(text), new globalThis.TextEncoder().encode(text));
    for (let n = 0; n < 12; n++) {
      const a = new Uint8Array(n), b = new Uint8Array(n);
      assert.deepEqual(new TextEncoder().encodeInto(text, a), new globalThis.TextEncoder().encodeInto(text, b));
      assert.deepEqual(a, b);
    }
  }
});

test('UTF-8 decoding matches the platform across chunk boundaries and malformed sequences', () => {
  const samples = [new TextEncoder().encode('﻿Todo 🍋 日本語'), [0xf0, 0x9f, 0x80], [0xed, 0xa0, 0x80], [0xe1, 0x80, 65], [0xff, 0x80], [0xc0, 0x80]];
  for (const sample of samples) {
    const bytes = Uint8Array.from(sample);
    for (let split = 0; split <= bytes.length; split++) {
      const a = new TextDecoder(), b = new globalThis.TextDecoder();
      assert.equal(a.decode(bytes.subarray(0, split), { stream: true }), b.decode(bytes.subarray(0, split), { stream: true }));
      assert.equal(a.decode(bytes.subarray(split)), b.decode(bytes.subarray(split)));
    }
  }
  assert.throws(() => new TextDecoder('latin1'), RangeError);
  assert.throws(() => new TextDecoder('utf8', { fatal: true }).decode(Uint8Array.of(0xff)), TypeError);
});

test('abort fires once with the exact reason and supports listener removal', () => {
  const controller = new AbortController();
  let count = 0;
  const removed = () => { throw Error('removed listener ran'); };
  controller.signal.addEventListener('abort', removed);
  controller.signal.removeEventListener('abort', removed);
  controller.signal.addEventListener('abort', () => { count++; }, { once: true });
  controller.abort('stop'); controller.abort('again');
  assert.equal(count, 1);
  assert.throws(() => controller.signal.throwIfAborted(), error => error === 'stop');
});
