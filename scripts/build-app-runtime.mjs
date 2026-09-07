#!/usr/bin/env node

import { createHash } from 'node:crypto';
import { spawn, spawnSync } from 'node:child_process';
import { constants, createReadStream, createWriteStream } from 'node:fs';
import { copyFile, lstat, mkdir, open, readFile, readdir, realpath, rm, statfs, writeFile } from 'node:fs/promises';
import { totalmem } from 'node:os';
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from 'node:path';
import { Readable, Transform } from 'node:stream';
import { pipeline } from 'node:stream/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { availableBuildMemory, checkArtifactBudget, validateCiProfile } from './app-runtime-ci-resources.mjs';
import { NATIVE_BUILD_INPUTS } from './app-runtime-ci-receipt.mjs';

const REPOSITORY = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const LOCK_PATH = join(REPOSITORY, 'apps/app-worker/runtime.lock.json');
const SOURCE_PATHS = NATIVE_BUILD_INPUTS;
const TARGETS = ['darwin-arm64', 'darwin-x64', 'linux-x64', 'linux-arm64'];
const USAGE = 'build-app-runtime.mjs plan|build --target <target> --scratch <new-empty-directory-outside-repositories> [--jobs 1|2] [--resource-profile default|github-linux] [--source-archive <cached-tar.xz>|--download-source] [--cc <path> --cxx <path> --python <path>]';

export function parseOptions(argv) {
  const mode = argv[0];
  if (!['plan', 'build'].includes(mode)) throw new Error(USAGE);
  const options = { mode, jobs: 1, downloadSource: false };
  const names = { '--target': 'target', '--scratch': 'scratch', '--jobs': 'jobs', '--resource-profile': 'resourceProfile', '--source-archive': 'sourceArchive', '--cc': 'cc', '--cxx': 'cxx', '--python': 'python' };
  const seen = new Set();
  for (let index = 1; index < argv.length; index += 1) {
    const flag = argv[index];
    if (seen.has(flag)) throw new Error(`duplicate option: ${flag}`);
    seen.add(flag);
    if (flag === '--download-source') { options.downloadSource = true; continue; }
    const name = names[flag];
    if (!name || !argv[index + 1] || argv[index + 1].startsWith('--')) throw new Error(USAGE);
    options[name] = argv[++index];
  }
  options.jobs = Number(options.jobs);
  if (!options.target || !options.scratch || options.sourceArchive && options.downloadSource) throw new Error(USAGE);
  if (mode === 'build' && !options.sourceArchive && !options.downloadSource) {
    throw new Error('build requires a cached --source-archive or explicit --download-source');
  }
  return options;
}

export function validateLock(lock) {
  if (lock.schema !== 'chariox.app-runtime-build.v1' || lock.workerAbi !== 1
      || !/^\d+\.\d+\.\d+$/.test(lock.runtimeVersion)
      || !/^\d+\.\d+\.\d+$/.test(lock.node?.version)
      || !/^[a-f0-9]{64}$/.test(lock.node?.sha256)
      || lock.node.archive !== `node-v${lock.node.version}.tar.xz`
      || lock.node.url !== `https://nodejs.org/download/release/v${lock.node.version}/${lock.node.archive}`
      || JSON.stringify(Object.keys(lock.targets).sort()) !== JSON.stringify([...TARGETS].sort())
      || lock.loading.mode !== 'sandbox-before-dlopen' || lock.loading.launcherLinksNode !== false) {
    throw new Error('invalid pinned App runtime build contract');
  }
  const expectedFlags = ['--shared', '--without-inspector', '--without-node-options', '--disable-single-executable-application'];
  if (JSON.stringify(lock.node.configure) !== JSON.stringify(expectedFlags)) throw new Error('unsupported Node build options');
  for (const [name, target] of Object.entries(lock.targets)) {
    if (`${target.platform}-${target.arch}` !== name || !lock.toolchains[target.toolchain]
        || target.nodeLibrary !== (target.platform === 'darwin' ? `libnode.${lock.node.moduleAbi}.dylib` : `libnode.so.${lock.node.moduleAbi}`)
        || target.runtimeLibrary !== (target.platform === 'darwin' ? 'libchariox-app-runtime.dylib' : 'libchariox-app-runtime.so')) {
      throw new Error(`invalid runtime target: ${name}`);
    }
  }
  const boundNames = ['sourceDateEpoch', 'maxJobs', 'minTotalMemoryBytes', 'minFreeMemoryBytes', 'minFreeDiskBytes', 'maxSourceArchiveBytes', 'minimumRemainingMemoryBytes', 'minimumRemainingDiskBytes'];
  if (JSON.stringify(Object.keys(lock.build).sort()) !== JSON.stringify(boundNames.sort())) throw new Error('missing or unknown build resource bound');
  for (const [name, value] of Object.entries(lock.build)) {
    if (!Number.isSafeInteger(value) || value < 0) throw new Error(`invalid build bound: ${name}`);
  }
  if (lock.build.maxJobs < 1 || lock.build.maxJobs > 2 || lock.build.minFreeDiskBytes < 32 * 1024 ** 3
      || lock.build.minTotalMemoryBytes < 16 * 1024 ** 3 || lock.build.minFreeMemoryBytes < 8 * 1024 ** 3
      || lock.build.maxSourceArchiveBytes < 1 || lock.build.maxSourceArchiveBytes > 80 * 1024 ** 2
      || lock.build.minimumRemainingMemoryBytes < 2 * 1024 ** 3 || lock.build.minimumRemainingDiskBytes < 4 * 1024 ** 3) {
    throw new Error('build resource bounds cannot be weakened');
  }
  validateCiProfile(lock.dedicatedCi);
  if (!lock.dedicatedCi.builderImage.startsWith(`node:${lock.node.version}-bookworm@`)) throw new Error('CI builder must use the pinned Node release');
  return lock;
}

function absoluteBuildPath(path, label) {
  if (typeof path !== 'string' || !isAbsolute(path) || !/^\/[A-Za-z0-9_./+-]+$/.test(path) || resolve(path) !== path || path === '/') {
    throw new Error(`${label} must be a normalized absolute path without spaces or shell metacharacters`);
  }
  return path;
}

function inside(parent, child) {
  const suffix = relative(parent, child);
  return suffix === '' || suffix !== '..' && !suffix.startsWith(`..${sep}`) && !isAbsolute(suffix);
}

export function createPlan(options, lock, repository = REPOSITORY) {
  validateLock(lock);
  const target = lock.targets[options.target];
  if (!target) throw new Error('unsupported runtime target');
  const resourceProfile = options.resourceProfile ?? 'default';
  if (!['default', 'github-linux'].includes(resourceProfile) || resourceProfile === 'github-linux' && target.platform !== 'linux') throw new Error('unsupported resource profile for target');
  const resourceBounds = resourceProfile === 'github-linux' ? { ...lock.build, ...lock.dedicatedCi.resourceBounds } : lock.build;
  if (!Number.isInteger(options.jobs) || options.jobs < 1 || options.jobs > resourceBounds.maxJobs) throw new Error('jobs exceed resource profile limit');
  const scratch = absoluteBuildPath(options.scratch, 'scratch');
  if (inside(repository, scratch) || inside(scratch, repository)) throw new Error('scratch must be separate from the source repository');
  const source = join(scratch, 'source');
  const output = join(scratch, 'artifacts');
  const cc = absoluteBuildPath(options.cc ?? `/usr/bin/${target.platform === 'darwin' ? 'clang' : 'gcc'}`, 'cc');
  const cxx = absoluteBuildPath(options.cxx ?? `/usr/bin/${target.platform === 'darwin' ? 'clang++' : 'g++'}`, 'cxx');
  const python = absoluteBuildPath(options.python ?? '/usr/bin/python3', 'python');
  if (options.sourceArchive) absoluteBuildPath(options.sourceArchive, 'source archive');
  const nativeLibrary = join(source, 'out/Release', target.nodeLibrary);
  const include = [join(source, 'src'), join(source, 'deps/v8/include'), join(source, 'deps/uv/include')].flatMap(path => ['-I', path]);
  const commonCompile = ['-std=c++20', '-O2', '-fPIC', '-fvisibility=hidden', '-DNODE_SHARED_MODE', ...include,
    `-ffile-prefix-map=${scratch}=/chariox-build`, `-fdebug-prefix-map=${scratch}=/chariox-build`,
    join(source, 'chariox/node_runtime.cc'), nativeLibrary];
  const linker = target.platform === 'darwin'
    ? ['-dynamiclib', '-arch', target.arch === 'x64' ? 'x86_64' : 'arm64', `-mmacosx-version-min=${target.deploymentTarget}`,
      '-Wl,-rpath,@loader_path', `-Wl,-install_name,@rpath/${target.runtimeLibrary}`]
    : ['-shared', '-pthread', '-Wl,-z,relro,-z,now', '-Wl,-rpath,$ORIGIN', `-Wl,-soname,${target.runtimeLibrary}`];
  return {
    schema: 'chariox.app-runtime-build-plan.v1', target: options.target, targetContract: target,
    source, scratch, output, sourceArchive: options.sourceArchive ?? null,
    downloadSource: options.downloadSource ?? false,
    tools: { cc, cxx, python, make: '/usr/bin/make', tar: '/usr/bin/tar' },
    toolchain: lock.toolchains[target.toolchain], requiresNativeHost: true,
    sourceProvenance: lock.node, resourceBounds, resourceProfile,
    ciProfile: resourceProfile === 'github-linux' ? lock.dedicatedCi : null,
    commands: [
      { program: python, args: ['./configure', ...lock.node.configure, `--dest-cpu=${target.arch}`], cwd: source },
      { program: '/usr/bin/make', args: ['-C', 'out', 'BUILDTYPE=Release', `-j${options.jobs}`, 'libnode'], cwd: source },
      { program: cxx, args: [...commonCompile, ...linker, '-o', join(output, target.runtimeLibrary)], cwd: source },
    ],
    artifactSigning: 'unsigned', integrationRequired: lock.loading.requiredIntegration,
  };
}

// This path check is also used by plan mode; it never creates a directory.
export async function validateScratch(path) {
  for (let current = path; ; current = dirname(current)) {
    const metadata = await lstat(current).catch(error => error.code === 'ENOENT' ? null : Promise.reject(error));
    if (metadata?.isSymbolicLink()) throw new Error('scratch ancestors must not be symlinks');
    if (metadata && !metadata.isDirectory()) throw new Error('scratch ancestor must be a directory');
    const git = await lstat(join(current, '.git')).catch(error => error.code === 'ENOENT' ? null : Promise.reject(error));
    if (git) throw new Error('scratch cannot be inside any Git repository');
    if (current === dirname(current)) break;
  }
  const metadata = await lstat(path).catch(error => error.code === 'ENOENT' ? null : Promise.reject(error));
  if (metadata && (metadata.uid !== process.getuid() || (metadata.mode & 0o077) !== 0)) throw new Error('scratch must be owned by the current user and private (0700)');
  if (metadata && (await readdir(path)).length !== 0) throw new Error('scratch must be empty');
}

export function checkedOutput(program, args, environment, timeout = 30_000) {
  if (!Number.isInteger(timeout) || timeout < 1 || timeout > 30_000) throw new Error('invalid metadata command timeout');
  const result = spawnSync(program, args, { encoding: 'utf8', maxBuffer: 256 * 1024, env: environment, timeout, killSignal: 'SIGKILL' });
  if (result.error || result.status !== 0) throw new Error(`tool failed: ${basename(program)}`);
  return result.stdout.trim();
}

export function checkToolchainVersions(toolchain, observed, platform) {
  for (const name of ['cc', 'cxx']) {
    const version = platform === 'darwin' ? /^Apple clang version (\S+)/.exec(observed[name])?.[1] : observed[name];
    if (version !== toolchain.compilerVersion) throw new Error(`${name} does not match pinned compiler version`);
  }
  if (observed.python !== `Python ${toolchain.pythonVersion}` || !observed.make.startsWith(`GNU Make ${toolchain.makeVersion}\n`)) throw new Error('Python or Make does not match pinned toolchain');
  if (platform === 'darwin' && !observed.xcode.startsWith(`Xcode ${toolchain.xcodeVersion}\n`)) throw new Error('Xcode does not match pinned toolchain');
}

export function checkBuildResources(bounds, actual) {
  if (!['totalMemoryBytes', 'freeMemoryBytes', 'freeDiskBytes'].every(key => Number.isSafeInteger(actual[key]) && actual[key] >= 0)
      || actual.totalMemoryBytes < bounds.minTotalMemoryBytes || actual.freeMemoryBytes < bounds.minFreeMemoryBytes || actual.freeDiskBytes < bounds.minFreeDiskBytes) {
    throw new Error('insufficient dedicated build resources; no compilation started');
  }
}

async function freeDisk(path) {
  const disk = await statfs(path, { bigint: true });
  return Number(disk.bavail * disk.bsize);
}

async function digestFile(path) {
  const hash = createHash('sha256');
  for await (const bytes of createReadStream(path)) hash.update(bytes);
  return hash.digest('hex');
}

export async function verifySource(path, source, maxBytes) {
  const metadata = await lstat(path);
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > maxBytes) throw new Error('source archive must be a bounded regular file');
  if (await digestFile(path) !== source.sha256) throw new Error('Node source checksum does not match runtime.lock.json');
}

async function downloadSource(url, destination, maxBytes) {
  const response = await fetch(url, { redirect: 'error', signal: AbortSignal.timeout(120_000) });
  if (!response.ok || !response.body) throw new Error('Node source download failed');
  const length = Number(response.headers.get('content-length'));
  if (Number.isFinite(length) && length > maxBytes) {
    await response.body.cancel();
    throw new Error('Node source download exceeds size bound');
  }
  let received = 0;
  const bound = new Transform({ transform(chunk, _, callback) {
    received += chunk.length;
    callback(received > maxBytes ? new Error('Node source download exceeds size bound') : null, chunk);
  } });
  await pipeline(Readable.fromWeb(response.body), bound, createWriteStream(destination, { flags: 'r+', mode: 0o600 }));
}

export async function runCommand(command, environment, plan) {
  const child = spawn(command.program, command.args, { cwd: command.cwd, env: environment, stdio: 'inherit', detached: true });
  let resourceFailure;
  let checking = false;
  let forceKill;
  let finished = false;
  let failed = true;
  function stop(reason) {
    if (finished || resourceFailure) return;
    resourceFailure = reason;
    if (child.pid) {
      try { process.kill(-child.pid, 'SIGTERM'); } catch {}
      forceKill = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }, 3000);
    }
  }
  const interrupted = () => stop(new Error('build interrupted'));
  process.once('SIGINT', interrupted);
  process.once('SIGTERM', interrupted);
  const interval = setInterval(async () => {
    if (checking || resourceFailure) return;
    checking = true;
    try {
      if (await availableBuildMemory(plan) < plan.resourceBounds.minimumRemainingMemoryBytes || await freeDisk(plan.scratch) < plan.resourceBounds.minimumRemainingDiskBytes) {
        stop(new Error('build stopped to preserve host memory/disk reserves'));
      }
    } catch (error) { stop(error); }
    finally { checking = false; }
  }, 2000);
  try {
    await new Promise((accept, reject) => {
      child.once('error', reject);
      child.once('exit', code => code === 0 ? accept() : reject(resourceFailure ?? new Error(`build command failed: ${basename(command.program)}`)));
    });
    if (resourceFailure) throw resourceFailure;
    failed = false;
  } finally {
    finished = true;
    // make/compiler failure or an externally killed leader can leave detached
    // descendants alive. Stop the whole owned group before deleting scratch.
    if (failed && child.pid) { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
    clearInterval(interval);
    clearTimeout(forceKill);
    process.removeListener('SIGINT', interrupted);
    process.removeListener('SIGTERM', interrupted);
  }
}

export function checkDependencies(platform, text, target) {
  if (platform === 'darwin') {
    const allowed = new Set([`@rpath/${target.nodeLibrary}`, `@rpath/${target.runtimeLibrary}`]);
    for (const line of text.split('\n').slice(1).filter(Boolean)) {
      const dependency = line.trim().split(' ')[0];
      if (!dependency.startsWith('/usr/lib/') && !dependency.startsWith('/System/Library/Frameworks/') && !allowed.has(dependency)) throw new Error(`unbundled runtime dependency: ${dependency}`);
    }
  } else {
    const allowed = new Set([target.nodeLibrary, 'libstdc++.so.6', 'libgcc_s.so.1', 'libm.so.6', 'libc.so.6', 'libpthread.so.0', 'libdl.so.2', 'librt.so.1', 'libatomic.so.1']);
    for (const match of text.matchAll(/\(NEEDED\).*\[([^\]]+)\]/g)) if (!allowed.has(match[1])) throw new Error(`unbundled runtime dependency: ${match[1]}`);
    for (const match of text.matchAll(/\((?:RUNPATH|RPATH)\).*\[([^\]]*)\]/g)) if (match[1] !== '$ORIGIN') throw new Error('runtime RPATH must remain inside its artifact directory');
  }
}

export async function buildRuntime(plan, lock) {
  if (`${process.platform}-${process.arch}` !== plan.target) throw new Error('runtime builds require the matching native OS and architecture');
  await validateScratch(plan.scratch);
  let ancestor = dirname(plan.scratch);
  while (!await lstat(ancestor).catch(() => null)) ancestor = dirname(ancestor);
  checkBuildResources(plan.resourceBounds, { totalMemoryBytes: totalmem(), freeMemoryBytes: await availableBuildMemory(plan), freeDiskBytes: await freeDisk(ancestor) });
  const environment = {
    PATH: [...new Set([...Object.values(plan.tools).map(dirname), '/usr/bin', '/bin'])].join(':'),
    HOME: process.env.HOME, LANG: 'C', LC_ALL: 'C', TZ: 'UTC', SOURCE_DATE_EPOCH: String(lock.build.sourceDateEpoch),
    CC: plan.tools.cc, CXX: plan.tools.cxx, PYTHON: plan.tools.python,
    CFLAGS: `-ffile-prefix-map=${plan.scratch}=/chariox-build -fdebug-prefix-map=${plan.scratch}=/chariox-build`,
    CXXFLAGS: `-ffile-prefix-map=${plan.scratch}=/chariox-build -fdebug-prefix-map=${plan.scratch}=/chariox-build`,
    CCACHE_DISABLE: '1',
    ...(plan.targetContract.deploymentTarget ? { MACOSX_DEPLOYMENT_TARGET: plan.targetContract.deploymentTarget } : {}),
  };
  const observed = {
    cc: checkedOutput(plan.tools.cc, [process.platform === 'darwin' ? '--version' : '-dumpfullversion'], environment),
    cxx: checkedOutput(plan.tools.cxx, [process.platform === 'darwin' ? '--version' : '-dumpfullversion'], environment),
    python: checkedOutput(plan.tools.python, ['--version'], environment),
    make: checkedOutput(plan.tools.make, ['--version'], environment),
    ...(process.platform === 'darwin' ? { xcode: checkedOutput('/usr/bin/xcodebuild', ['-version'], environment) } : {}),
  };
  checkToolchainVersions(plan.toolchain, observed, process.platform);
  const git = args => checkedOutput('/usr/bin/git', ['-C', REPOSITORY, ...args], environment);
  if (git(['status', '--porcelain', '--', ...SOURCE_PATHS])) throw new Error('commit runtime build inputs before creating a provenance artifact');
  const sourceCommit = git(['rev-parse', 'HEAD']);
  const sourceTree = git(['rev-parse', 'HEAD^{tree}']);
  const buildInputs = await Promise.all(SOURCE_PATHS.map(async path => ({ path, sha256: await digestFile(join(REPOSITORY, path)) })));
  const toolInputs = await Promise.all(Object.entries(plan.tools).map(async ([name, path]) => ({ name, path: await realpath(path), sha256: await digestFile(path) })));

  await mkdir(plan.scratch, { recursive: true, mode: 0o700 });
  await validateScratch(plan.scratch);
  const marker = join(plan.scratch, '.app-runtime-build');
  const markerHandle = await open(marker, 'wx', 0o600);
  await markerHandle.close();
  const archive = join(plan.scratch, lock.node.archive);
  let succeeded = false;
  let archiveOwned = false;
  let sourceOwned = false;
  let outputOwned = false;
  try {
    const archiveHandle = await open(archive, 'wx', 0o600);
    archiveOwned = true;
    await archiveHandle.close();
    if (plan.sourceArchive) {
      const metadata = await lstat(plan.sourceArchive);
      if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > lock.build.maxSourceArchiveBytes) throw new Error('cached source must be a bounded regular file');
      await copyFile(plan.sourceArchive, archive);
    } else if (plan.downloadSource) {
      await downloadSource(lock.node.url, archive, lock.build.maxSourceArchiveBytes);
    } else { throw new Error('explicit source selection is required'); }
    await verifySource(archive, lock.node, lock.build.maxSourceArchiveBytes);
    await mkdir(plan.source, { mode: 0o700 });
    sourceOwned = true;
    await mkdir(plan.output, { mode: 0o700 });
    outputOwned = true;
    await runCommand({ program: plan.tools.tar, args: ['-xJf', archive, '--strip-components=1', '-C', plan.source], cwd: plan.scratch }, environment, plan);
    await mkdir(join(plan.source, 'chariox'), { mode: 0o700 });
    for (const file of ['node_runtime.cc', 'runtime.h']) await copyFile(join(REPOSITORY, 'apps/app-worker/src', file), join(plan.source, 'chariox', file), constants.COPYFILE_EXCL);
    for (const command of plan.commands) await runCommand(command, environment, plan);
    await copyFile(join(plan.source, 'out/Release', plan.targetContract.nodeLibrary), join(plan.output, plan.targetContract.nodeLibrary), constants.COPYFILE_EXCL);
    await copyFile(join(plan.source, 'LICENSE'), join(plan.output, 'NODE-LICENSE'), constants.COPYFILE_EXCL);
    const artifactFiles = [plan.targetContract.runtimeLibrary, plan.targetContract.nodeLibrary, 'NODE-LICENSE'];
    const dependencies = {};
    for (const name of artifactFiles.slice(0, 2)) {
      dependencies[name] = checkedOutput(process.platform === 'darwin' ? '/usr/bin/otool' : '/usr/bin/readelf', process.platform === 'darwin' ? ['-L', join(plan.output, name)] : ['-d', join(plan.output, name)], environment);
      checkDependencies(process.platform, dependencies[name], plan.targetContract);
    }
    const files = await Promise.all(artifactFiles.map(async path => ({ path, size: (await lstat(join(plan.output, path))).size, sha256: await digestFile(join(plan.output, path)) })));
    const manifest = {
      schema: 'chariox.app-runtime-artifact.v1', runtimeVersion: lock.runtimeVersion, workerAbi: lock.workerAbi,
      target: plan.target, nodeVersion: lock.node.version, nodeModuleAbi: lock.node.moduleAbi,
      source: { url: lock.node.url, sha256: lock.node.sha256 }, sourceCommit, sourceTree, buildInputs, toolInputs,
      toolchain: observed, commands: plan.commands, dependencies,
      files, resourceProfile: plan.resourceProfile, resourceBounds: plan.resourceBounds, builderProfile: plan.ciProfile,
      signing: { status: 'unsigned', notarization: 'not-performed' },
      validation: { nativeBuild: 'completed', runtimeExecution: 'not-performed', containment: 'not-performed', reproducibility: 'not-compared' },
      loading: lock.loading,
    };
    const manifestText = `${JSON.stringify(manifest, null, 2)}\n`;
    checkArtifactBudget(plan.ciProfile, [...files.map(file => file.size), Buffer.byteLength(manifestText)]);
    await writeFile(join(plan.output, 'artifact-manifest.json'), manifestText, { flag: 'wx', mode: 0o600 });
    succeeded = true;
    return manifest;
  } finally {
    if (sourceOwned) await rm(plan.source, { recursive: true, force: true });
    if (archiveOwned) await rm(archive, { force: true });
    if (!succeeded && outputOwned) await rm(plan.output, { recursive: true, force: true });
    await rm(marker, { force: true });
  }
}

async function main() {
  const options = parseOptions(process.argv.slice(2));
  const lock = JSON.parse(await readFile(LOCK_PATH, 'utf8'));
  const plan = createPlan(options, lock);
  await validateScratch(plan.scratch);
  const result = options.mode === 'plan' ? plan : await buildRuntime(plan, lock);
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch(error => { process.stderr.write(`build-app-runtime: ${error.message}\n`); process.exitCode = 1; });
}
