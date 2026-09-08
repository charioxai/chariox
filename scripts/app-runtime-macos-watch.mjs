// Dedicated builder monitoring only. Not a hard RAM/CPU/swap quota and not an
// App sandbox. Not wired into the native build driver until separately reviewed.
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { statfs } from 'node:fs/promises';
import { isAbsolute } from 'node:path';
import { performance } from 'node:perf_hooks';
import { promisify } from 'node:util';
import { processGroup, PROPOSED_BOUNDS, swapUsage } from './app-runtime-macos-preflight.mjs';

const execute = promisify(execFile);
const MAX_DURATION_MS = 165 * 60 * 1000;

function unsigned(value) { return Number.isSafeInteger(value) && value >= 0; }

export function resourceFailure(sample, initialSwapBytes) {
  const fields = ['availableMemoryBytes', 'freeDiskBytes', 'groupRssBytes', 'groupProcesses', 'swapBytes'];
  if (!unsigned(initialSwapBytes) || !sample || fields.some(field => !unsigned(sample[field]))
    || sample.groupProcesses < 1) return 'telemetry';
  if (sample.availableMemoryBytes < PROPOSED_BOUNDS.remainingAvailableMemoryBytes) return 'available-memory';
  if (sample.freeDiskBytes < PROPOSED_BOUNDS.remainingDiskBytes) return 'disk';
  if (sample.groupRssBytes > PROPOSED_BOUNDS.observedGroupRssBytes) return 'group-memory';
  if (sample.groupProcesses > PROPOSED_BOUNDS.observedGroupProcesses) return 'process-count';
  if (sample.swapBytes - initialSwapBytes > PROPOSED_BOUNDS.additionalSwapBytes) return 'swap-growth';
  return null;
}

/** The caller must retain ownership of this spawned process group through exit
 * and cleanup. A PID supplied by a terminal/App is not an admissible input. */
export async function readMacBuildSample(leaderPid, scratch, signal) {
  assert.equal(process.platform, 'darwin');
  assert.ok(Number.isSafeInteger(leaderPid) && leaderPid > 1 && isAbsolute(scratch));
  assert.equal(typeof process.availableMemory, 'function');
  const options = { encoding: 'utf8', timeout: 2000, killSignal: 'SIGKILL', maxBuffer: 1024 * 1024,
    env: { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', LANG: 'C', LC_ALL: 'C' }, signal };
  const [processes, swap, disk] = await Promise.all([
    execute('/bin/ps', ['-axo', 'pid=,ppid=,pgid=,rss='], options),
    execute('/usr/sbin/sysctl', ['-n', 'vm.swapusage'], { ...options, maxBuffer: 4096 }),
    statfs(scratch, { bigint: true }),
  ]);
  const group = processGroup(processes.stdout, leaderPid);
  return { availableMemoryBytes: process.availableMemory(), freeDiskBytes: Number(disk.bavail * disk.bsize),
    groupRssBytes: group.rssBytes, groupProcesses: group.processCount, swapBytes: swapUsage(swap.stdout.trim()).usedBytes };
}

/** The command owner supplies a synchronous stop(reason) callback that requests
 * group termination. It must separately await exit/reaping and retain ownership
 * through cleanup: done reports the guard decision, not process completion.
 * This module never starts or signals a build itself. close() is for normal
 * command completion; abort/limits invoke stop once. */
export function startMacResourceWatch({ sample, stop, initialSwapBytes, signal,
  intervalMs = 1000, sampleTimeoutMs = 3000, maxDurationMs = MAX_DURATION_MS }) {
  assert.equal(typeof sample, 'function'); assert.equal(typeof stop, 'function');
  assert.ok(unsigned(initialSwapBytes));
  for (const [value, maximum] of [[intervalMs, 1000], [sampleTimeoutMs, 3000], [maxDurationMs, MAX_DURATION_MS]])
    assert.ok(Number.isSafeInteger(value) && value >= 1 && value <= maximum);
  const started = performance.now();
  const controller = new AbortController();
  let timer;
  let finished = false;
  let lastSample = null;
  let settle;
  let cancelSample;
  const done = new Promise(resolve => { settle = resolve; });
  const complete = reason => {
    if (finished) return;
    finished = true; clearTimeout(timer); cancelSample?.(); controller.abort();
    signal?.removeEventListener('abort', abort);
    let stopFailed = false;
    if (reason !== null) { try { stop(reason); } catch { stopFailed = true; } }
    settle({ reason, stopFailed, lastSample });
  };
  const abort = () => complete('aborted');

  async function tick() {
    if (finished) return;
    const remaining = maxDurationMs - (performance.now() - started);
    if (remaining <= 0) { complete('deadline'); return; }
    let sampleTimer;
    let timedOut = false;
    try {
      const result = await Promise.race([
        Promise.resolve().then(() => sample(controller.signal)),
        new Promise((_, reject) => {
          cancelSample = () => { clearTimeout(sampleTimer); reject(new Error('closed')); };
          sampleTimer = setTimeout(() => { timedOut = true; reject(new Error('timeout')); }, Math.min(sampleTimeoutMs, remaining));
        }),
      ]);
      if (finished) return;
      lastSample = result;
      const failure = resourceFailure(result, initialSwapBytes);
      if (failure !== null) { complete(failure); return; }
      timer = setTimeout(tick, Math.min(intervalMs, Math.max(1, maxDurationMs - (performance.now() - started))));
    } catch {
      if (!finished) complete(timedOut && performance.now() - started >= maxDurationMs ? 'deadline' : 'telemetry');
    } finally { clearTimeout(sampleTimer); cancelSample = undefined; }
  }
  signal?.addEventListener('abort', abort, { once: true });
  if (signal?.aborted) complete('aborted');
  else timer = setTimeout(tick, 0);
  return Object.freeze({ done, close: () => complete(null) });
}
