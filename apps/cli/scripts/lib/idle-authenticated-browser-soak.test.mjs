import assert from "node:assert/strict"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  DEFAULT_IDLE_SOAK_DURATION_SECONDS,
  assertCheckpointAdvanced,
  assertResourceCeilings,
  assertRetainedEvidenceRedacted,
  buildIdleSoakPaths,
  parseIdleAuthenticatedBrowserSoakArgs,
  validateCompletedIdleSoakResult,
} from "./idle-authenticated-browser-soak.mjs"

const repoRoot = path.resolve(import.meta.dirname, "..", "..", "..", "..")

test("idle authenticated soak defaults to 24 hours with explicit ceilings", () => {
  const options = parseIdleAuthenticatedBrowserSoakArgs([], { repoRoot, homeDir: os.tmpdir() })
  assert.equal(DEFAULT_IDLE_SOAK_DURATION_SECONDS, 24 * 60 * 60)
  assert.deepEqual({
    durationSeconds: options.durationSeconds,
    healthIntervalSeconds: options.healthIntervalSeconds,
    sampleIntervalSeconds: options.sampleIntervalSeconds,
    maxCpuPercent: options.maxCpuPercent,
    maxRssMb: options.maxRssMb,
    maxProcesses: options.maxProcesses,
    minFreeDiskMb: options.minFreeDiskMb,
  }, {
    durationSeconds: 86_400,
    healthIntervalSeconds: 30,
    sampleIntervalSeconds: 30,
    maxCpuPercent: 300,
    maxRssMb: 2_048,
    maxProcesses: 32,
    minFreeDiskMb: 1_024,
  })
})

test("smoke mode is short and keeps the same safety contract", () => {
  const options = parseIdleAuthenticatedBrowserSoakArgs(["--smoke"], { repoRoot, homeDir: os.tmpdir() })
  assert.equal(options.durationSeconds, 15)
  assert.equal(options.healthIntervalSeconds, 2)
  assert.equal(options.sampleIntervalSeconds, 2)
  assert.equal(options.idleAuthenticated, true)
})

test("unsafe limits and repository evidence paths fail before runtime state", () => {
  for (const args of [
    ["--max-cpu-percent", "0"],
    ["--max-rss-mb", "0"],
    ["--max-processes", "0"],
    ["--min-free-disk-mb", "0"],
    ["--duration-seconds", "86401"],
    ["--evidence-root", path.join(repoRoot, ".artifacts")],
  ]) assert.throws(() => parseIdleAuthenticatedBrowserSoakArgs(args, { repoRoot, homeDir: os.tmpdir() }))
})

test("evidence paths include ownership, profile, failure, and cleanup records", () => {
  const paths = buildIdleSoakPaths("/tmp/evidence", "run")
  assert.equal(paths.ownership, "/tmp/evidence/run/process-ownership.json")
  assert.equal(paths.profile, "/tmp/evidence/run/profile-marker.json")
  assert.equal(paths.cleanup, "/tmp/evidence/run/cleanup-ledger.json")
  assert.equal(paths.failure, "/tmp/evidence/run/failure.json")
})

test("health checkpoints must advance every monotonic counter", () => {
  const before = { healthChecks: 4, controllerRequests: 8, authenticatedSessionChecks: 4, profileMarkerChecks: 4 }
  const after = { healthChecks: 5, controllerRequests: 10, authenticatedSessionChecks: 5, profileMarkerChecks: 5 }
  assert.deepEqual(assertCheckpointAdvanced(before, after), after)
  for (const key of Object.keys(before)) {
    const stalled = { ...after, [key]: before[key] }
    assert.throws(() => assertCheckpointAdvanced(before, stalled), new RegExp(key))
  }
})

test("resource ceilings fail closed at the configured boundary", () => {
  const limits = { maxCpuPercent: 300, maxRssMb: 2_048, maxProcesses: 32, minFreeDiskMb: 1_024 }
  const safe = { owned: { cpuPercent: 20, rssBytes: 512 * 1024 ** 2, processCount: 12 }, disk: { availableBytes: 8 * 1024 ** 3 } }
  assert.deepEqual(assertResourceCeilings(safe, limits), safe)
  assert.throws(() => assertResourceCeilings({ ...safe, owned: { ...safe.owned, processCount: 33 } }, limits), /process ceiling/)
  assert.throws(() => assertResourceCeilings({ ...safe, owned: { ...safe.owned, rssBytes: 2_049 * 1024 ** 2 } }, limits), /memory ceiling/)
  assert.throws(() => assertResourceCeilings({ ...safe, owned: { ...safe.owned, cpuPercent: 301 } }, limits), /CPU ceiling/)
  assert.throws(() => assertResourceCeilings({ ...safe, disk: { availableBytes: 1_023 * 1024 ** 2 } }, limits), /disk floor/)
})

test("synthetic auth material is forbidden from retained evidence", () => {
  assert.equal(assertRetainedEvidenceRedacted(["status ok", "marker digest abc"], ["cookie-value", "marker-value"]), true)
  assert.throws(() => assertRetainedEvidenceRedacted(["cookie=cookie-value"], ["cookie-value"]), /leak/)
})

test("completed results require health, resources, redaction, and clean cleanup", () => {
  const result = {
    schema: "chariox.idle_authenticated_browser_soak.v1",
    status: "passed",
    counters: { healthChecks: 2, controllerRequests: 4, authenticatedSessionChecks: 2, profileMarkerChecks: 2 },
    resources: { sampleCount: 2, ceilingsRespected: true },
    redaction: { passed: true },
    cleanup: { clean: true },
  }
  assert.deepEqual(validateCompletedIdleSoakResult(result), result)
  assert.throws(() => validateCompletedIdleSoakResult({ ...result, cleanup: { clean: false } }), /cleanup/)
})
