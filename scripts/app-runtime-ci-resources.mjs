// Dedicated, disposable GitHub Linux builds. Shared-host defaults stay separate.
import { readFile } from 'node:fs/promises';
import { freemem } from 'node:os';

const GiB = 1024 ** 3;
const MiB = 1024 ** 2;
const RESOURCE_KEYS = ['maxJobs', 'minTotalMemoryBytes', 'minFreeMemoryBytes', 'minFreeDiskBytes', 'minimumRemainingMemoryBytes', 'minimumRemainingDiskBytes'];

export function validateCiProfile(profile) {
  if (!profile || profile.name !== 'github-linux'
      || !/^node:\d+\.\d+\.\d+-bookworm@sha256:[a-f0-9]{64}$/.test(profile.builderImage)
      || JSON.stringify(Object.keys(profile.resourceBounds ?? {}).sort()) !== JSON.stringify([...RESOURCE_KEYS].sort())
      || RESOURCE_KEYS.some(key => !Number.isSafeInteger(profile.resourceBounds[key]))) {
    throw new Error('invalid dedicated CI resource profile');
  }
  const bounds = profile.resourceBounds;
  if (bounds.maxJobs !== 2 || bounds.minTotalMemoryBytes < 7 * GiB
      || bounds.minFreeMemoryBytes < 3 * GiB || bounds.minFreeDiskBytes < 24 * GiB
      || bounds.minimumRemainingMemoryBytes < 768 * MiB || bounds.minimumRemainingDiskBytes < 4 * GiB
      || profile.containerMemoryBytes !== 6 * GiB || profile.maxCpus !== 2 || profile.maxPids !== 256
      || !Number.isSafeInteger(profile.maxArtifactBytes) || profile.maxArtifactBytes < 1 || profile.maxArtifactBytes > 512 * MiB) {
    throw new Error('dedicated CI resource limits cannot be weakened');
  }
  return profile;
}

function unsigned(value) {
  if (typeof value !== 'string' || !/^\d+$/.test(value.trim())) throw new Error('missing finite cgroup limit');
  const number = Number(value.trim());
  if (!Number.isSafeInteger(number)) throw new Error('invalid cgroup number');
  return number;
}

export function checkCiIsolation(profile, observed, environment, platform = process.platform) {
  validateCiProfile(profile);
  if (platform !== 'linux' || environment.GITHUB_ACTIONS !== 'true' || environment.RUNNER_ENVIRONMENT !== 'github-hosted') {
    throw new Error('dedicated CI profile requires a disposable GitHub-hosted Linux container');
  }
  const memory = unsigned(observed['memory.max']);
  const current = unsigned(observed['memory.current']);
  const inactiveFile = unsigned(/^inactive_file (\d+)$/m.exec(observed['memory.stat'] ?? '')?.[1]);
  const swap = unsigned(observed['memory.swap.max']);
  const pids = unsigned(observed['pids.max']);
  const cpu = observed['cpu.max']?.trim().split(/\s+/);
  if (cpu?.length !== 2) throw new Error('missing cgroup CPU quota');
  const quota = unsigned(cpu[0]);
  const period = unsigned(cpu[1]);
  if (memory !== profile.containerMemoryBytes || current > memory || inactiveFile > current || swap !== 0
      || pids < 16 || pids > profile.maxPids || quota < 1 || period < 1 || quota / period > profile.maxCpus) {
    throw new Error('dedicated CI requires enforced memory, no-swap, CPU and PID limits');
  }
  // Match Docker's cgroup-v2 working-set accounting: inactive file cache is
  // reclaimable. The hard memory.max still bounds every charged byte.
  return memory - current + inactiveFile;
}

export async function readCiCgroup() {
  const names = ['memory.max', 'memory.current', 'memory.stat', 'memory.swap.max', 'cpu.max', 'pids.max'];
  return Object.fromEntries(await Promise.all(names.map(async name => [name, await readFile(`/sys/fs/cgroup/${name}`, 'utf8')])));
}

export async function availableBuildMemory(plan) {
  if (plan.resourceProfile === 'github-macos') {
    validateMacProfile(plan.ciProfile);
    if (process.platform !== 'darwin' || typeof process.availableMemory !== 'function') throw new Error('macOS available-memory telemetry is required');
    return process.availableMemory();
  }
  if (plan.resourceProfile !== 'github-linux') return freemem();
  const [cgroup, meminfo] = await Promise.all([readCiCgroup(), readFile('/proc/meminfo', 'utf8')]);
  const available = checkCiIsolation(plan.ciProfile, cgroup, process.env);
  return Math.min(hostAvailableMemory(meminfo), available);
}

export function validateMacProfile(profile) {
  const expected = {
    schema: 'chariox.app-runtime-macos-builder.v1', name: 'github-macos', target: 'darwin-arm64', runner: 'macos-15',
    developerDirectory: '/Applications/Xcode_16.4.app/Contents/Developer', xcodeBuild: '16F6', sdkVersion: '15.5',
    compilerBuild: 'clang-1700.0.13.5',
    resourceBounds: { maxJobs: 1, minTotalMemoryBytes: 7 * GiB, minFreeMemoryBytes: 3 * GiB,
      minFreeDiskBytes: 24 * GiB, minimumRemainingMemoryBytes: GiB, minimumRemainingDiskBytes: 4 * GiB },
    observedGroupRssBytes: 4 * GiB, observedGroupProcesses: 128, additionalSwapBytes: 256 * MiB,
    sampleIntervalMs: 1000, sampleTimeoutMs: 3000, maxDurationMs: 300 * 60 * 1000,
    maxArtifactBytes: 512 * MiB, enforcement: 'monitored-disposable-vm-not-hard-memory-cpu-or-swap-caps',
  };
  const sorted = value => value && typeof value === 'object' && !Array.isArray(value)
    ? Object.fromEntries(Object.keys(value).sort().map(key => [key, sorted(value[key])])) : value;
  if (JSON.stringify(sorted(profile)) !== JSON.stringify(sorted(expected))) throw new Error('invalid dedicated macOS profile; observed limits cannot be weakened');
  return profile;
}

export function hostAvailableMemory(meminfo) {
  const value = /^MemAvailable:\s+(\d+)\s+kB$/m.exec(meminfo)?.[1];
  const bytes = unsigned(value) * 1024;
  if (!Number.isSafeInteger(bytes)) throw new Error('invalid host available memory');
  return bytes;
}

export function checkArtifactBudget(profile, sizes) {
  if (!profile) return;
  if (sizes.some(size => !Number.isSafeInteger(size) || size < 0)
      || sizes.reduce((total, size) => total + size, 0) > profile.maxArtifactBytes) {
    throw new Error('native CI artifacts exceed size limit');
  }
}
