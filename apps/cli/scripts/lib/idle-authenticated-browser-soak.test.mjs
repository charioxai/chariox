import assert from "node:assert/strict"
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { spawn } from "node:child_process"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  DEFAULT_IDLE_SOAK_DURATION_SECONDS,
  assertCheckpointAdvanced,
  assertResourceCeilings,
  assertRetainedEvidenceRedacted,
  assertCleanIdleSoakRunDirectory,
  minimumIdleSoakCheckpointCount,
  buildIdleSoakPaths,
  parseIdleAuthenticatedBrowserSoakArgs,
  validateIdleSoakDetachContract,
  validateCompletedIdleSoakResult,
} from "./idle-authenticated-browser-soak.mjs"
import {
  assertControllerReady,
  captureProcessIdentity,
  cleanupOwned,
  ControllerClient,
  launchOwnedProcess,
  materializeLaunchFailure,
  parseControllerResponseLine,
  processIdentityMatches,
  runIdleAuthenticatedBrowserSoak,
  spawnLogged,
  startFixtureServer,
  terminateOwnedTree,
  validateTerminalIdleSoakEvidence,
  waitForDetachContract,
  waitForLogClosed,
} from "./idle-authenticated-browser-soak-runtime.mjs"

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
  assert.equal(paths.checkpoints, "/tmp/evidence/run/health-checkpoints.jsonl")
})

test("health checkpoints must advance every monotonic counter", () => {
  const before = { healthChecks: 4, controllerRequests: 8, authenticatedSessionChecks: 4, profileMarkerChecks: 4, freshAuthenticatedRequests: 4 }
  const after = { healthChecks: 5, controllerRequests: 10, authenticatedSessionChecks: 5, profileMarkerChecks: 5, freshAuthenticatedRequests: 5 }
  assert.deepEqual(assertCheckpointAdvanced(before, after), after)
  for (const key of Object.keys(before)) {
    const stalled = { ...after, [key]: before[key] }
    assert.throws(() => assertCheckpointAdvanced(before, stalled), new RegExp(key))
  }
})

test("resource ceilings fail closed at the configured boundary", () => {
  const limits = { maxCpuPercent: 300, maxRssMb: 2_048, maxProcesses: 32, minFreeDiskMb: 1_024 }
  const safe = { owned: { cpuPercent: 20, rssBytes: 512 * 1024 ** 2, processCount: 12 },
    disk: { availableBytes: 8 * 1024 ** 3, totalBytes: 16 * 1024 ** 3 },
    host: { totalMemoryBytes: 16 * 1024 ** 3, freeMemoryBytes: 8 * 1024 ** 3, loadAverage: [0, 0, 0] } }
  assert.deepEqual(assertResourceCeilings(safe, limits), safe)
  assert.throws(() => assertResourceCeilings({ ...safe, owned: { ...safe.owned, processCount: 33 } }, limits), /process ceiling/)
  assert.throws(() => assertResourceCeilings({ ...safe, owned: { ...safe.owned, rssBytes: 2_049 * 1024 ** 2 } }, limits), /memory ceiling/)
  assert.throws(() => assertResourceCeilings({ ...safe, owned: { ...safe.owned, cpuPercent: 301 } }, limits), /CPU ceiling/)
  assert.throws(() => assertResourceCeilings({ ...safe, disk: { ...safe.disk, availableBytes: 1_023 * 1024 ** 2 } }, limits), /disk floor/)
  for (const invalid of [Number.NaN, Number.POSITIVE_INFINITY, Number.NEGATIVE_INFINITY]) {
    assert.throws(() => assertResourceCeilings({ ...safe, owned: { ...safe.owned, cpuPercent: invalid } }, limits), /finite/)
  }
})

test("synthetic auth material is forbidden from retained evidence", () => {
  assert.equal(assertRetainedEvidenceRedacted(["status ok", "marker digest abc"], ["cookie-value", "marker-value"]), true)
  assert.throws(() => assertRetainedEvidenceRedacted(["cookie=cookie-value"], ["cookie-value"]), /leak/)
  assert.throws(() => assertRetainedEvidenceRedacted(["CHARIOX_IDLE_SOAK_SYNTHETIC_SECRET_partial"], []), /synthetic secret marker/)
})

test("completed results require monotonic duration, cadence coverage, final health, restart persistence, resources, provenance, and cleanup", () => {
  const requiredCheckpoints = minimumIdleSoakCheckpointCount(15, 2)
  const entries = [
    { label: "initial", elapsedMonotonicMs: 0 },
    { label: "periodic", elapsedMonotonicMs: 2_000 },
    { label: "periodic", elapsedMonotonicMs: 4_000 },
    { label: "periodic", elapsedMonotonicMs: 6_000 },
    { label: "restart", elapsedMonotonicMs: 7_500 },
    { label: "periodic", elapsedMonotonicMs: 8_000 },
    { label: "periodic", elapsedMonotonicMs: 10_000 },
    { label: "periodic", elapsedMonotonicMs: 12_000 },
    { label: "periodic", elapsedMonotonicMs: 14_000 },
    { label: "final", elapsedMonotonicMs: 15_000, controllerReady: true, controllerPid: 123, browserPid: 124,
      browserAlive: true, freshAuthenticatedRequest: true, profileMarkerObserved: true },
  ].map((entry, index) => ({ ...entry, controllerReady: true, controllerPid: 123, controllerStartTime: "100",
    browserPid: index < 4 ? 124 : 125, browserStartTime: index < 4 ? "200" : "300", browserAlive: true,
    freshAuthenticatedRequest: true, profileMarkerObserved: true }))
  assert.equal(entries.length, requiredCheckpoints)
  const result = {
    schema: "chariox.idle_authenticated_browser_soak.v1",
    status: "passed",
    durationSeconds: 15,
    elapsedMonotonicMs: 15_000,
    healthIntervalSeconds: 2,
    counters: { healthChecks: requiredCheckpoints, controllerRequests: requiredCheckpoints * 3, authenticatedSessionChecks: requiredCheckpoints, profileMarkerChecks: requiredCheckpoints, freshAuthenticatedRequests: requiredCheckpoints },
    checkpoints: { count: requiredCheckpoints, required: requiredCheckpoints, entries, final: entries.at(-1) },
    resources: { sampleCount: 2, peakOwnedRssBytes: 1, peakOwnedCpuPercent: 0, ceilingsRespected: true, baseline: { owned: { processCount: 1, rssBytes: 1, cpuPercent: 0 }, disk: { availableBytes: 1, totalBytes: 2 }, host: { totalMemoryBytes: 1, freeMemoryBytes: 1, loadAverage: [0, 0, 0] } }, final: { owned: { processCount: 1, rssBytes: 1, cpuPercent: 0 }, disk: { availableBytes: 1, totalBytes: 2 }, host: { totalMemoryBytes: 1, freeMemoryBytes: 1, loadAverage: [0, 0, 0] } } },
    source: { commit: "a".repeat(40), branch: "codex/idle-authenticated-browser-soak", dirty: false },
    image: { available: true, imageDigest: "sha256:fixture", sourceCommit: "a".repeat(40) },
    profile: { markerDigest: "b".repeat(64), checks: requiredCheckpoints, browserObserved: true, cookiePersistedAfterRestart: true, controlledRestartCompleted: true },
    redaction: { passed: true },
    cleanup: { clean: true, remainingPids: [], remainingListeners: [], stateRemoved: true, debugPortReleased: true },
  }
  assert.deepEqual(validateCompletedIdleSoakResult(result), result)
  assert.throws(() => validateCompletedIdleSoakResult({ ...result, cleanup: { clean: false } }), /cleanup/)
  assert.throws(() => validateCompletedIdleSoakResult({ ...result, elapsedMonotonicMs: 14_999 }), /monotonic duration/)
  assert.throws(() => validateCompletedIdleSoakResult({ ...result, checkpoints: { ...result.checkpoints, count: requiredCheckpoints - 1 } }), /checkpoint coverage/)
  assert.throws(() => validateCompletedIdleSoakResult({ ...result, checkpoints: { ...result.checkpoints, final: { ...result.checkpoints.final, browserAlive: false } } }), /final health/)
  const suspended = entries.map((entry, index) => index >= 3 ? { ...entry, elapsedMonotonicMs: entry.elapsedMonotonicMs + 60_000 } : entry)
  assert.throws(() => validateCompletedIdleSoakResult({ ...result, checkpoints: { ...result.checkpoints, entries: suspended, final: suspended.at(-1) }, elapsedMonotonicMs: 75_000 }), /checkpoint cadence/)
  assert.throws(() => validateCompletedIdleSoakResult({ ...result, profile: { ...result.profile, cookiePersistedAfterRestart: false } }), /restart persistence/)
})

test("clean dedicated run directories reject retained files and remove only stale failure markers for detached continuation", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-idle-run-dir-"))
  try {
    const clean = path.join(root, "clean")
    await assertCleanIdleSoakRunDirectory(clean)
    await writeFile(path.join(clean, "unexpected"), "x")
    await assert.rejects(assertCleanIdleSoakRunDirectory(clean), /not clean/)
    const detached = path.join(root, "detached")
    await mkdir(detached)
    await writeFile(path.join(detached, "failure.json"), "stale")
    await writeFile(path.join(detached, "preflight.json"), "{}")
    await assertCleanIdleSoakRunDirectory(detached, { detachedContinuation: true })
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("detach contract requires passed same-source smoke and preflight before child start", () => {
  const source = { commit: "c".repeat(40), dirty: false }
  const contract = { source, preflight: { status: "passed", source }, smoke: { status: "passed", source, smoke: true }, childPid: 321 }
  assert.deepEqual(validateIdleSoakDetachContract(contract, { source, childPid: 321 }), contract)
  assert.throws(() => validateIdleSoakDetachContract({ ...contract, smoke: { ...contract.smoke, source: { ...source, commit: "d".repeat(40) } } }, { source, childPid: 321 }), /same-source smoke/)
  assert.throws(() => validateIdleSoakDetachContract({ ...contract, childPid: 322 }, { source, childPid: 321 }), /child pid/)
})

test("detached child waits through the parent status race for its exact contract", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-idle-detach-race-"))
  const paths = buildIdleSoakPaths(root, "run")
  const source = { commit: "e".repeat(40), dirty: false }
  const contract = { source, preflight: { status: "passed", source }, smoke: { status: "passed", source, smoke: true }, childPid: process.pid }
  await mkdir(paths.runDir)
  const timer = setTimeout(() => { void writeFile(paths.detachContract, JSON.stringify(contract)) }, 25)
  try {
    assert.deepEqual(await waitForDetachContract(paths, source, process.pid), contract)
  } finally {
    clearTimeout(timer)
    await rm(root, { recursive: true, force: true })
  }
})

test("loopback fixture advances only fresh authenticated requests and binds restart proof to browser storage", async () => {
  const secret = "CHARIOX_IDLE_SOAK_SYNTHETIC_SECRET_test-only"
  const markerDigest = "f".repeat(64)
  const fixture = await startFixtureServer(secret, markerDigest, 2_000)
  try {
    const login = await fetch(fixture.loginUrl, { redirect: "manual" })
    const cookie = login.headers.get("set-cookie").split(";", 1)[0]
    assert.match(login.headers.get("location"), /bootstrap=/)
    const page = await fetch(fixture.authenticatedUrl, { headers: { cookie } })
    const html = await page.text()
    assert.match(html, /localStorage\.getItem\("chariox-idle-profile-marker"\)/)
    assert.match(html, new RegExp(markerDigest))
    assert.equal(fixture.authenticatedRequests, 0)
    const checkpoint = await fetch(`${fixture.baseUrl}/checkpoint`, { headers: { cookie } })
    assert.equal(checkpoint.status, 200)
    assert.equal(fixture.authenticatedRequests, 1)
  } finally {
    await fixture.close()
  }
})

test("controller lifecycle fails closed on malformed async output and validates exact ready PID", () => {
  assert.deepEqual(parseControllerResponseLine('{"id":1,"ok":true,"result":{}}'), { id: 1, ok: true, result: {} })
  assert.throws(() => parseControllerResponseLine("not-json"), /invalid JSON/)
  assert.deepEqual(assertControllerReady({ state: "ready", process_id: 77 }, 77), { state: "ready", process_id: 77 })
  assert.throws(() => assertControllerReady({ state: "ready", process_id: 78 }, 77), /PID/)
})

test("an asynchronous controller parser failure rejects the active request and remains terminal", async () => {
  const child = spawn(process.execPath, ["-e", "process.stdin.once('data',()=>process.stdout.write('not-json\\n'));setInterval(()=>{},1000)"], {
    detached: true, stdio: ["pipe", "pipe", "ignore"],
  })
  const identity = await captureProcessIdentity(child.pid)
  try {
    const client = new ControllerClient(child)
    await assert.rejects(client.request("health"), /invalid JSON/)
    await assert.rejects(client.request("health"), /invalid JSON/)
  } finally {
    const outcome = await terminateOwnedTree("malformed-controller", identity)
    assert.equal(outcome.ok, true)
  }
})

test("an owned child is recorded before readiness and readiness failure remains terminal", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-idle-owned-readiness-"))
  const events = []
  let child
  try {
    await assert.rejects(launchOwnedProcess({
      name: "readiness-fixture",
      start: () => {
        child = spawnLogged("readiness-fixture", process.execPath, ["-e", "setInterval(()=>{},1000)"], { cwd: root, logsRoot: root })
        return child
      },
      record: async identity => events.push(["record", identity.pid]),
      ready: async () => { events.push(["ready", child.pid]); throw new Error("readiness failed") },
    }), /readiness failed/)
    assert.deepEqual(events, [["record", child.pid], ["ready", child.pid]])
    assert.match((await child.terminalFailure).message, /readiness failed/)
  } finally {
    if (child?.pid) await terminateOwnedTree("readiness-fixture", await captureProcessIdentity(child.pid).catch(() => null))
    await rm(root, { recursive: true, force: true })
  }
})

test("child and log-stream errors become one terminal lifecycle failure", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-idle-log-failure-"))
  const missingLogs = path.join(root, "missing")
  let child
  try {
    child = spawnLogged("log-failure", process.execPath, ["-e", "setInterval(()=>{},1000)"], { cwd: root, logsRoot: missingLogs })
    const error = await child.terminalFailure
    assert.match(error.message, /log-failure.*log/i)
  } finally {
    if (child?.pid) await terminateOwnedTree("log-failure", await captureProcessIdentity(child.pid).catch(() => null))
    await rm(root, { recursive: true, force: true })
  }
})

test("spawn errors use the same terminal process lifecycle channel", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-idle-spawn-failure-"))
  try {
    const child = spawnLogged("spawn-failure", path.join(root, "missing-command"), [], { cwd: root, logsRoot: root })
    assert.match((await child.terminalFailure).message, /spawn-failure process error/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("completed log close clears its losing timeout", async () => {
  const before = process.getActiveResourcesInfo().filter(name => name === "Timeout").length
  await waitForLogClosed({ logClosed: Promise.resolve() })
  await new Promise(resolve => setImmediate(resolve))
  const after = process.getActiveResourcesInfo().filter(name => name === "Timeout").length
  assert.equal(after, before)
})

test("PID reuse never matches an owned process identity", () => {
  const owned = { pid: 44, startTime: "100", pgid: 44 }
  assert.equal(processIdentityMatches(owned, { pid: 44, startTime: "100", pgid: 44 }), true)
  assert.equal(processIdentityMatches(owned, { pid: 44, startTime: "101", pgid: 44 }), false)
})

test("exact owned process-tree cleanup terminates descendants without broad process-group killing", async () => {
  const child = spawn(process.execPath, ["-e", "const c=require('child_process').spawn(process.execPath,['-e','setInterval(()=>{},1000)']);console.log(c.pid);setInterval(()=>{},1000)"], {
    detached: true, stdio: ["ignore", "pipe", "ignore"],
  })
  await new Promise((resolve, reject) => { child.stdout.once("data", resolve); child.once("error", reject) })
  const identity = await captureProcessIdentity(child.pid)
  const outcome = await terminateOwnedTree("test-tree", identity)
  assert.equal(outcome.ok, true)
  assert.equal(outcome.survivors.length, 0)
  assert.ok(outcome.pids.includes(child.pid))
  assert.ok(outcome.pids.length >= 2)
})

test("cleanup finds a descendant after its recorded parent exits and preserves an unrelated process", async () => {
  const root = spawn(process.execPath, ["-e", "const {spawn}=require('node:child_process');const c=spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'});console.log(c.pid);setTimeout(()=>process.exit(0),100)"], {
    detached: true, stdio: ["ignore", "pipe", "ignore"],
  })
  const unrelated = spawn(process.execPath, ["-e", "setInterval(()=>{},1000)"], { detached: true, stdio: "ignore" })
  const rootIdentity = await captureProcessIdentity(root.pid)
  const unrelatedIdentity = await captureProcessIdentity(unrelated.pid)
  const descendantPid = Number(await new Promise((resolve, reject) => {
    root.stdout.once("data", chunk => resolve(String(chunk).trim()))
    root.once("error", reject)
  }))
  await new Promise(resolve => root.once("exit", resolve))
  try {
    const outcome = await terminateOwnedTree("escaped-descendant", rootIdentity)
    assert.equal(outcome.ok, true)
    assert.ok(outcome.pids.includes(descendantPid))
    assert.equal(processIdentityMatches(unrelatedIdentity, await captureProcessIdentity(unrelated.pid)), true)
  } finally {
    await terminateOwnedTree("unrelated", unrelatedIdentity)
  }
})

test("terminal cleanup removes the disposable profile, closes listeners, and leaves no recursive process leak", async () => {
  const stateRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-idle-cleanup-fixture-"))
  await mkdir(path.join(stateRoot, "chromium-profile"))
  const fixture = await startFixtureServer("fixture-secret", "a".repeat(64), 1_000)
  const child = spawn(process.execPath, ["-e", "const {spawn}=require('node:child_process');spawn(process.execPath,['-e','setInterval(()=>{},1000)'],{stdio:'ignore'});setInterval(()=>{},1000)"], {
    detached: true, stdio: "ignore",
  })
  const identity = await captureProcessIdentity(child.pid)
  const cleanup = await cleanupOwned({
    ownedProcesses: [{ name: "fixture-tree", ...identity }], fixture, stateRoot,
    allocation: { debugPort: fixture.port },
  })
  assert.equal(cleanup.clean, true)
  assert.deepEqual(cleanup.remainingPids, [])
  assert.deepEqual(cleanup.remainingListeners, [])
  assert.equal(cleanup.stateRemoved, true)
  await assert.rejects(readFile(stateRoot), /ENOENT|EISDIR/)
  for (const action of cleanup.actions) {
    for (const pid of action.pids ?? []) await assert.rejects(captureProcessIdentity(pid))
  }
})

test("detached launch failures publish terminal status and a failure marker", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-idle-detached-terminal-"))
  const paths = buildIdleSoakPaths(root, "run")
  await mkdir(paths.runDir)
  try {
    await materializeLaunchFailure(paths, { mode: "detach" }, { commit: "b".repeat(40), dirty: false }, new Error("detached fixture failed"))
    const status = JSON.parse(await readFile(paths.status, "utf8"))
    const failure = JSON.parse(await readFile(paths.failure, "utf8"))
    const cleanup = JSON.parse(await readFile(paths.cleanup, "utf8"))
    assert.equal(status.status, "failed")
    assert.equal(status.failureMarker, paths.failure)
    assert.match(failure.marker, /detached fixture failed/)
    assert.equal(cleanup.clean, true)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("setup failures materialize terminal failure status and cleanup evidence", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-idle-setup-failure-"))
  const invalidRepo = path.join(root, "not-a-repository")
  const evidenceRoot = path.join(root, "evidence")
  await mkdir(invalidRepo)
  const options = parseIdleAuthenticatedBrowserSoakArgs(["--preflight", "--evidence-root", evidenceRoot], { repoRoot: invalidRepo, homeDir: root })
  try {
    await assert.rejects(runIdleAuthenticatedBrowserSoak({ options, repoRoot: invalidRepo, scriptPath: "/unused" }))
    const [runDir] = await readdir(evidenceRoot)
    const status = JSON.parse(await readFile(path.join(evidenceRoot, runDir, "status.json"), "utf8"))
    const cleanup = JSON.parse(await readFile(path.join(evidenceRoot, runDir, "cleanup-ledger.json"), "utf8"))
    assert.equal(status.status, "failed")
    assert.equal(status.cleanupReady, true)
    assert.equal(cleanup.clean, true)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("terminal evidence scan includes final result, failure, status, and runner log", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-idle-terminal-evidence-"))
  try {
    const paths = buildIdleSoakPaths(root, "run")
    await mkdir(paths.runDir)
    for (const file of [paths.result, paths.failure, paths.status, paths.log]) await writeFile(file, "safe")
    await validateTerminalIdleSoakEvidence(paths, [])
    for (const file of [paths.result, paths.failure, paths.status, paths.log]) {
      await writeFile(file, "CHARIOX_IDLE_SOAK_SYNTHETIC_SECRET_leak")
      await assert.rejects(validateTerminalIdleSoakEvidence(paths, []), /synthetic secret marker/)
      await writeFile(file, "safe")
    }
    const nested = path.join(paths.runDir, "process-logs")
    await mkdir(nested)
    await writeFile(path.join(nested, "late.log"), "CHARIOX_IDLE_SOAK_SYNTHETIC_SECRET_late")
    await assert.rejects(validateTerminalIdleSoakEvidence(paths, []), /synthetic secret marker/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
