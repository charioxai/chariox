import assert from "node:assert/strict"
import { chmod, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from "node:fs/promises"
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
import {
  assertProcessHealth,
  assertSandboxCapableChromiumIdentity,
  baselineResourceSnapshot,
  captureSoakScreenshot,
  processIsRunning,
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

test("soak screenshot capture overwrites its stable evidence path", async (context) => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-soak-screenshot-"))
  context.after(() => rm(root, { recursive: true, force: true }))
  const fakeBin = path.join(root, "bin")
  const screenshotPath = path.join(root, "latest-screen.png")
  await mkdir(fakeBin)
  const fakeScrot = path.join(fakeBin, "scrot")
  await writeFile(fakeScrot, `#!/bin/sh
overwrite=
if test "$1" = "-o"; then
  overwrite=1
  shift
fi
target=$1
if test -e "$target" && test -z "$overwrite"; then
  echo "scrot can no longer generate new file names" >&2
  exit 1
fi
printf frame > "$target"
`)
  await chmod(fakeScrot, 0o700)
  const execOptions = {
    cwd: root,
    env: { ...process.env, PATH: `${fakeBin}:${process.env.PATH}` },
    timeout: 10_000,
  }

  await captureSoakScreenshot(screenshotPath, execOptions)
  await captureSoakScreenshot(screenshotPath, execOptions)

  assert.deepEqual((await readdir(root)).sort(), ["bin", "latest-screen.png"])
  assert.equal(await readFile(screenshotPath, "utf8"), "frame")
})

test("cleanup treats a reparented zombie as stopped", () => {
  const probe = () => {}
  const live = () => "13499 (selkies) S 1 13499 13499 0 -1"
  const zombie = () => "13499 (selkies) Z 1 13499 13499 0 -1"

  assert.equal(processIsRunning(13499, { probe, readStat: live }), true)
  assert.equal(processIsRunning(13499, { probe, readStat: zombie }), false)
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
