import assert from "node:assert/strict"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  DEFAULT_SOAK_DURATION_SECONDS,
  SMOKE_DURATION_SECONDS,
  buildSoakPaths,
  detachedLaunchSummary,
  parseBrowserComputerSoakArgs,
  validateCompletedSoakResult,
} from "./browser-computer-soak.mjs"
import { assertSandboxCapableChromiumIdentity } from "./browser-computer-soak-runtime.mjs"

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
    () => assertSandboxCapableChromiumIdentity({ uid: 0 }),
    /refuses root Chromium because the renderer sandbox would be disabled.*non-root slice user/,
  )
  assert.throws(
    () => assertSandboxCapableChromiumIdentity({ uid: 0, managedProviderIsolationActive: "1" }),
    /slice-owner desktop\/runtime instead of the provider sandbox/,
  )
})

test("run paths keep logs, state, samples, status, result, PID, and cleanup together", () => {
  const paths = buildSoakPaths("/tmp/evidence", "2026-09-10T00-00-00-000Z")
  assert.equal(paths.runDir, "/tmp/evidence/2026-09-10T00-00-00-000Z")
  assert.equal(paths.log, `${paths.runDir}/runner.log`)
  assert.equal(paths.pid, `${paths.runDir}/runner.pid`)
  assert.equal(paths.status, `${paths.runDir}/status.json`)
  assert.equal(paths.result, `${paths.runDir}/result.json`)
  assert.equal(paths.samples, `${paths.runDir}/resource-samples.jsonl`)
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
    cleanup: paths.cleanup,
    failure: paths.failure,
    preflight: paths.preflight,
  })
})

test("completed result requires browser, controller, changing stream frames, samples, and cleanup", () => {
  const valid = {
    schema: "chariox.browser_computer_soak.v1",
    status: "passed",
    firstHealth: { state: "ready", process_id: 42, diagnostic_code: null },
    activity: { iterations: 3, controllerRequests: 12, chromiumMutations: 3 },
    stream: { binaryFrames: 2, binaryBytes: 1024, changingFrameDigests: 2 },
    resources: { sampleCount: 3, peakOwnedRssBytes: 1024, peakOwnedCpuPercent: 1 },
    cleanup: { clean: true },
  }
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
