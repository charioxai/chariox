import assert from 'node:assert/strict';
import test from 'node:test';
import { admission, processGroup, PROPOSED_BOUNDS, swapUsage } from './app-runtime-macos-preflight.mjs';

test('swap telemetry preserves units and rejects malformed or impossible counters', () => {
  const result = swapUsage('total = 1024.00M  used = 12.50M  free = 1011.50M  (encrypted)');
  assert.equal(result.usedBytes, 12.5 * 1024 ** 2);
  for (const value of ['total=NaNM used=0M free=0M', 'total=1M used=2M free=0M',
    'total=1M used=0M free=2M', 'x'.repeat(4097)]) assert.throws(() => swapUsage(value));
});

test('process measurement counts only the exclusive metadata group and remains observational', () => {
  const result = processGroup(' 100 1 100 20\n 101 100 100 30\n 102 1 102 999\n', 100);
  assert.equal(result.rssBytes, 50 * 1024); assert.equal(result.processCount, 2);
  assert.match(result.observation, /no limits enforced/);
  for (const rows of ['100 1 99 20', '101 1 101 20', '100 1 100 -1',
    '100 1 100 2\n100 1 100 3', '100 1 100 1\n'.repeat(8193)])
    assert.throws(() => processGroup(rows, 100));
});

test('preflight threshold report never authorizes or claims a native build', () => {
  const result = admission(PROPOSED_BOUNDS);
  assert.equal(result.wouldMeetProposedStartThresholds, true);
  assert.equal(result.buildAuthorized, false); assert.equal(result.compilationPerformed, false);
  const low = admission({ ...PROPOSED_BOUNDS, availableMemoryBytes: 0, freeDiskBytes: 14 * 1024 ** 3 });
  assert.deepEqual(low.failures, ['availableMemoryBytes', 'freeDiskBytes']);
  assert.throws(() => admission({ ...PROPOSED_BOUNDS, totalMemoryBytes: NaN }));
});
