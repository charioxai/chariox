import test from 'node:test';
import assert from 'node:assert/strict';
import { occurrenceId } from '../src/index.js';
import { validateOccurrence } from '../src/occurrences.js';

test('source identity hashes match independent UTF-8 and escaping vectors', () => {
  // Independently generated with Python json.dumps(ensure_ascii=False,
  // separators=(',', ':')) + hashlib.sha256 over UTF-8.
  assert.equal(occurrenceId('source/é😀', 1000),
    'evt1.1000.21d560d98416f29e501984812520f520d02ac1907e3455d85bf17e219a8cd0cf');
  assert.equal(occurrenceId('a"\n\\b', 0),
    'evt1.0.65a334110b948868e0415f900bc8df7682229878dcbabf555017ca49e1b8f4df');
  assert.equal(occurrenceId('same', 0), occurrenceId('same', -0));
  assert.equal(occurrenceId('same', 1000), occurrenceId('same', 1000));
  assert.notEqual(occurrenceId('same', 1000), occurrenceId('other', 1000));
  assert.notEqual(occurrenceId('same', 1000), occurrenceId('same', 1001));
});

test('source helper enforces Unicode, UTF-8 byte and safe timestamp boundaries', () => {
  assert.match(occurrenceId('😀'.repeat(1024), Number.MAX_SAFE_INTEGER), /^evt1\.9007199254740991\.[a-f0-9]{64}$/u);
  for (const source of ['', null, {}, '\ud800', '😀'.repeat(1024) + 'a', 'a'.repeat(4097)]) {
    assert.throws(() => occurrenceId(source, 0), { code: 'INVALID_ARGUMENT' });
  }
  for (const time of [-1, 0.1, Infinity, NaN, Number.MAX_SAFE_INTEGER + 1, '1000', null]) {
    assert.throws(() => occurrenceId('source', time), { code: 'INVALID_ARGUMENT' });
  }
});

test('occurrence validation rejects changed time and noncanonical or legacy IDs', () => {
  const original = { automationId: 'automation', occurrenceId: occurrenceId('source', 1000),
    eventVersion: 1, occurredAtMs: 1000, payload: {}, invocation: { prompt: 'Handle event', artifacts: [] } };
  assert.equal(validateOccurrence(original), original);
  for (const id of ['source', original.occurrenceId.replace('.1000.', '.01000.'),
    original.occurrenceId.replace('.1000.', '.1e3.'), original.occurrenceId.toUpperCase(),
    original.occurrenceId.replace('evt1.', 'evt2.'), original.occurrenceId + 'a']) {
    assert.throws(() => validateOccurrence({ ...original, occurrenceId: id }), { code: 'INVALID_ARGUMENT' });
  }
  assert.throws(() => validateOccurrence({ ...original, occurredAtMs: 1001 }), { code: 'INVALID_ARGUMENT' });
});
