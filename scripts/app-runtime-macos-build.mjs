import assert from 'node:assert/strict';
import { availableParallelism } from 'node:os';
import { join } from 'node:path';
import { writeFile } from 'node:fs/promises';
import { validateMacProfile } from './app-runtime-ci-resources.mjs';
import { swapUsage } from './app-runtime-macos-preflight.mjs';
import { readMacHostResources, resourceFailure } from './app-runtime-macos-watch.mjs';

export function checkMacBuilder(profile, environment, platform = process.platform, arch = process.arch) {
  validateMacProfile(profile);
  assert.equal(platform, 'darwin'); assert.equal(arch, 'arm64');
  assert.equal(environment.GITHUB_ACTIONS, 'true');
  assert.equal(environment.RUNNER_ENVIRONMENT, 'github-hosted');
  assert.equal(environment.GITHUB_REPOSITORY, 'charioxai/chariox');
  assert.ok(['workflow_dispatch', 'pull_request'].includes(environment.GITHUB_EVENT_NAME));
  assert.equal(environment.NATIVE_ADMISSION_EVENT, environment.GITHUB_EVENT_NAME === 'pull_request' ? 'pull_request:labeled' : 'workflow_dispatch');
  assert.equal(environment.CHARIOX_NATIVE_BUILD_CONFIRMED, 'true');
  assert.equal(environment.ImageOS, 'macos15');
  assert.match(environment.ImageVersion ?? '', /^[0-9.]{1,40}$/);
  assert.match(environment.GITHUB_RUN_ID ?? '', /^[1-9][0-9]{0,19}$/);
  assert.match(environment.GITHUB_RUN_ATTEMPT ?? '', /^[1-9][0-9]{0,19}$/);
}

export function checkMacTools(profile, observed, tools) {
  assert.equal(observed.xcode, 'Xcode 16.4\nBuild version 16F6');
  assert.equal(observed.sdkVersion, profile.sdkVersion);
  assert.equal(observed.developerDirectory, profile.developerDirectory);
  assert.equal(observed.sdkPath, `${profile.developerDirectory}/Platforms/MacOSX.platform/Developer/SDKs/MacOSX${profile.sdkVersion}.sdk`);
  for (const key of ['cc', 'cxx']) {
    assert.ok(tools[key].startsWith(`${profile.developerDirectory}/Toolchains/XcodeDefault.xctoolchain/usr/bin/`));
    assert.ok(observed[key].startsWith(`Apple clang version 17.0.0 (${profile.compilerBuild})\n`));
  }
}

export function checkMachO(target, architectures, installName, name) {
  assert.equal(architectures, target.arch === 'arm64' ? 'arm64' : 'x86_64');
  assert.ok([target.runtimeLibrary, target.nodeLibrary].includes(name));
  assert.equal(installName, `@rpath/${name}`);
}

export function createMacBuildSession(profile, initialSwapBytes, now = performance.now()) {
  validateMacProfile(profile);
  assert.ok(Number.isSafeInteger(initialSwapBytes) && initialSwapBytes >= 0);
  const evidence = { classification: profile.enforcement, samples: 0, initialSwapBytes,
    peakGroupRssBytes: 0, peakGroupProcesses: 0, peakSwapBytes: initialSwapBytes,
    minimumAvailableMemoryBytes: null, minimumFreeDiskBytes: null, completed: false };
  return {
    deadline: now + profile.maxDurationMs, initialSwapBytes, evidence,
    observe(sample) {
      evidence.samples++;
      evidence.peakGroupRssBytes = Math.max(evidence.peakGroupRssBytes, sample.groupRssBytes);
      evidence.peakGroupProcesses = Math.max(evidence.peakGroupProcesses, sample.groupProcesses);
      evidence.peakSwapBytes = Math.max(evidence.peakSwapBytes, sample.swapBytes);
      evidence.minimumAvailableMemoryBytes = Math.min(evidence.minimumAvailableMemoryBytes ?? Infinity, sample.availableMemoryBytes);
      evidence.minimumFreeDiskBytes = Math.min(evidence.minimumFreeDiskBytes ?? Infinity, sample.freeDiskBytes);
    },
  };
}

export function prepareMacBuild(plan, checkedOutput) {
  checkMacBuilder(plan.ciProfile, process.env);
  assert.equal(process.versions.node, '24.20.0');
  const environment = { PATH: '/usr/bin:/bin:/usr/sbin:/sbin', LANG: 'C', LC_ALL: 'C', DEVELOPER_DIR: plan.ciProfile.developerDirectory };
  const initialSwap = swapUsage(checkedOutput('/usr/sbin/sysctl', ['-n', 'vm.swapusage'], environment));
  const session = createMacBuildSession(plan.ciProfile, initialSwap.usedBytes);
  session.evidence.runnerImage = { os: process.env.ImageOS, version: process.env.ImageVersion };
  session.evidence.run = { repository: process.env.GITHUB_REPOSITORY, id: process.env.GITHUB_RUN_ID, attempt: process.env.GITHUB_RUN_ATTEMPT };
  session.evidence.logicalCpuCount = availableParallelism();
  session.evidence.initialSwap = initialSwap;
  return session;
}

export async function finalMacBuildCheck(plan, session, sample = readMacHostResources) {
  const remaining = Math.floor(session.deadline - performance.now());
  if (remaining <= 0) throw new Error('macOS build deadline reached before final resource check');
  const controller = new AbortController(); let timeout;
  try {
    const host = await Promise.race([
      sample(plan.scratch, controller.signal),
      new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error('macOS final resource telemetry unavailable')),
        Math.min(remaining, plan.ciProfile.sampleTimeoutMs)); }),
    ]);
    const reason = resourceFailure({ ...host, groupProcesses: 1, groupRssBytes: 0 }, session.initialSwapBytes);
    if (reason !== null) throw new Error(`macOS final resource check failed: ${reason}`);
    if (performance.now() >= session.deadline) throw new Error('macOS build deadline reached during final resource check');
    // No build process group remains at this stage: these are host observations,
    // not a fabricated final process sample or a claim of hard caps.
    session.evidence.finalHostResources = host;
  } finally { clearTimeout(timeout); controller.abort(); }
}

export async function recordMacBuild(plan, session, completed) {
  session.evidence.completed = completed;
  // Separate evidence from the strict four-member native artifact archive.
  await writeFile(join(plan.scratch, 'macos-build-resources.json'), `${JSON.stringify(session.evidence, null, 2)}\n`, { flag: 'wx', mode: 0o600 });
}
