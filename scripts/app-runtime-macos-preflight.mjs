// Read-only hosted measurements. This does not build, sign, delete SDKs, or
// enforce a runtime sandbox/resource quota.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { appendFile, lstat, mkdtemp, realpath, statfs, writeFile } from 'node:fs/promises';
import { availableParallelism, freemem, totalmem } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const GiB = 1024 ** 3;
const MiB = 1024 ** 2;
const XCODE = '/Applications/Xcode_16.4.app/Contents/Developer';
const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export const UNUSED_XCODE = Object.freeze(['/Applications/Xcode_26.3.app', '/Applications/Xcode_26.2.app', '/Applications/Xcode_16.3.app']);
export const PROPOSED_BOUNDS = Object.freeze({ totalMemoryBytes: 7 * GiB, availableMemoryBytes: 3 * GiB,
  freeDiskBytes: 24 * GiB, remainingAvailableMemoryBytes: GiB, remainingDiskBytes: 4 * GiB,
  observedGroupRssBytes: 4 * GiB, observedGroupProcesses: 128, additionalSwapBytes: 256 * MiB });

function integer(value) {
  assert.match(value, /^\d+$/);
  const number = Number(value); assert.ok(Number.isSafeInteger(number));
  return number;
}

export function swapUsage(text) {
  assert.ok(Buffer.byteLength(text) <= 4096);
  const match = /^total\s*=\s*(\d+(?:\.\d+)?)M\s+used\s*=\s*(\d+(?:\.\d+)?)M\s+free\s*=\s*(\d+(?:\.\d+)?)M\s*(?:\(encrypted\))?\s*$/.exec(text);
  assert.ok(match, 'unexpected vm.swapusage');
  const values = match.slice(1).map(value => Math.round(Number(value) * MiB));
  assert.ok(values.every(Number.isSafeInteger));
  assert.ok(values[1] <= values[0] && values[2] <= values[0]);
  return { totalBytes: values[0], usedBytes: values[1], freeBytes: values[2], precision: 'sysctl decimal MiB, rounded to bytes' };
}

export function processGroup(text, pid) {
  assert.ok(Buffer.byteLength(text) <= MiB && Number.isSafeInteger(pid) && pid > 0);
  const lines = text.trim().split('\n'); assert.ok(lines.length <= 8192);
  const rows = lines.map(line => {
    const fields = line.trim().split(/\s+/); assert.equal(fields.length, 4);
    const [id, parent, group, rss] = fields.map(integer);
    return { pid: id, parentPid: parent, group, rssBytes: rss * 1024 };
  });
  assert.equal(new Set(rows.map(row => row.pid)).size, rows.length);
  const observer = rows.find(row => row.pid === pid); assert.ok(observer);
  assert.equal(observer.group, pid, 'preflight requires its own metadata process group');
  const members = rows.filter(row => row.group === pid);
  const rssBytes = members.reduce((sum, row) => sum + row.rssBytes, 0);
  assert.ok(Number.isSafeInteger(rssBytes));
  return { leaderPid: pid, processCount: members.length, rssBytes, observation: 'one ps snapshot; includes only this metadata process group; no limits enforced' };
}

export function admission(observed) {
  const failures = [];
  for (const key of ['totalMemoryBytes', 'availableMemoryBytes', 'freeDiskBytes']) {
    assert.ok(Number.isSafeInteger(observed[key]) && observed[key] >= 0);
    if (observed[key] < PROPOSED_BOUNDS[key]) failures.push(key);
  }
  return { wouldMeetProposedStartThresholds: failures.length === 0, failures,
    buildAuthorized: false, compilationPerformed: false, thresholdsMeasuredUnderBuild: false };
}

function command(program, args, timeout = 10000, maxBuffer = 65536) {
  return execFileSync(program, args, { encoding: 'utf8', timeout, killSignal: 'SIGKILL', maxBuffer,
    env: { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', LANG: 'C', LC_ALL: 'C', DEVELOPER_DIR: XCODE },
    stdio: ['ignore', 'pipe', 'pipe'] }).trim();
}

async function fingerprint(path) {
  path = await realpath(path);
  const metadata = await lstat(path); assert.ok(metadata.isFile());
  const hash = createHash('sha256');
  for await (const chunk of createReadStream(path, { highWaterMark: 65536 })) hash.update(chunk);
  return { path, size: metadata.size, sha256: hash.digest('hex') };
}

async function xcodeSizes() {
  const rows = [];
  for (const path of UNUSED_XCODE) {
    const metadata = await lstat(path).catch(error => error.code === 'ENOENT' ? null : Promise.reject(error));
    if (!metadata) { rows.push({ path, present: false }); continue; }
    if (!metadata.isDirectory() || metadata.isSymbolicLink()) {
      rows.push({ path, present: true, measurement: 'not-a-regular-directory' }); continue;
    }
    try {
      const text = command('/usr/bin/du', ['-sk', path], 15000, 4096);
      const match = /^(\d+)\s+(.+)$/.exec(text); assert.ok(match && match[2] === path);
      const allocatedBytes = integer(match[1]) * 1024; assert.ok(Number.isSafeInteger(allocatedBytes));
      rows.push({ path, present: true, ownerUid: metadata.uid, allocatedBytes });
    } catch { rows.push({ path, present: true, measurement: 'unavailable-within-15-second-bound' }); }
  }
  return { paths: rows, deletionPerformed: false,
    note: 'du allocation is not promised reclaimable space: APFS clones/shared extents may overlap.' };
}

async function preflight() {
  assert.equal(process.env.GITHUB_ACTIONS, 'true');
  assert.equal(process.env.RUNNER_ENVIRONMENT, 'github-hosted');
  assert.equal(process.env.GITHUB_REPOSITORY, 'charioxai/chariox');
  assert.equal(process.platform, 'darwin'); assert.equal(process.arch, 'arm64');
  assert.equal(process.versions.node, '24.20.0');
  assert.equal(typeof process.availableMemory, 'function');
  const scratch = await mkdtemp(join(await realpath(process.env.RUNNER_TEMP), 'chariox-macos-preflight.'));
  await appendFile(process.env.GITHUB_OUTPUT, `scratch=${scratch}\nevidence=${scratch}/preflight.json\n`);
  const report = { schema: 'chariox.app-runtime-macos-preflight.v1', observedAt: new Date().toISOString(),
    runnerImage: { os: process.env.ImageOS ?? null, version: process.env.ImageVersion ?? null },
    scope: 'Dedicated hosted metadata only. No native compile, deletion, signing, App execution, hard memory cap or no-swap enforcement.',
    proposedBounds: PROPOSED_BOUNDS, complete: false };
  try {
    report.sourceCommit = command('/usr/bin/git', ['-C', REPOSITORY, 'rev-parse', 'HEAD']);
    const disk = await statfs(scratch, { bigint: true });
    report.resources = { logicalCpuCount: availableParallelism(), totalMemoryBytes: totalmem(), freeMemoryBytes: freemem(),
      availableMemoryBytes: process.availableMemory(), freeDiskBytes: Number(disk.bavail * disk.bsize),
      swap: swapUsage(command('/usr/sbin/sysctl', ['-n', 'vm.swapusage'])),
      vmStatistics: command('/usr/bin/vm_stat', []),
      metadataProcessGroup: processGroup(command('/bin/ps', ['-axo', 'pid=,ppid=,pgid=,rss='], 10000, MiB), process.pid) };
    report.admission = admission(report.resources);
    const clang = command('/usr/bin/xcrun', ['--find', 'clang']);
    const clangxx = command('/usr/bin/xcrun', ['--find', 'clang++']);
    const python = await realpath(process.env.PREFLIGHT_PYTHON);
    report.toolchain = { selectedDeveloper: command('/usr/bin/xcode-select', ['-p']),
      xcode: command('/usr/bin/xcodebuild', ['-version']),
      sdkVersion: command('/usr/bin/xcrun', ['--sdk', 'macosx', '--show-sdk-version']),
      sdkPath: command('/usr/bin/xcrun', ['--sdk', 'macosx', '--show-sdk-path']),
      clang: command(clang, ['--version']), clangxx: command(clangxx, ['--version']),
      python: command(python, ['--version']), make: command('/usr/bin/make', ['--version']),
      node: process.versions.node, os: command('/usr/bin/sw_vers', []),
      files: await Promise.all([clang, clangxx, python, process.execPath, '/usr/bin/make'].map(fingerprint)) };
    const toolchain = report.toolchain;
    assert.equal(toolchain.selectedDeveloper, XCODE);
    assert.equal(toolchain.xcode, 'Xcode 16.4\nBuild version 16F6');
    assert.equal(toolchain.sdkVersion, '15.5');
    assert.ok(clang.startsWith(`${XCODE}/Toolchains/`) && clangxx.startsWith(`${XCODE}/Toolchains/`));
    assert.match(toolchain.clang, /^Apple clang version 17\.0\.0\s/);
    assert.match(toolchain.clangxx, /^Apple clang version 17\.0\.0\s/);
    assert.equal(toolchain.python, 'Python 3.11.9');
    assert.match(toolchain.make, /^GNU Make 3\.81\n/);
    report.unusedXcode = await xcodeSizes();
    report.complete = true;
  } finally {
    await writeFile(join(scratch, 'preflight.json'), `${JSON.stringify(report, null, 2)}\n`, { mode: 0o600 });
    console.log(JSON.stringify({ complete: report.complete, admission: report.admission, resources: report.resources }));
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href)
  preflight().catch(() => { console.error('app_runtime_macos_preflight_failed'); process.exitCode = 1; });
