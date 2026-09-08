import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { resourceFailure, startMacResourceWatch } from './app-runtime-macos-watch.mjs';
import { PROPOSED_BOUNDS as bounds } from './app-runtime-macos-preflight.mjs';

const good = () => ({ availableMemoryBytes: bounds.remainingAvailableMemoryBytes, freeDiskBytes: bounds.remainingDiskBytes,
  groupRssBytes: bounds.observedGroupRssBytes, groupProcesses: bounds.observedGroupProcesses,
  swapBytes: bounds.additionalSwapBytes });

test('every measured threshold has an exact accepted boundary and invalid telemetry stops', () => {
  assert.equal(resourceFailure(good(), 0), null);
  for (const [field, delta, reason] of [['availableMemoryBytes', -1, 'available-memory'], ['freeDiskBytes', -1, 'disk'],
    ['groupRssBytes', 1, 'group-memory'], ['groupProcesses', 1, 'process-count'], ['swapBytes', 1, 'swap-growth']])
    assert.equal(resourceFailure({ ...good(), [field]: good()[field] + delta }, 0), reason);
  for (const value of [null, {}, { ...good(), groupProcesses: 0 }, { ...good(), groupRssBytes: NaN }])
    assert.equal(resourceFailure(value, 0), 'telemetry');
  assert.equal(resourceFailure(good(), NaN), 'telemetry');
  assert.equal(resourceFailure({ ...good(), swapBytes: 0 }, bounds.additionalSwapBytes), null);
});

test('sampling failure, stalled sampling, and elapsed deadline stop exactly once', async () => {
  for (const scenario of ['failure', 'stalled', 'deadline']) {
    const stopped = [];
    const watch = startMacResourceWatch({ initialSwapBytes: 0, intervalMs: 2, sampleTimeoutMs: 10,
      maxDurationMs: scenario === 'deadline' ? 15 : 1000,
      sample: scenario === 'failure' ? async () => { throw new Error('private probe detail'); }
        : scenario === 'stalled' ? () => new Promise(() => {}) : async () => good(),
      stop: reason => stopped.push(reason) });
    const result = await watch.done; watch.close();
    assert.equal(result.reason, scenario === 'deadline' ? 'deadline' : 'telemetry');
    assert.deepEqual(stopped, [result.reason]);
  }
});

test('normal closure ignores a late sample and cancellation never overlaps samples', { timeout: 1000 }, async () => {
  let resolveSample;
  let sampled = 0;
  const stopped = [];
  const watch = startMacResourceWatch({ initialSwapBytes: 0, intervalMs: 1,
    sample: () => { sampled++; return new Promise(resolve => { resolveSample = resolve; }); }, stop: reason => stopped.push(reason) });
  while (!resolveSample) await new Promise(resolve => setTimeout(resolve, 1));
  watch.close(); resolveSample({ ...good(), availableMemoryBytes: 0 });
  assert.equal((await watch.done).reason, null);
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.equal(sampled, 1); assert.deepEqual(stopped, []);
  const controller = new AbortController(); let abortedSample;
  const cancelled = startMacResourceWatch({ initialSwapBytes: 0, intervalMs: 1, signal: controller.signal,
    sample: signal => { abortedSample = signal; return new Promise(() => {}); }, stop: reason => stopped.push(reason) });
  while (!abortedSample) await new Promise(resolve => setTimeout(resolve, 1));
  controller.abort();
  assert.equal((await cancelled.done).reason, 'aborted'); assert.equal(abortedSample.aborted, true);
  assert.deepEqual(stopped, ['aborted']);
});

test('a sampled breach reaches the process owner and stops an owned idle Node heartbeat', { timeout: 5000 }, async t => {
  const scratch = await mkdtemp(join(tmpdir(), 'chariox-mac-watch-'));
  const heartbeat = join(scratch, 'heartbeat');
  // Fixed test code, no App/package/native build and no meaningful allocation.
  const code = "const fs=require('node:fs');fs.writeFileSync(process.argv[1],'x');process.stdout.write('ready');setInterval(()=>fs.appendFileSync(process.argv[1],'x'),10);";
  const child = spawn(process.execPath, ['-e', code, heartbeat], { detached: true, env: {}, stdio: ['ignore', 'pipe', 'ignore'] });
  let closed = false;
  child.once('close', () => { closed = true; });
  const ended = once(child, 'close');
  t.after(async () => {
    if (!closed) { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
    await ended.catch(() => {});
    await rm(scratch, { recursive: true, force: true });
  });
  await once(child.stdout, 'data');
  const watch = startMacResourceWatch({ initialSwapBytes: 0, sample: async () => ({ ...good(), availableMemoryBytes: 0 }),
    stop: () => process.kill(-child.pid, 'SIGKILL') });
  assert.equal((await watch.done).reason, 'available-memory');
  assert.equal((await ended)[1], 'SIGKILL');
  const bytes = await readFile(heartbeat);
  await new Promise(resolve => setTimeout(resolve, 30));
  assert.deepEqual(await readFile(heartbeat), bytes);
});
