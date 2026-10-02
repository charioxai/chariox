import assert from 'node:assert/strict';
import test from 'node:test';
import { NATIVE_CHECKS, parseNativeChecks } from './linux-native-contract.mjs';

const lines = NATIVE_CHECKS.map(name => `${name}:ok`);
test('Linux libc probe has exactly 30 distinct passing checks', () => {
  assert.equal(NATIVE_CHECKS.length, 30);
  assert.equal(Object.keys(parseNativeChecks(`${lines.join('\n')}\n`)).length, 30);
});
test('missing, duplicate, extra, failed and malformed native checks fail closed', () => {
  for (const output of [
    lines.slice(1), [...lines, 'unexpected:ok'], [lines[1], ...lines.slice(1)],
    ['unexpected:ok', ...lines.slice(1)], [`${NATIVE_CHECKS[0]}:FAIL`, ...lines.slice(1)],
    [`${lines[0]}:extra`, ...lines.slice(1)], [],
  ]) assert.throws(() => parseNativeChecks(output.join('\n')));
});
