import assert from "node:assert/strict"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  DEFAULT_SOAK_DURATION_SECONDS,
  SMOKE_DURATION_SECONDS,
  buildSoakPaths,
  detachedLaunchSummary,
  gateFingerprint,
  validateGatePrerequisites,
  parseBrowserComputerSoakArgs,
  validateCompletedSoakResult,
} from "./browser-computer-soak.mjs"
import {
  assertProcessHealth,
  assertSandboxCapableChromiumIdentity,
  baselineResourceSnapshot,
  createLifecycleGuard,
  finalGateEligibility,
  findRuntimeLeakMatches,
  processIdentityMatches,
  runBrowserComputerSoak,
} from "./browser-computer-soak-runtime.mjs"

const repoRoot = path.resolve(import.meta.dirname, "..", "..", "..", "..")

test("real soak defaults to eight hours with bounded active samples", () => {
  const options = parseBrowserComputerSoakArgs([], { repoRoot, homeDir: os.tmpdir() })
  assert.equal(DEFAULT_SOAK_DURATION_SECONDS, 8 * 60 * 60)
  assert.equal(options.durationSeconds, DEFAULT_SOAK_DURATION_SECONDS)
  assert.equal(options.activityIntervalSeconds, 10)
  assert.equal(options.sampleIntervalSeconds, 10)
  assert.equal(options.mode, "run")
})

test("smoke mode is deterministic and short", () => {
  const options = parseBrowserComputerSoakArgs(["--smoke"], { repoRoot, homeDir: os.tmpdir() })
  assert.equal(options.durationSeconds, SMOKE_DURATION_SECONDS)
  assert.equal(options.activityIntervalSeconds, 1)
  assert.equal(options.sampleIntervalSeconds, 1)
  assert.equal(options.smoke, true)
})

test("explicit duration and external evidence root are accepted", () => {
  const evidenceRoot = path.join(os.tmpdir(), "chariox-browser-computer-soak-evidence")
  const options = parseBrowserComputerSoakArgs([
    "--duration-seconds", "42",
    "--activity-interval-seconds=3",
    "--sample-interval-seconds", "4",
    "--evidence-root", evidenceRoot,
  ], { repoRoot, homeDir: os.tmpdir() })
  assert.equal(options.durationSeconds, 42)
  assert.equal(options.activityIntervalSeconds, 3)
  assert.equal(options.sampleIntervalSeconds, 4)
  assert.equal(options.evidenceRoot, evidenceRoot)
})

test("historical long intervals derive a safe cadence bound unless explicitly tightened", () => {
  const options = parseBrowserComputerSoakArgs([
    "--activity-interval-seconds", "300", "--sample-interval-seconds", "120",
  ], { repoRoot, homeDir: os.tmpdir() })
  assert.equal(options.limits.maxCadenceGapMs, 900_000)
  assert.throws(() => parseBrowserComputerSoakArgs([
    "--activity-interval-seconds", "300", "--max-cadence-gap-seconds", "30",
  ], { repoRoot, homeDir: os.tmpdir() }), /must exceed both activity and sample intervals/)
})

test("unsafe durations and repository evidence paths are rejected", () => {
  assert.throws(
    () => parseBrowserComputerSoakArgs(["--duration-seconds", "0"], { repoRoot, homeDir: os.tmpdir() }),
    /duration-seconds must be an integer from 1 to 86400/,
  )
  assert.throws(
    () => parseBrowserComputerSoakArgs(["--evidence-root", path.join(repoRoot, ".artifacts")], { repoRoot, homeDir: os.tmpdir() }),
    /evidence must stay outside repositories/,
  )
})

test("soak requires a non-root Chromium identity instead of disabling its sandbox", () => {
  assert.doesNotThrow(() => assertSandboxCapableChromiumIdentity({ uid: 1000 }))
  assert.throws(
    () => assertSandboxCapableChromiumIdentity({ uid: 0, managedProviderIsolationActive: null }),
    /refuses root Chromium because the renderer sandbox would be disabled.*non-root slice user/,
  )
  assert.throws(
    () => assertSandboxCapableChromiumIdentity({ uid: 0, managedProviderIsolationActive: "1" }),
    /slice-owner desktop\/runtime instead of the provider sandbox/,
  )
})

test("signal-terminated children fail health checks even when exitCode is null", () => {
  const killed = { exitCode: null, signalCode: "SIGKILL" }
  const running = { exitCode: null, signalCode: null }
  assert.throws(
    () => assertProcessHealth(new Map([["openbox", killed]]), { child: running }, {
      child: running,
      ready: true,
      readyAt: Date.now(),
      lastBinaryFrameAt: Date.now(),
    }),
    /openbox exited during soak with signal SIGKILL/,
  )
})

test("a ready stream must deliver its first frame before the health deadline", () => {
  const running = { exitCode: null, signalCode: null }
  assert.throws(
    () => assertProcessHealth(new Map(), { child: running }, {
      child: running,
      ready: true,
      readyAt: 1_000,
      lastBinaryFrameAt: null,
    }, 31_001),
    /did not deliver its first video frame/,
  )
})

test("the preflight baseline measures the evidence filesystem", async () => {
  const paths = buildSoakPaths("/evidence", "run")
  let observedDiskPath = null
  const baseline = await baselineResourceSnapshot(paths, {
    snapshot: async (_label, _pids, diskPath) => {
      observedDiskPath = diskPath
      return { disk: { path: diskPath } }
    },
  })
  assert.equal(observedDiskPath, paths.runDir)
  assert.equal(baseline.disk.path, paths.runDir)
})

test("startup failures leave terminal status, result, failure, and cleanup evidence", async (context) => {
  const evidenceRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-soak-startup-failure-"))
  context.after(() => rm(evidenceRoot, { recursive: true, force: true }))
  const paths = buildSoakPaths(evidenceRoot, "run")
  const options = {
    mode: "run",
    smoke: true,
    durationSeconds: 12,
    activityIntervalSeconds: 1,
    sampleIntervalSeconds: 1,
    evidenceRoot,
    runDir: paths.runDir,
    displayNumber: 70,
    debugPort: 52_000,
    viewerPort: 52_000,
    internalRun: true,
  }
  await assert.rejects(
    runBrowserComputerSoak({ options, repoRoot, scriptPath: "/unused" }),
    /debug-port and viewer-port must differ/,
  )
  const [status, result, failure, cleanup] = await Promise.all([
    readFile(paths.status, "utf8").then(JSON.parse),
    readFile(paths.result, "utf8").then(JSON.parse),
    readFile(paths.failure, "utf8").then(JSON.parse),
    readFile(paths.cleanup, "utf8").then(JSON.parse),
  ])
  assert.equal(status.status, "failed")
  assert.equal(result.status, "failed")
  assert.equal(result.phase, "startup")
  assert.match(failure.marker, /debug-port and viewer-port must differ/)
  assert.equal(cleanup.clean, true)
})

test("run paths keep logs, state, samples, status, result, PID, and cleanup together", () => {
  const paths = buildSoakPaths("/tmp/evidence", "2026-09-10T00-00-00-000Z")
  assert.equal(paths.runDir, "/tmp/evidence/2026-09-10T00-00-00-000Z")
  assert.equal(paths.log, `${paths.runDir}/runner.log`)
  assert.equal(paths.pid, `${paths.runDir}/runner.pid`)
  assert.equal(paths.status, `${paths.runDir}/status.json`)
  assert.equal(paths.result, `${paths.runDir}/result.json`)
  assert.equal(paths.samples, `${paths.runDir}/resource-samples.jsonl`)
  assert.equal(paths.activity, `${paths.runDir}/activity.jsonl`)
  assert.equal(paths.cleanup, `${paths.runDir}/cleanup-ledger.json`)
  assert.equal(paths.failure, `${paths.runDir}/failure.json`)
})

test("detached launch summary preserves numeric PID and started status", () => {
  const paths = buildSoakPaths("/tmp/evidence", "run")
  assert.deepEqual(detachedLaunchSummary(paths, 1234), {
    status: "started",
    pid: 1234,
    runDir: paths.runDir,
    statusPath: paths.status,
    pidPath: paths.pid,
    log: paths.log,
    result: paths.result,
    samples: paths.samples,
    activity: paths.activity,
    cleanup: paths.cleanup,
    failure: paths.failure,
    preflight: paths.preflight,
  })
})

test("completed result requires browser, controller, changing stream frames, samples, and cleanup", () => {
  const valid = completedResult()
  assert.deepEqual(validateCompletedSoakResult(valid), valid)
  for (const mutate of [
    (value) => { value.activity.chromiumMutations = 0 },
    (value) => { value.firstHealth.state = "starting" },
    (value) => { value.activity.controllerRequests = 0 },
    (value) => { value.stream.binaryFrames = 0 },
    (value) => { value.stream.changingFrameDigests = 0 },
    (value) => { value.resources.sampleCount = 0 },
    (value) => { value.cleanup.clean = false },
  ]) {
    const candidate = structuredClone(valid)
    mutate(candidate)
    assert.throws(() => validateCompletedSoakResult(candidate), /soak result/)
  }
})

test("completion cannot false-pass without active Browser and Computer evidence", () => {
  const valid = completedResult()
  for (const mutate of [
    (value) => { value.finalHealth = null },
    (value) => { value.finalHealth.process_id += 1 },
    (value) => { value.activity.structuredBrowserActions = 0 },
    (value) => { value.activity.computerScreenshots = 0 },
    (value) => { value.activity.computerInputs = 0 },
    (value) => { value.activity.screenshotDigests = 1 },
    (value) => { value.activity.lastAt = "2026-09-11T07:00:00.000Z" },
    (value) => { value.stream.ready = false },
    (value) => { value.stream.finalBinaryFrames = value.stream.initialBinaryFrames },
    (value) => { value.stream.finalActivityCounter = value.stream.initialActivityCounter },
    (value) => { value.stream.finalChangingFrameDigests = value.stream.initialChangingFrameDigests },
  ]) {
    const candidate = structuredClone(valid)
    mutate(candidate)
    assert.throws(() => validateCompletedSoakResult(candidate), /soak result/)
  }
})

test("completion requires authoritative clean source, image, backend, protocol, and limits", () => {
  const valid = completedResult()
  for (const mutate of [
    (value) => { value.provenance.source.dirty = true },
    (value) => { value.provenance.source.commit = "not-a-commit" },
    (value) => { value.provenance.image.identity = "" },
    (value) => { value.provenance.viewer.backend = "novnc" },
    (value) => { value.provenance.localDaemonProtocolVersion = 321 },
    (value) => { value.provenance.limits.maxProcesses += 1 },
  ]) {
    const candidate = structuredClone(valid)
    mutate(candidate)
    assert.throws(() => validateCompletedSoakResult(candidate), /soak result/)
  }
})

test("completion rejects clock gaps, stale evidence, unbounded resources, and cleanup leaks", () => {
  const valid = completedResult()
  for (const mutate of [
    (value) => { value.timing.monotonicElapsedMs = value.timing.expectedDurationMs - 1 },
    (value) => { value.timing.observedMaxCadenceGapMs = value.timing.maxCadenceGapMs + 1 },
    (value) => { value.timing.wallMonotonicSkewMs = value.timing.maxCadenceGapMs + 1 },
    (value) => { value.resources.withinBounds = false },
    (value) => { value.resources.peakOwnedRssBytes = value.resources.limits.maxRssBytes + 1 },
    (value) => { value.resources.peakOwnedCpuPercent = value.resources.limits.maxCpuPercent + 1 },
    (value) => { value.resources.peakOwnedProcessCount = value.resources.limits.maxProcesses + 1 },
    (value) => { value.resources.diskGrowthBytes = value.resources.limits.maxDiskGrowthBytes + 1 },
    (value) => { value.resources.peakOwnedOpenFiles = value.resources.limits.maxOpenFiles + 1 },
    (value) => { value.resources.networkBytes = value.resources.limits.maxNetworkBytes + 1 },
    (value) => { value.cleanup.pidReuseSafe = false },
    (value) => { value.cleanup.leakScan.clean = false },
    (value) => { value.cleanup.remainingListeners.push("127.0.0.1:55000") },
  ]) {
    const candidate = structuredClone(valid)
    mutate(candidate)
    assert.throws(() => validateCompletedSoakResult(candidate), /soak result/)
  }
})

test("noVNC evidence is recorded but cannot close the final gate", () => {
  const candidate = completedResult()
  candidate.viewer.backend = "novnc"
  candidate.provenance.viewer.backend = "novnc"
  candidate.gate.eligible = false
  candidate.gate.reason = "novnc_not_final_gate"
  assert.deepEqual(validateCompletedSoakResult(candidate), candidate)
  candidate.gate.eligible = true
  assert.throws(() => validateCompletedSoakResult(candidate), /noVNC.*final gate/i)
})

test("Selkies final-gate eligibility activates exactly at protocol 322", () => {
  assert.deepEqual(finalGateEligibility("selkies", 321), { eligible: false, reason: "protocol_322_not_integrated" })
  assert.deepEqual(finalGateEligibility("selkies", 322), { eligible: true, reason: null })
  assert.deepEqual(finalGateEligibility("novnc", 322), { eligible: false, reason: "novnc_not_final_gate" })
})

test("preflight and smoke receipts must be fresh and match clean source, image, and limits", () => {
  const provenance = {
    source: { commit: "a".repeat(40), tree: "b".repeat(40), dirty: false },
    image: { identity: "sha256:" + "c".repeat(64) },
    limits: { maxRssBytes: 1024, maxCpuPercent: 200 },
    viewer: { backend: "selkies" },
  }
  const fingerprint = gateFingerprint(provenance)
  const now = Date.parse("2026-09-11T08:00:00.000Z")
  const preflight = { schema: "chariox.browser_computer_soak_gate_receipt.v1", phase: "preflight", status: "passed", completedAt: "2026-09-11T07:55:00.000Z", fingerprint }
  const smoke = { ...preflight, phase: "smoke", completedAt: "2026-09-11T07:58:00.000Z", cleanup: { clean: true } }
  assert.doesNotThrow(() => validateGatePrerequisites({ preflight, smoke, provenance, now }))
  for (const mutate of [
    ({ provenance: value }) => { value.source.dirty = true },
    ({ preflight: value }) => { value.fingerprint = "wrong" },
    ({ smoke: value }) => { value.fingerprint = "wrong" },
    ({ smoke: value }) => { value.completedAt = "2026-09-10T00:00:00.000Z" },
    ({ smoke: value }) => { value.cleanup.clean = false },
  ]) {
    const candidate = structuredClone({ preflight, smoke, provenance })
    mutate(candidate)
    assert.throws(() => validateGatePrerequisites({ ...candidate, now }), /preflight|smoke|clean|stale/i)
  }
})

test("controller, browser, and stream death each fail immediately", () => {
  const running = { exitCode: null, signalCode: null }
  const dead = { exitCode: 9, signalCode: null }
  const stream = { child: running, ready: true, readyAt: 1_000, lastBinaryFrameAt: 2_000 }
  assert.throws(() => assertProcessHealth(new Map(), { child: dead }, stream, 2_001), /Browser Controller exited/)
  assert.throws(() => assertProcessHealth(new Map([["chromium", dead]]), { child: running }, stream, 2_001), /chromium exited/)
  assert.throws(() => assertProcessHealth(new Map(), { child: running }, { ...stream, child: dead }, 2_001), /stream exited/)
})

test("cleanup identities reject PID reuse without treating the replacement as owned", () => {
  const expected = { pid: 42, startedAtTicks: "100", executable: "/usr/bin/chromium" }
  assert.equal(processIdentityMatches(expected, { ...expected }), true)
  assert.equal(processIdentityMatches(expected, { ...expected, startedAtTicks: "101" }), false)
  assert.equal(processIdentityMatches(expected, { ...expected, executable: "/usr/bin/unrelated" }), false)
  assert.equal(processIdentityMatches(expected, null), false)
})

test("leak scanning is bounded, excludes the runner, and never retains full arguments", () => {
  const secret = "DO_NOT_RETAIN_THIS_ARGUMENT"
  const listing = [
    `10 node runner --state /tmp/owned ${secret}`,
    `11 chromium chromium --user-data-dir=/tmp/owned ${secret}`,
    `12 harmless harmless --other /tmp/unrelated ${secret}`,
    `13 Xvfb Xvfb :77 ${secret}`,
  ].join("\n")
  assert.deepEqual(findRuntimeLeakMatches(listing, {
    markers: ["/tmp/owned", "Xvfb :77"], currentPid: 10, maximumMatches: 1,
  }), [{ pid: 11, command: "chromium" }])
})

test("lifecycle guard rejects stale callbacks after abort and supersession", () => {
  const guard = createLifecycleGuard()
  const first = guard.begin()
  assert.equal(guard.accept(first), true)
  const second = guard.begin()
  assert.equal(guard.accept(first), false)
  assert.equal(guard.accept(second), true)
  guard.abort()
  assert.equal(guard.accept(second), false)
  assert.equal(guard.abort(), false, "abort must be idempotent")
})

function completedResult() {
  const limits = { maxRssBytes: 4096, maxCpuPercent: 200, maxProcesses: 32, maxDiskGrowthBytes: 4096, maxOpenFiles: 100, maxNetworkBytes: 4096 }
  const source = { commit: "a".repeat(40), tree: "b".repeat(40), branch: "test", dirty: false }
  return {
    schema: "chariox.browser_computer_soak.v1",
    status: "passed",
    startedAt: "2026-09-11T00:00:00.000Z",
    completedAt: "2026-09-11T08:00:00.000Z",
    source,
    provenance: {
      schema: "chariox.browser_computer_soak_provenance.v1",
      capturedAt: "2026-09-10T23:59:59.000Z",
      source: { ...source },
      image: { identity: "sha256:" + "c".repeat(64) },
      limits: { ...limits },
      viewer: { backend: "selkies" },
      localDaemonProtocolVersion: 322,
    },
    firstHealth: { state: "ready", process_id: 42, diagnostic_code: null },
    controller: { pid: 42, startedAtTicks: "100", executable: "/usr/bin/node" },
    finalHealth: { state: "ready", process_id: 42, diagnostic_code: null, checkedAt: "2026-09-11T07:59:59.000Z" },
    activity: {
      iterations: 3, controllerRequests: 12, chromiumMutations: 3,
      structuredBrowserActions: 3, computerScreenshots: 3, computerInputs: 3,
      screenshotDigests: 3, lastAt: "2026-09-11T07:59:58.000Z",
    },
    viewer: { backend: "selkies" },
    stream: {
      ready: true, binaryFrames: 12, binaryBytes: 1024, changingFrameDigests: 4,
      initialBinaryFrames: 2, finalBinaryFrames: 12,
      initialActivityCounter: 2, finalActivityCounter: 12,
      initialChangingFrameDigests: 2, finalChangingFrameDigests: 4,
      lastBinaryFrameAt: "2026-09-11T07:59:58.000Z",
    },
    timing: {
      expectedDurationMs: 28_800_000, monotonicElapsedMs: 28_800_001,
      maxCadenceGapMs: 30_000, observedMaxCadenceGapMs: 10_100, wallMonotonicSkewMs: 5,
    },
    resources: {
      sampleCount: 3, peakOwnedRssBytes: 1024, peakOwnedCpuPercent: 1,
      peakOwnedProcessCount: 8, peakOwnedOpenFiles: 40, diskGrowthBytes: 1024, networkBytes: 2048,
      limits,
      withinBounds: true,
    },
    cleanup: { clean: true, pidReuseSafe: true, remainingPids: [], remainingListeners: [], leakScan: { clean: true, matches: [] } },
    gate: { eligible: true, reason: null },
  }
}
