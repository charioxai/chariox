// Dedicated CI wrapper. Every image/mount/recovery operation is the production
// Rust module, exercised by ignored hosted tests; no parallel shell provisioner.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { constants, createWriteStream } from 'node:fs';
import { appendFile, lstat, mkdtemp, open, opendir, realpath, rm, statfs, writeFile } from 'node:fs/promises';
import { basename, join, resolve } from 'node:path';
import { totalmem } from 'node:os';
import { startMacResourceWatch, readMacBuildSample } from './app-runtime-macos-watch.mjs';
import { swapUsage } from './app-runtime-macos-preflight.mjs';

assert.equal(process.platform, 'darwin');
assert.equal(process.env.GITHUB_ACTIONS, 'true');
assert.equal(process.env.RUNNER_ENVIRONMENT, 'github-hosted');
assert.equal(process.env.GITHUB_REPOSITORY, 'charioxai/chariox');
assert.ok(totalmem() >= 7 * 1024 ** 3 && process.availableMemory() >= 3 * 1024 ** 3);
const temp = await realpath(process.env.RUNNER_TEMP);
const disk = await statfs(temp, { bigint: true });
assert.ok(disk.bavail * disk.bsize >= 12n * 1024n ** 3n);
const root = await mkdtemp(join(temp, 'chariox-app-storage.'));
const original = await lstat(root);
const evidence = await mkdtemp(join(temp, 'chariox-app-storage-evidence.'));
const target = await mkdtemp(join(temp, 'chariox-app-storage-build.'));
await appendFile(process.env.GITHUB_OUTPUT, `evidence=${evidence}\n`);
const env = { ...process.env, CARGO_BUILD_JOBS: '1', CARGO_INCREMENTAL: '0',
  CARGO_PROFILE_DEV_DEBUG: '0', CARGO_PROFILE_TEST_DEBUG: '0', CARGO_TARGET_DIR: target,
  RUST_TEST_THREADS: '1', CHARIOX_STORAGE_DRILL_ROOT: root };
const initialSwapBytes = swapUsage(execFileSync('/usr/sbin/sysctl', ['-n', 'vm.swapusage'],
  { encoding: 'utf8', timeout: 2000, maxBuffer: 4096 }).trim()).usedBytes;
await writeFile(join(evidence, 'scope.json'), JSON.stringify({ source: execFileSync('/usr/bin/git', ['rev-parse', 'HEAD'],
  { encoding: 'utf8', timeout: 2000 }).trim(), platform: process.platform, arch: process.arch,
  totalMemoryBytes: totalmem(), availableMemoryBytes: process.availableMemory(), initialSwapBytes,
  hardResourceCaps: false, observation: 'Monitored build group and host headroom; fixed private image capacity tested separately. DiskImages service descendants may run outside the sampled group.' }, null, 2));

async function run(label, filter, ignored = false) {
  const stream = createWriteStream(join(evidence, `${label}.log`), { mode: 0o600 });
  const args = ['test', '-p', 'chariox-app-runtime', '--lib', filter, '--', '--test-threads=1', '--nocapture'];
  if (ignored) args.push('--ignored', '--exact');
  const child = spawn('cargo', args, { cwd: resolve(import.meta.dirname, '..'), env, detached: true,
    stdio: ['ignore', 'pipe', 'pipe'] });
  let ended = false;
  let closed = false;
  child.once('exit', () => { ended = true; });
  const exit = new Promise(resolve => {
    child.once('error', () => { ended = true; closed = true; resolve({ code: null, signal: 'spawn-error' }); });
    child.once('close', (code, signal) => { closed = true; resolve({ code, signal }); });
  });
  let logBytes = 0;
  let logFailure = false;
  const stop = () => { if (!ended && !closed) { try { process.kill(-child.pid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; } } };
  // Trusted compiler/test output is still bounded; retain the prefix as evidence.
  stream.on('error', () => { logFailure = true; stop(); });
  for (const output of [child.stdout, child.stderr]) output.on('data', bytes => {
    logBytes += bytes.length;
    if (logBytes > 16 * 1024 * 1024) { logFailure = true; stop(); return; }
    if (!logFailure && !stream.write(bytes)) {
      output.pause(); stream.once('drain', () => output.resume());
    }
  });
  let guard;
  if (child.pid) guard = startMacResourceWatch({
    sample: signal => readMacBuildSample(child.pid, root, signal), initialSwapBytes, maxDurationMs: 5 * 60 * 1000,
    stop,
  });
  const result = await exit;
  guard?.close();
  const observation = guard ? await guard.done : { reason: 'spawn-error' };
  if (!stream.destroyed) await new Promise(resolve => { stream.once('error', resolve); stream.end(resolve); });
  await writeFile(join(evidence, `${label}-resources.json`), JSON.stringify({ ...result, ...observation, logBytes, logFailure }, null, 2));
  return result.code === 0 && observation.reason === null && !logFailure;
}

async function boundedFile(path, maximum) {
  const file = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW);
  try {
    const stat = await file.stat();
    assert.ok(stat.isFile() && stat.uid === process.getuid() && stat.nlink === 1 && stat.size <= maximum);
    const buffer = Buffer.alloc(maximum + 1);
    let length = 0;
    while (length < buffer.length) {
      const { bytesRead } = await file.read(buffer, length, buffer.length - length, length);
      if (!bytesRead) break;
      length += bytesRead;
    }
    assert.ok(length <= maximum);
    return buffer.subarray(0, length);
  } finally { await file.close(); }
}

let main = false;
let cleanup = false;
let compiled = false;
try {
  compiled = await run('offline', 'worker_process::storage_macos::tests::');
  if (compiled) main = await run('real-storage', 'worker_process::storage_macos::tests::hosted::hosted_storage_create_quota_restart_and_crash_recovery', true);
} finally {
  if (compiled) cleanup = await run('cleanup', 'worker_process::storage_macos::tests::hosted::hosted_storage_cleanup', true);
  // Retain bounded journal evidence only, never the disk images or file payloads.
  let count = 0;
  for await (const entry of await opendir(root)) {
    assert.ok(++count <= 64);
    if (!entry.isDirectory() || !/^installation-[a-f0-9]{64}$/.test(entry.name)) continue;
    const path = join(root, entry.name, 'storage.json');
    try { const bytes = await boundedFile(path, 16384); await writeFile(join(evidence, `${entry.name}.json`), bytes); }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
  }
  for (const name of [`${basename(root)}.child.log`, `${basename(root)}.child-stderr.log`]) {
    try { const bytes = await boundedFile(join(temp, name), 1024 * 1024); await writeFile(join(evidence, name), bytes); await rm(join(temp, name)); }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
  }
  if (cleanup || !compiled) {
    const current = await lstat(root);
    assert.ok(current.isDirectory() && !current.isSymbolicLink() && current.ino === original.ino && current.dev === original.dev);
    await rm(root, { recursive: true });
  }
  await rm(target, { recursive: true });
}
assert.ok(compiled && main && cleanup, 'Production storage drill failed; exact journals/logs retained. Failed recovery state is left for disposal with this dedicated runner.');
