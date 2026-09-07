import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { chmod, mkdtemp, mkdir, readFile, readdir, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

import { checkBuildResources, checkDependencies, checkedOutput, checkToolchainVersions, createPlan, parseOptions, runCommand, validateLock, validateScratch, verifySource } from './build-app-runtime.mjs';

const repository = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const lock = JSON.parse(await readFile(join(repository, 'apps/app-worker/runtime.lock.json'), 'utf8'));

async function temporary(t) {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'chariox-runtime-build-test-')));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

test('pinned source identity and platform ABI contract are explicit', () => {
  validateLock(lock);
  assert.equal(lock.node.version, '24.20.0');
  assert.equal(lock.node.sha256, '2732fc3f588dd335cd6779c06864f7cd424bb1b5ff9a1743059a66c54f9ca4a1');
  assert.equal(lock.loading.launcherLinksNode, false);
  assert.equal(lock.loading.mode, 'sandbox-before-dlopen');
  for (const mutate of [
    value => { value.node.url = 'https://untrusted.invalid/source.tar.xz'; },
    value => { value.node.configure.push('--shared-openssl'); },
    value => { value.build.maxJobs = 20; },
    value => { value.build.minFreeDiskBytes = 1; },
    value => { delete value.build.maxSourceArchiveBytes; },
    value => { value.build.minimumRemainingMemoryBytes = 0; },
    value => { value.targets['linux-x64'].nodeLibrary = '../../libnode.so'; },
    value => { value.loading.launcherLinksNode = true; },
  ]) {
    const changed = structuredClone(lock); mutate(changed);
    assert.throws(() => validateLock(changed));
  }
});

test('build requires explicit source selection; ambiguous options are rejected', () => {
  const base = ['build', '--target', 'linux-x64', '--scratch', '/safe/build'];
  assert.throws(() => parseOptions(base), /explicit --download-source/);
  assert.throws(() => parseOptions([...base, '--download-source', '--source-archive', '/cache/node.tar.xz']));
  assert.throws(() => parseOptions([...base, '--download-source', '--download-source']), /duplicate/);
  assert.throws(() => parseOptions([...base, '--run-app', 'x']));
  const parsed = parseOptions([...base, '--source-archive', '/cache/node.tar.xz', '--jobs', '2']);
  assert.equal(parsed.downloadSource, false);
  assert.equal(parsed.jobs, 2);
});

test('all four plans build a shared runtime without executing an App or Node test program', () => {
  for (const target of Object.keys(lock.targets)) {
    const plan = createPlan({ mode: 'plan', target, scratch: '/safe/build', jobs: 1 }, lock, '/source/chariox');
    assert.equal(plan.requiresNativeHost, true);
    assert.equal(plan.artifactSigning, 'unsigned');
    assert.equal(plan.commands.length, 3);
    assert.deepEqual(plan.commands[1].args, ['-C', 'out', 'BUILDTYPE=Release', '-j1', 'libnode']);
    assert.ok(plan.commands[0].args.includes('--shared'));
    const args = plan.commands[2].args;
    assert.ok(args.includes(join(plan.source, 'out/Release', plan.targetContract.nodeLibrary)));
    assert.ok(args.includes(target.startsWith('darwin') ? '-Wl,-rpath,@loader_path' : '-Wl,-rpath,$ORIGIN'));
    assert.ok(plan.integrationRequired.includes('native-sandbox-loader'));
    assert.ok(plan.integrationRequired.includes('trusted-sdk-bootstrap'));
    assert.equal(plan.commands.some(command => /(?:^|\/)node$/.test(command.program)), false);
  }
});

test('plan CLI performs no downloads, tool invocations or scratch writes', async t => {
  const root = await temporary(t);
  const scratch = join(root, 'never-created');
  const result = spawnSync(process.execPath, [join(repository, 'scripts/build-app-runtime.mjs'), 'plan', '--target', 'linux-arm64', '--scratch', scratch,
    '--download-source', '--cc', '/missing/gcc', '--cxx', '/missing/g++', '--python', '/missing/python3'], { encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  const plan = JSON.parse(result.stdout);
  assert.equal(plan.target, 'linux-arm64');
  assert.equal(plan.downloadSource, true);
  assert.deepEqual(await readdir(root), []);
});

test('scratch rejects repositories, parent directories, symlinks and existing data', async t => {
  const root = await temporary(t);
  const base = { mode: 'plan', target: 'linux-x64', jobs: 1 };
  for (const scratch of [repository, join(repository, 'target/tmp'), dirname(repository), '/', '/bad/path with spaces', '/bad/../path', '/bad/$(shell)']) {
    assert.throws(() => createPlan({ ...base, scratch }, lock));
  }
  for (const jobs of [0, 3, NaN, 1.5]) assert.throws(() => createPlan({ ...base, scratch: '/safe/build', jobs }, lock));
  assert.throws(() => createPlan({ ...base, scratch: '/safe/build', target: 'win32-x64' }, lock));
  const otherRepo = join(root, 'other-repo');
  await mkdir(otherRepo); await writeFile(join(otherRepo, '.git'), 'gitdir: /somewhere');
  await assert.rejects(validateScratch(join(otherRepo, 'scratch')), /any Git repository/);
  const occupied = join(root, 'occupied');
  await mkdir(occupied, { mode: 0o700 }); await writeFile(join(occupied, 'keep.txt'), 'retain me');
  await assert.rejects(validateScratch(occupied), /empty/);
  const linked = join(root, 'linked'); await symlink(occupied, linked);
  await assert.rejects(validateScratch(join(linked, 'scratch')), /symlinks/);
  const publicScratch = join(root, 'public'); await mkdir(publicScratch); await chmod(publicScratch, 0o777);
  await assert.rejects(validateScratch(publicScratch), /owned by the current user and private/);
  assert.deepEqual(await readdir(publicScratch), []);
  assert.equal(await readFile(join(occupied, 'keep.txt'), 'utf8'), 'retain me');
});

test('failed command terminates its detached descendants before returning', async t => {
  const root = await temporary(t);
  const heartbeat = join(root, 'heartbeat');
  const pidFile = join(root, 'pid');
  const descendant = `const fs=require('node:fs');fs.writeFileSync(process.argv[1],String(process.pid));let count=0;setInterval(()=>fs.writeFileSync(process.argv[2],String(++count)),10);process.stdout.write('ready');`;
  const leader = `const {spawn}=require('node:child_process');const child=spawn(process.execPath,['-e',process.argv[1],process.argv[2],process.argv[3]],{stdio:['ignore','pipe','ignore']});child.stdout.once('data',()=>process.exit(7));`;
  let descendantPid;
  try {
    await assert.rejects(runCommand({ program: process.execPath, args: ['-e', leader, descendant, pidFile, heartbeat], cwd: root }, process.env,
      { scratch: root, resourceBounds: lock.build }), /build command failed/);
    descendantPid = Number(await readFile(pidFile, 'utf8'));
    await new Promise(resolve => setTimeout(resolve, 40));
    const before = await readFile(heartbeat, 'utf8').catch(error => error.code === 'ENOENT' ? null : Promise.reject(error));
    await new Promise(resolve => setTimeout(resolve, 60));
    const after = await readFile(heartbeat, 'utf8').catch(error => error.code === 'ENOENT' ? null : Promise.reject(error));
    assert.equal(after, before, 'descendant continued writing after the failed leader exited');
  } finally {
    if (descendantPid) { try { process.kill(descendantPid, 'SIGKILL'); } catch {} }
  }
});

test('metadata tools have a bounded timeout', () => {
  assert.throws(() => checkedOutput(process.execPath, ['-e', 'setInterval(()=>{},1000)'], process.env, 50), /tool failed/);
  assert.throws(() => checkedOutput(process.execPath, ['--version'], process.env, 0), /timeout/);
});

test('source must match pinned bytes before any extraction', async t => {
  const root = await temporary(t);
  const path = join(root, 'fixture.tar.xz');
  const bytes = Buffer.from('small test fixture, not a Node source archive');
  await writeFile(path, bytes);
  const expected = { sha256: createHash('sha256').update(bytes).digest('hex') };
  await verifySource(path, expected, 1024);
  await assert.rejects(verifySource(path, lock.node, 1024), /checksum/);
  await assert.rejects(verifySource(path, expected, 1), /bounded regular file/);
  const link = join(root, 'symlink.tar.xz'); await symlink(path, link);
  await assert.rejects(verifySource(link, expected, 1024), /bounded regular file/);
  await assert.rejects(verifySource(root, expected, 1024), /bounded regular file/);
});

test('compiler and resource checks require the pinned dedicated builder', () => {
  const linux = { cc: '12.2.0', cxx: '12.2.0', python: 'Python 3.11.2', make: 'GNU Make 4.3\nBuilt for x86_64-pc-linux-gnu' };
  checkToolchainVersions(lock.toolchains['linux-gcc12'], linux, 'linux');
  for (const field of ['cc', 'cxx', 'python', 'make']) {
    assert.throws(() => checkToolchainVersions(lock.toolchains['linux-gcc12'], { ...linux, [field]: 'unexpected' }, 'linux'));
  }
  const mac = { cc: 'Apple clang version 17.0.0 (clang-build)', cxx: 'Apple clang version 17.0.0 (clang-build)', python: 'Python 3.11.9', make: 'GNU Make 3.81\nApple build', xcode: 'Xcode 16.4\nBuild version 16F6' };
  checkToolchainVersions(lock.toolchains['macos-xcode16'], mac, 'darwin');
  assert.throws(() => checkToolchainVersions(lock.toolchains['macos-xcode16'], { ...mac, cc: 'Apple clang version 17.0.01 (unwanted patch)' }, 'darwin'));
  assert.throws(() => checkToolchainVersions(lock.toolchains['macos-xcode16'], { ...mac, xcode: 'Xcode 26.5\nBuild version X' }, 'darwin'));
  const safe = { totalMemoryBytes: 32 * 1024 ** 3, freeMemoryBytes: 16 * 1024 ** 3, freeDiskBytes: 64 * 1024 ** 3 };
  checkBuildResources(lock.build, safe);
  for (const field of Object.keys(safe)) {
    for (const invalid of [0, NaN, undefined, Infinity]) assert.throws(() => checkBuildResources(lock.build, { ...safe, [field]: invalid }), /no compilation started/);
  }
});

test('dependency inspection rejects Homebrew, external crypto and escaping rpaths', () => {
  const mac = lock.targets['darwin-arm64'];
  checkDependencies('darwin', `runtime:\n\t@rpath/${mac.nodeLibrary} (compatibility version 0)\n\t/usr/lib/libSystem.B.dylib (compatibility version 0)`, mac);
  assert.throws(() => checkDependencies('darwin', 'runtime:\n\t/opt/homebrew/lib/libssl.dylib (compatibility version 0)', mac), /unbundled/);
  const linux = lock.targets['linux-x64'];
  checkDependencies('linux', ` 0x1 (NEEDED) Shared library: [${linux.nodeLibrary}]\n 0x1 (NEEDED) Shared library: [libc.so.6]\n 0x1 (RUNPATH) Library runpath: [$ORIGIN]`, linux);
  assert.throws(() => checkDependencies('linux', ' 0x1 (NEEDED) Shared library: [libssl.so.3]', linux), /unbundled/);
  assert.throws(() => checkDependencies('linux', ' 0x1 (RPATH) Library rpath: [/tmp/untrusted]', linux), /RPATH/);
});
