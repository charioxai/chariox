import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { checkMacBuilder, checkMacTools, checkMachO, createMacBuildSession, finalMacBuildCheck } from './app-runtime-macos-build.mjs';
import { validateMacProfile } from './app-runtime-ci-resources.mjs';
import { createPlan, checkBuildResources } from './build-app-runtime.mjs';

const profile = JSON.parse(await readFile(new URL('../apps/app-worker/macos-build-profile.json', import.meta.url), 'utf8'));
const lock = JSON.parse(await readFile(new URL('../apps/app-worker/runtime.lock.json', import.meta.url), 'utf8'));
const environment = { GITHUB_ACTIONS: 'true', RUNNER_ENVIRONMENT: 'github-hosted', GITHUB_REPOSITORY: 'charioxai/chariox',
  GITHUB_EVENT_NAME: 'workflow_dispatch', CHARIOX_NATIVE_BUILD_CONFIRMED: 'true', ImageOS: 'macos15', ImageVersion: '20260829.0321.1',
  NATIVE_ADMISSION_EVENT: 'workflow_dispatch',
  GITHUB_RUN_ID: '123', GITHUB_RUN_ATTEMPT: '1' };

test('observed Mac profile needs explicit hosted admission and does not weaken laptop defaults', () => {
  validateMacProfile(profile);
  checkMacBuilder(profile, environment, 'darwin', 'arm64');
  for (const patch of [{ GITHUB_EVENT_NAME: 'pull_request' }, { CHARIOX_NATIVE_BUILD_CONFIRMED: 'false' },
    { RUNNER_ENVIRONMENT: 'self-hosted' }, { ImageOS: 'ubuntu24' }, { GITHUB_REPOSITORY: 'fork/repo' }])
    assert.throws(() => checkMacBuilder(profile, { ...environment, ...patch }, 'darwin', 'arm64'));
  assert.throws(() => checkMacBuilder(profile, environment, 'linux', 'arm64'));
  assert.throws(() => checkMacBuilder(profile, environment, 'darwin', 'x64'));
  for (const patch of [{ maxDurationMs: 301 * 60_000 }, { observedGroupRssBytes: 5 * 1024 ** 3 }, { additionalSwapBytes: 1024 ** 3 },
    { resourceBounds: { ...profile.resourceBounds, maxJobs: 2 } }]) assert.throws(() => validateMacProfile({ ...profile, ...patch }));
  const base = { mode: 'plan', target: 'darwin-arm64', scratch: '/safe/build', jobs: 1 };
  const normal = createPlan(base, lock); const dedicated = createPlan({ ...base, resourceProfile: 'github-macos' }, lock);
  assert.equal(normal.resourceBounds.minTotalMemoryBytes, 16 * 1024 ** 3);
  assert.equal(normal.resourceBounds.minFreeMemoryBytes, 8 * 1024 ** 3);
  assert.equal(normal.resourceBounds.minFreeDiskBytes, 32 * 1024 ** 3);
  assert.equal(dedicated.resourceBounds.maxJobs, 1);
  checkBuildResources(dedicated.resourceBounds, { totalMemoryBytes: 7516192768, freeMemoryBytes: 3350708224, freeDiskBytes: 45340545024 });
  assert.throws(() => checkBuildResources(dedicated.resourceBounds, { totalMemoryBytes: 7516192768, freeMemoryBytes: 241074176, freeDiskBytes: 45340545024 }));
  assert.throws(() => createPlan({ ...base, resourceProfile: 'github-macos', jobs: 2 }, lock));
  assert.throws(() => createPlan({ ...base, resourceProfile: 'github-macos', target: 'darwin-x64' }, lock));
});

test('Mac toolchain pins exact SDK, Xcode and compiler build; Mach-O identity is native', () => {
  const toolRoot = `${profile.developerDirectory}/Toolchains/XcodeDefault.xctoolchain/usr/bin/`;
  const tools = { cc: `${toolRoot}clang`, cxx: `${toolRoot}clang++` };
  const observed = { xcode: 'Xcode 16.4\nBuild version 16F6', sdkVersion: '15.5', developerDirectory: profile.developerDirectory,
    sdkPath: `${profile.developerDirectory}/Platforms/MacOSX.platform/Developer/SDKs/MacOSX15.5.sdk`,
    cc: 'Apple clang version 17.0.0 (clang-1700.0.13.5)\nTarget: arm64-apple-darwin',
    cxx: 'Apple clang version 17.0.0 (clang-1700.0.13.5)\nTarget: arm64-apple-darwin' };
  checkMacTools(profile, observed, tools);
  for (const patch of [{ sdkVersion: '15.6' }, { xcode: 'Xcode 16.4\nBuild version OTHER' }, { sdkPath: '/untrusted/sdk' },
    { cc: 'Apple clang version 17.0.0 (other)\n' }]) assert.throws(() => checkMacTools(profile, { ...observed, ...patch }, tools));
  assert.throws(() => checkMacTools(profile, observed, { ...tools, cc: '/usr/local/bin/clang' }));
  const target = lock.targets['darwin-arm64'];
  checkMachO(target, 'arm64', '@rpath/libnode.137.dylib', target.nodeLibrary);
  for (const [arch, id] of [['x86_64', '@rpath/libnode.137.dylib'], ['arm64 x86_64', '@rpath/libnode.137.dylib'], ['arm64', '/tmp/libnode.137.dylib'], ['arm64', '@rpath/libchariox-app-runtime.dylib']])
    assert.throws(() => checkMachO(target, arch, id, target.nodeLibrary));
});

test('resource evidence aggregates bounded extrema across all commands under one deadline', () => {
  const session = createMacBuildSession(profile, 0, 100);
  session.observe({ groupRssBytes: 20, groupProcesses: 3, swapBytes: 10, availableMemoryBytes: 50, freeDiskBytes: 100 });
  session.observe({ groupRssBytes: 10, groupProcesses: 4, swapBytes: 5, availableMemoryBytes: 60, freeDiskBytes: 80 });
  assert.equal(session.deadline, 100 + 300 * 60_000);
  assert.equal(session.evidence.samples, 2); assert.equal(session.evidence.peakGroupRssBytes, 20);
  assert.equal(session.evidence.peakGroupProcesses, 4); assert.equal(session.evidence.peakSwapBytes, 10);
  assert.equal(session.evidence.minimumAvailableMemoryBytes, 50); assert.equal(session.evidence.minimumFreeDiskBytes, 80);
  assert.equal(session.evidence.completed, false);
});

test('heavyweight jobs require manual confirmation while PRs retain cheap verification', async () => {
  const linux = await readFile(new URL('../.github/workflows/app-runtime-native.yml', import.meta.url), 'utf8');
  const mac = await readFile(new URL('../.github/workflows/app-runtime-macos-native.yml', import.meta.url), 'utf8');
  for (const text of [linux, mac]) {
    assert.match(text, /build:\n    if: needs\.admission\.outputs\.confirmed == 'true'/);
    assert.match(text, /github\.event_name == 'workflow_dispatch' && inputs\.confirm_build == true/);
    assert.match(text, /github\.event\.action == 'labeled'/);
    assert.match(text, /github\.event\.pull_request\.head\.repo\.full_name == github\.repository/);
    assert.match(text, /ref: \$\{\{ needs\.admission\.outputs\.source_sha \}\}/);
    assert.match(text, /cancel-in-progress: false/); assert.match(text, /default: false/);
  }
  assert.match(linux, /pull_request:/); assert.match(linux, /checks:\n    runs-on: ubuntu-24\.04/);
  assert.match(mac, /pull_request:\n    types: \[labeled\]/); assert.doesNotMatch(mac, /synchronize|schedule:/);
});

test('artifact publication rechecks host memory, disk and swap after copy/hash/inspection', async () => {
  const plan = { scratch: '/safe/build', ciProfile: profile };
  const healthy = { availableMemoryBytes: 2 * 1024 ** 3, freeDiskBytes: 8 * 1024 ** 3, swapBytes: 0 };
  const session = createMacBuildSession(profile, 0);
  await finalMacBuildCheck(plan, session, async () => healthy);
  assert.deepEqual(session.evidence.finalHostResources, healthy); assert.equal(session.evidence.completed, false);
  for (const patch of [{ availableMemoryBytes: 1 }, { freeDiskBytes: 1 }, { swapBytes: 257 * 1024 ** 2 }, { swapBytes: NaN }])
    await assert.rejects(finalMacBuildCheck(plan, createMacBuildSession(profile, 0), async () => ({ ...healthy, ...patch })), /resource check failed/);
  const deadline = createMacBuildSession(profile, 0); deadline.deadline = performance.now() + 15;
  await assert.rejects(finalMacBuildCheck(plan, deadline, () => new Promise(() => {})), /telemetry unavailable/);
});
