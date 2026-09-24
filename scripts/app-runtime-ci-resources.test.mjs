import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { checkArtifactBudget, checkCiIsolation, hostAvailableMemory, validateCiProfile } from './app-runtime-ci-resources.mjs';
import { checkBuildResources, createPlan, parseOptions, validateLock } from './build-app-runtime.mjs';

const lock = JSON.parse(await readFile(new URL('../apps/app-worker/runtime.lock.json', import.meta.url), 'utf8'));
const GiB = 1024 ** 3;
const hosted = { GITHUB_ACTIONS: 'true', RUNNER_ENVIRONMENT: 'github-hosted' };
const cgroup = {
  'memory.max': String(6 * GiB), 'memory.current': String(2 * GiB),
  'memory.stat': 'anon 2147483648\ninactive_file 0\n',
  'memory.swap.max': '0', 'cpu.max': '200000 100000', 'pids.max': '256',
};

test('dedicated profile is explicit and never changes the shared-host defaults', () => {
  validateLock(lock);
  const options = { mode: 'plan', target: 'linux-x64', scratch: '/outside/runtime', jobs: 1 };
  const ordinary = createPlan(options, lock);
  assert.equal(ordinary.resourceProfile, 'default');
  assert.equal(ordinary.resourceBounds.minTotalMemoryBytes, 16 * GiB);
  assert.equal(ordinary.resourceBounds.minFreeMemoryBytes, 8 * GiB);
  assert.equal(ordinary.resourceBounds.minFreeDiskBytes, 32 * GiB);
  const ci = createPlan({ ...options, resourceProfile: 'github-linux' }, lock);
  const observed = { totalMemoryBytes: 7.5 * GiB, freeMemoryBytes: 4 * GiB, freeDiskBytes: 30 * GiB };
  checkBuildResources(ci.resourceBounds, observed);
  assert.throws(() => checkBuildResources(ordinary.resourceBounds, observed));
  const twoJobs = createPlan({ ...options, resourceProfile: 'github-linux', jobs: 2 }, lock);
  assert.ok(twoJobs.commands.some(command => command.args.includes('-j2')));
  assert.throws(() => createPlan({ ...options, resourceProfile: 'github-linux', jobs: 3 }, lock));
  assert.throws(() => createPlan({ ...options, resourceProfile: 'github-linux', target: 'darwin-arm64' }, lock));
  assert.throws(() => createPlan({ ...options, resourceProfile: 'unrestricted' }, lock));
  assert.equal(parseOptions(['plan', '--target', 'linux-x64', '--scratch', '/outside/runtime', '--resource-profile', 'github-linux']).resourceProfile, 'github-linux');
});

test('CI requires real finite cgroup limits in the disposable hosted environment', () => {
  const profile = lock.dedicatedCi;
  assert.equal(checkCiIsolation(profile, cgroup, hosted, 'linux'), 4 * GiB);
  for (const [key, bad] of [
    ['memory.max', 'max'], ['memory.max', String(8 * GiB)], ['memory.current', String(7 * GiB)],
    ['memory.swap.max', '1'], ['memory.swap.max', 'max'], ['cpu.max', 'max 100000'],
    ['cpu.max', '300000 100000'], ['cpu.max', '0 0'], ['pids.max', '257'], ['pids.max', 'max'],
  ]) assert.throws(() => checkCiIsolation(profile, { ...cgroup, [key]: bad }, hosted, 'linux'));
  for (const key of Object.keys(cgroup)) {
    const missing = { ...cgroup }; delete missing[key];
    assert.throws(() => checkCiIsolation(profile, missing, hosted, 'linux'));
  }
  for (const environment of [{}, { ...hosted, RUNNER_ENVIRONMENT: 'self-hosted' }]) {
    assert.throws(() => checkCiIsolation(profile, cgroup, environment, 'linux'), /disposable/);
  }
  assert.throws(() => checkCiIsolation(profile, cgroup, hosted, 'darwin'), /disposable/);
  assert.equal(checkCiIsolation(profile, { ...cgroup, 'memory.current': String(6 * GiB - 1024) }, hosted, 'linux'), 1024);
  assert.equal(checkCiIsolation(profile, { ...cgroup, 'memory.current': String(6 * GiB), 'memory.stat': `inactive_file ${2 * GiB}\n` }, hosted, 'linux'), 2 * GiB);
  assert.throws(() => checkCiIsolation(profile, { ...cgroup, 'memory.stat': `inactive_file ${3 * GiB}\n` }, hosted, 'linux'));
});

test('host availability includes reclaimable memory but rejects missing measurements', () => {
  assert.equal(hostAvailableMemory('MemTotal: 8000000 kB\nMemFree: 500000 kB\nMemAvailable: 4000000 kB\n'), 4096000000);
  for (const invalid of ['', 'MemFree: 4000000 kB', 'MemAvailable: max kB', 'MemAvailable: -1 kB']) {
    assert.throws(() => hostAvailableMemory(invalid));
  }
});

test('CI profile and complete artifact budget cannot silently widen', () => {
  for (const mutate of [
    profile => { profile.resourceBounds.maxJobs = 3; },
    profile => { profile.resourceBounds.minimumRemainingMemoryBytes = 0; },
    profile => { profile.resourceBounds.minFreeDiskBytes = 1; },
    profile => { profile.containerMemoryBytes = 8 * GiB; },
    profile => { profile.maxArtifactBytes = GiB; },
    profile => { profile.builderImage = 'node:latest'; },
  ]) {
    const changed = structuredClone(lock.dedicatedCi); mutate(changed);
    assert.throws(() => validateCiProfile(changed));
  }
  const cap = lock.dedicatedCi.maxArtifactBytes;
  checkArtifactBudget(lock.dedicatedCi, [cap - 1024, 1024]);
  assert.throws(() => checkArtifactBudget(lock.dedicatedCi, [cap, 1]));
  assert.throws(() => checkArtifactBudget(lock.dedicatedCi, [NaN]));
});


test('hosted retry changes build parallelism and deadline without widening hard limits', async () => {
  const recipe = await readFile(new URL('./run-app-runtime-native-ci.sh', import.meta.url), 'utf8');
  const workflow = await readFile(new URL('../.github/workflows/app-runtime-native.yml', import.meta.url), 'utf8');
  assert.match(recipe, /timeout --signal=TERM --kill-after=10s 300m docker run/);
  assert.match(recipe, /--cpus=2 --memory=6g --memory-swap=6g --pids-limit=256/);
  assert.match(recipe, /--target linux-x64 --jobs 2/);
  assert.match(workflow, /timeout-minutes: 330/);
  assert.equal(lock.dedicatedCi.resourceBounds.minimumRemainingMemoryBytes, 768 * 1024 ** 2);
  assert.equal(lock.dedicatedCi.resourceBounds.minimumRemainingDiskBytes, 4 * GiB);
});
