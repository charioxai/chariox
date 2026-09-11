import { appendFile, mkdir, mkdtemp, open, readFile, readdir, rename, rm, stat, statfs, writeFile } from "node:fs/promises"
import { createWriteStream } from "node:fs"
import { createHash, randomBytes } from "node:crypto"
import { execFile, spawn } from "node:child_process"
import http from "node:http"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { promisify } from "node:util"

import {
  assertCheckpointAdvanced,
  assertCleanIdleSoakRunDirectory,
  assertResourceCeilings,
  assertRetainedEvidenceRedacted,
  buildIdleSoakPaths,
  IDLE_SOAK_SYNTHETIC_SECRET_MARKER,
  minimumIdleSoakCheckpointCount,
  validateIdleSoakDetachContract,
  validateCompletedIdleSoakResult,
} from "./idle-authenticated-browser-soak.mjs"

const execFileAsync = promisify(execFile)
const schema = "chariox.idle_authenticated_browser_soak.v1"

export async function runIdleAuthenticatedBrowserSoak({ options, repoRoot, scriptPath }) {
  const runId = new Date().toISOString().replace(/[:.]/g, "-")
  const paths = options.runDir
    ? buildIdleSoakPaths(path.dirname(options.runDir), path.basename(options.runDir))
    : buildIdleSoakPaths(options.evidenceRoot, runId)
  let source
  let detachedChildIdentity
  try {
    await mkdir(options.evidenceRoot, { recursive: true, mode: 0o700 })
    await assertCleanIdleSoakRunDirectory(paths.runDir, { detachedContinuation: options.internalRun })
    source = await sourceIdentity(repoRoot)
    const allocation = { debugPort: options.debugPort ?? await availablePort(52_000, 55_000) }
    if (options.internalRun) {
      const contract = await waitForDetachContract(paths, source, process.pid)
      const preflight = contract.preflight
      await executeSoak({ options, paths, allocation, source, image: preflight.image, baseline: preflight.baseline, repoRoot })
      return
    }
    const image = await imageIdentity()
    const baseline = await resourceSnapshot("preflight", [process.pid], paths.runDir)
    const preflight = await runPreflight({ options, paths, allocation, source, image, baseline, repoRoot })
    await writeJson(paths.preflight, preflight)
    if (options.mode === "preflight") {
      console.log(JSON.stringify({ status: "passed", runDir: paths.runDir, preflight: paths.preflight }))
      return
    }
    if (options.mode === "detach") {
      const smoke = await findSameSourceSmoke(options.evidenceRoot, paths.runDir, source)
      const descriptor = await open(paths.log, "a", 0o600)
      const child = spawn(process.execPath, [scriptPath, ...detachedArgs(options, paths, allocation)], {
        cwd: repoRoot, env: process.env, detached: true, stdio: ["ignore", descriptor.fd, descriptor.fd],
      })
      detachedChildIdentity = await captureProcessIdentity(child.pid)
      child.unref()
      await descriptor.close()
      await writeFile(paths.pid, `${child.pid}\n`, { mode: 0o600 })
      await writeJson(paths.status, {
        schema, status: "starting", pid: child.pid, startedAt: new Date().toISOString(), runDir: paths.runDir,
        processIdentity: detachedChildIdentity,
        command: commandEvidence(options, paths, allocation),
      })
      await writeJson(paths.detachContract, { schema: "chariox.idle_authenticated_browser_soak_detach.v1", childPid: child.pid, source, preflight, smoke })
      console.log(JSON.stringify({ status: "started", pid: child.pid, runDir: paths.runDir, statusPath: paths.status }))
      return
    }
    await executeSoak({ options, paths, allocation, source, image, baseline, repoRoot })
  } catch (error) {
    await materializeLaunchFailure(paths, options, source, error, detachedChildIdentity)
    throw error
  }
}

async function runPreflight({ options, paths, allocation, source, image, baseline, repoRoot }) {
  if (process.getuid?.() === 0) throw new Error("preflight refuses root Chromium; run as the non-root disposable slice owner")
  if (!/^[0-9a-f]{40}$/i.test(source.commit) || !source.branch || source.dirty) throw new Error("preflight requires an exact clean branch source identity")
  if (image.available !== true || !image.imageDigest || image.sourceCommit !== source.commit) {
    throw new Error("preflight requires a signed image manifest matching the source commit")
  }
  await assertNoOwnedRun(options.evidenceRoot, paths.runDir)
  const commandPaths = {}
  for (const command of ["chromium", "ps"]) {
    commandPaths[command] = (await execFileAsync("which", [command], { timeout: 5_000 })).stdout.trim()
  }
  for (const file of ["browser-controller.mjs", "browser-controller-cdp.mjs"]) {
    await stat(path.join(repoRoot, "apps", "kernel", "slice-linux-docker", "docker", file))
  }
  if (!await portAvailable(allocation.debugPort)) throw new Error(`debug port ${allocation.debugPort} is already in use`)
  assertResourceCeilings(baseline, options)
  return {
    schema: "chariox.idle_authenticated_browser_soak_preflight.v1", status: "passed", at: new Date().toISOString(),
    syntheticFixture: true, externalNetworkRequired: false, idleAuthenticated: true,
    durationSeconds: options.durationSeconds, limits: limits(options), allocation, source, image, baseline,
    commandPaths, paths,
  }
}

async function executeSoak({ options, paths, allocation, source, image, baseline, repoRoot }) {
  const startedAt = new Date().toISOString()
  let soakStartedMonotonic = null
  let stateRoot
  let profileRoot
  let logsRoot
  let markerPath
  let markerDigest
  const forbidden = [IDLE_SOAK_SYNTHETIC_SECRET_MARKER]
  let fixture
  let chromium
  let chromiumIdentity
  let controller
  let controllerIdentity
  const ownedProcesses = []
  let interrupted
  let status = "running"
  let failure = null
  let previousCounters = null
  let counters = { healthChecks: 0, controllerRequests: 0, authenticatedSessionChecks: 0, profileMarkerChecks: 0, freshAuthenticatedRequests: 0 }
  const checkpoints = []
  let sampleCount = 0
  let peakOwnedRssBytes = 0
  let peakOwnedCpuPercent = 0
  let finalResource = null
  let controlledRestartCompleted = false
  let cookiePersistedAfterRestart = false
  let browserObserved = false
  const signalHandler = signal => { interrupted ??= signal }
  process.once("SIGINT", signalHandler)
  process.once("SIGTERM", signalHandler)

  try {
    await rm(paths.failure, { force: true })
    stateRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-idle-auth-soak-"))
    profileRoot = path.join(stateRoot, "chromium-profile")
    logsRoot = path.join(paths.runDir, "process-logs")
    await Promise.all([mkdir(profileRoot, { recursive: true, mode: 0o700 }), mkdir(logsRoot, { recursive: true, mode: 0o700 })])
    await writeFile(paths.pid, `${process.pid}\n`, { mode: 0o600 })
    await writeJson(paths.status, { schema, status: "starting", pid: process.pid, startedAt, runDir: paths.runDir,
      processIdentity: await captureProcessIdentity(process.pid), cleanupReady: false })
    const sessionSecret = `${IDLE_SOAK_SYNTHETIC_SECRET_MARKER}${randomBytes(32).toString("hex")}`
    const markerSecret = `${IDLE_SOAK_SYNTHETIC_SECRET_MARKER}${randomBytes(32).toString("hex")}`
    forbidden.push(sessionSecret, markerSecret)
    markerPath = path.join(profileRoot, ".chariox-synthetic-auth-profile-marker")
    await writeFile(markerPath, markerSecret, { mode: 0o600 })
    markerDigest = digest(markerSecret)
    fixture = await startFixtureServer(sessionSecret, markerDigest, options.healthIntervalSeconds * 1_000)
    chromium = spawnSoakChromium({ profileRoot, debugPort: allocation.debugPort, url: fixture.loginUrl, cwd: repoRoot, logsRoot })
    await waitForHttp(`http://127.0.0.1:${allocation.debugPort}/json/version`, 20_000)
    chromiumIdentity = await captureProcessIdentity(chromium.pid)
    ownedProcesses.push({ name: "chromium-initial", ...chromiumIdentity })
    controller = new ControllerClient(spawnProtocol("browser-controller", process.execPath, [
      path.join(repoRoot, "apps", "kernel", "slice-linux-docker", "docker", "browser-controller.mjs"), "stdio",
    ], { cwd: repoRoot, logsRoot, env: { ...process.env, CHARIOX_BROWSER_DEBUGGER_ENDPOINT: `http://127.0.0.1:${allocation.debugPort}` } }))
    controllerIdentity = await captureProcessIdentity(controller.child.pid)
    ownedProcesses.push({ name: "browser-controller", ...controllerIdentity })
    await writeOwnership()
    await writeJson(paths.profile, { schema: "chariox.synthetic_profile_marker.v1", markerDigest, browserStorage: "localStorage", secretRetained: false })
    await checkpoint("initial")
    soakStartedMonotonic = monotonicMs()
    const durationMs = options.durationSeconds * 1_000
    let nextHealth = options.healthIntervalSeconds * 1_000
    let nextSample = options.sampleIntervalSeconds * 1_000
    const restartAt = Math.max(100, Math.floor(durationMs / 2))
    while (monotonicMs() - soakStartedMonotonic < durationMs) {
      if (interrupted) throw new Error(`soak interrupted by ${interrupted}`)
      await assertOwnedAlive(chromium, chromiumIdentity, "chromium")
      await assertOwnedAlive(controller.child, controllerIdentity, "browser-controller")
      const elapsed = monotonicMs() - soakStartedMonotonic
      if (!controlledRestartCompleted && elapsed >= restartAt) {
        const stopped = await terminateOwnedTree("chromium-controlled-restart", chromiumIdentity)
        if (!stopped.ok) throw new Error("controlled Chromium restart could not stop the exact owned tree")
        await waitForLogClosed(chromium)
        await waitForPortAvailable(allocation.debugPort, 5_000)
        chromium = spawnSoakChromium({ profileRoot, debugPort: allocation.debugPort, url: fixture.authenticatedUrl, cwd: repoRoot, logsRoot })
        await waitForHttp(`http://127.0.0.1:${allocation.debugPort}/json/version`, 20_000)
        chromiumIdentity = await captureProcessIdentity(chromium.pid)
        ownedProcesses.push({ name: "chromium-restarted", ...chromiumIdentity })
        await writeOwnership()
        controlledRestartCompleted = true
        const restarted = await checkpoint("restart")
        cookiePersistedAfterRestart = restarted.profileMarkerObserved && restarted.freshAuthenticatedRequest
      }
      if (elapsed >= nextHealth) {
        await checkpoint("periodic")
        nextHealth += options.healthIntervalSeconds * 1_000
      }
      if (elapsed >= nextSample) {
        await sample("periodic")
        nextSample += options.sampleIntervalSeconds * 1_000
      }
      const remaining = durationMs - (monotonicMs() - soakStartedMonotonic)
      await sleep(Math.min(250, Math.max(1, Math.min(nextHealth - elapsed, nextSample - elapsed, remaining))))
    }
    const finalCheckpoint = await checkpoint("final")
    await assertOwnedAlive(chromium, chromiumIdentity, "chromium")
    await assertOwnedAlive(controller.child, controllerIdentity, "browser-controller")
    finalResource = await sample("final")
    const required = minimumIdleSoakCheckpointCount(options.durationSeconds, options.healthIntervalSeconds)
    if (checkpoints.length < required) throw new Error(`checkpoint coverage ${checkpoints.length}/${required} is incomplete`)
    if (!finalCheckpoint.freshAuthenticatedRequest) throw new Error("final authenticated browser checkpoint was not fresh")
    status = "passed"
  } catch (error) {
    status = interrupted ? "interrupted" : "failed"
    failure = bounded(error?.stack ?? error)
  } finally {
    const elapsedMonotonicMs = soakStartedMonotonic == null ? 0 : monotonicMs() - soakStartedMonotonic
    const cleanup = await cleanupOwned({ controller, chromium, ownedProcesses, fixture, stateRoot, allocation,
      requiredTerminalIdentities: status === "passed" ? [controllerIdentity, chromiumIdentity] : [] })
    await writeJson(paths.cleanup, cleanup)
    const result = {
      schema, status, pid: process.pid, startedAt, completedAt: new Date().toISOString(),
      smoke: options.smoke, durationSeconds: options.durationSeconds, healthIntervalSeconds: options.healthIntervalSeconds,
      elapsedMonotonicMs, command: commandEvidence(options, paths, allocation), source, image,
      fixture: { synthetic: true, externalNetworkUsed: false }, counters,
      checkpoints: { count: checkpoints.length, required: minimumIdleSoakCheckpointCount(options.durationSeconds, options.healthIntervalSeconds),
        entries: checkpoints, final: checkpoints.at(-1) ?? null },
      resources: { sampleCount, peakOwnedRssBytes, peakOwnedCpuPercent, ceilingsRespected: status === "passed", limits: limits(options), baseline, final: finalResource },
      profile: { markerDigest, checks: counters.profileMarkerChecks, browserObserved, cookiePersistedAfterRestart, controlledRestartCompleted, secretRetained: false },
      redaction: { passed: false }, cleanup,
    }
    if (failure) result.failure = failure
    await materializeTerminalResult(result)
    try {
      await validateTerminalIdleSoakEvidence(paths, forbidden, logsRoot)
      result.redaction.passed = true
      if (result.status === "passed") validateCompletedIdleSoakResult(result)
    } catch (error) {
      result.status = "failed"
      result.redaction.passed = false
      result.failure = String(error?.message ?? error).includes("synthetic secret marker")
        ? "retained evidence contains a synthetic secret marker"
        : bounded(error)
    }
    await materializeTerminalResult(result)
    if (result.redaction.passed) {
      try { await validateTerminalIdleSoakEvidence(paths, forbidden, logsRoot) } catch (error) {
        result.status = "failed"; result.redaction.passed = false; result.failure = "final retained evidence failed redaction validation"
        await materializeTerminalResult(result)
      }
    }
    process.removeListener("SIGINT", signalHandler)
    process.removeListener("SIGTERM", signalHandler)
    if (!options.internalRun) console.log(JSON.stringify({ status: result.status, pid: process.pid, runDir: paths.runDir, result: paths.result }))
    if (result.status !== "passed") process.exitCode = 1
  }

  async function checkpoint(label) {
    const health = assertControllerReady(await controller.request("health"), controller.child.pid)
    await assertOwnedAlive(controller.child, controllerIdentity, "browser-controller")
    const reconciled = await controller.request("browser.reconcile", { viewport: { css_width: 800, css_height: 600, device_scale_factor: 1 } })
    const tab = reconciled.tabs.find(entry => entry.url?.startsWith(fixture.baseUrl)) ?? reconciled.tabs[0]
    if (!tab) throw new Error("Browser Controller returned no Chromium tab")
    const beforeRequests = counters.authenticatedSessionChecks
    await waitForFreshAuthenticatedRequest(fixture, beforeRequests, Math.min(10_000, options.healthIntervalSeconds * 1_000 + 2_000))
    const snapshot = await controller.request("browser.snapshot", { target_id: tab.target_id, document_id: tab.document_id })
    if (!JSON.stringify(snapshot).includes("Authenticated synthetic session")) throw new Error("synthetic authenticated page was not observable")
    if (!JSON.stringify(snapshot).includes(markerDigest)) throw new Error("Chromium did not observe the persistent profile marker")
    if (fixture.authenticatedRequests <= beforeRequests) throw new Error("synthetic authenticated session did not advance")
    if (digest(await readFile(markerPath, "utf8")) !== markerDigest) throw new Error("persistent profile marker changed")
    const freshCount = fixture.authenticatedRequests - beforeRequests
    counters = {
      healthChecks: counters.healthChecks + 1,
      controllerRequests: controller.requestCount,
      authenticatedSessionChecks: fixture.authenticatedRequests,
      profileMarkerChecks: counters.profileMarkerChecks + 1,
      freshAuthenticatedRequests: counters.freshAuthenticatedRequests + freshCount,
    }
    assertCheckpointAdvanced(previousCounters, counters)
    previousCounters = { ...counters }
    browserObserved = true
    const checkpoint = { label, at: new Date().toISOString(), elapsedMonotonicMs: soakStartedMonotonic == null ? 0 : monotonicMs() - soakStartedMonotonic,
      controllerReady: health.state === "ready", controllerPid: health.process_id, controllerStartTime: controllerIdentity.startTime,
      browserPid: chromium.pid, browserStartTime: chromiumIdentity.startTime,
      browserAlive: true, freshAuthenticatedRequest: freshCount > 0, profileMarkerObserved: true }
    checkpoints.push(checkpoint)
    await appendFile(paths.checkpoints, `${JSON.stringify(checkpoint)}\n`, { mode: 0o600 })
    await sample(label)
    const elapsedMonotonicMs = soakStartedMonotonic == null ? 0 : monotonicMs() - soakStartedMonotonic
    await writeJson(paths.status, { schema, status: "healthy", pid: process.pid, startedAt, updatedAt: new Date().toISOString(),
      elapsedMonotonicMs, elapsedSeconds: Math.floor(elapsedMonotonicMs / 1_000), counters,
      resources: { sampleCount, peakOwnedRssBytes, peakOwnedCpuPercent, limits: limits(options) },
      profile: { markerDigest, persisted: true }, checkpoint, failureMarker: null, cleanupReady: false, runDir: paths.runDir })
    return checkpoint
  }

  async function sample(label) {
    const liveOwned = []
    for (const identity of [controllerIdentity, chromiumIdentity]) {
      const current = identity && await currentProcessIdentity(identity.pid)
      if (current && processIdentityMatches(identity, current)) liveOwned.push(identity.pid)
    }
    const value = await resourceSnapshot(label, [process.pid, ...liveOwned], paths.runDir)
    assertResourceCeilings(value, options)
    sampleCount += 1
    peakOwnedRssBytes = Math.max(peakOwnedRssBytes, value.owned.rssBytes)
    peakOwnedCpuPercent = Math.max(peakOwnedCpuPercent, value.owned.cpuPercent)
    await appendFile(paths.samples, `${JSON.stringify(value)}\n`, { mode: 0o600 })
    return value
  }

  async function writeOwnership() {
    await writeJson(paths.ownership, { schema: "chariox.idle_authenticated_browser_soak_ownership.v1", ownerPid: process.pid,
      processes: [{ name: "runner", ...(await captureProcessIdentity(process.pid)) }, ...ownedProcesses],
      cleanupPolicy: "exact PID/start-time identities and owned descendants only" })
  }

  async function materializeTerminalResult(result) {
    if (result.status === "passed") await rm(paths.failure, { force: true })
    else await writeJson(paths.failure, { schema: "chariox.idle_authenticated_browser_soak_failure.v1", status: result.status,
      at: result.completedAt, marker: result.failure ?? "idle authenticated soak did not pass" })
    await writeJson(paths.result, result)
    await writeJson(paths.status, { schema, status: result.status, pid: process.pid, startedAt, completedAt: result.completedAt,
      elapsedMonotonicMs: result.elapsedMonotonicMs, counters, checkpoints: result.checkpoints, resources: result.resources,
      failureMarker: result.status === "passed" ? null : paths.failure, cleanupReady: result.cleanup.clean, result: paths.result })
  }
}

export class ControllerClient {
  constructor(child) {
    this.child = child; this.requestCount = 0; this.nextId = 0; this.pending = new Map()
    readJsonLines(child.stdout, value => {
      const pending = this.pending.get(value.id)
      if (!pending) return
      this.pending.delete(value.id)
      value.ok ? pending.resolve(value.result) : pending.reject(new Error(`Browser Controller: ${value.error?.message ?? "request failed"}`))
    }, error => {
      for (const pending of this.pending.values()) pending.reject(error)
      this.pending.clear()
      this.protocolError = error
    })
    child.once("exit", () => { for (const pending of this.pending.values()) pending.reject(new Error("Browser Controller exited")); this.pending.clear() })
  }
  request(method, params) {
    if (this.protocolError) return Promise.reject(this.protocolError)
    const id = ++this.nextId; this.requestCount += 1
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`Browser Controller ${method} timed out`)) }, 15_000)
      this.pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value) }, reject: error => { clearTimeout(timer); reject(error) } })
      this.child.stdin.write(`${JSON.stringify({ id, method, params })}\n`)
    })
  }
  async shutdown() {
    if (this.child.exitCode !== null) return
    await this.request("shutdown").catch(() => {})
    this.child.stdin.end()
  }
}

export async function startFixtureServer(sessionSecret, markerDigest, healthIntervalMs) {
  let authenticatedRequests = 0
  const cookie = `chariox_synthetic_session=${sessionSecret}`
  const server = http.createServer((request, response) => {
    if (request.url === "/login") {
      response.writeHead(302, { location: `/authenticated?bootstrap=${markerDigest}`, "set-cookie": `${cookie}; HttpOnly; SameSite=Strict; Path=/; Max-Age=172800`, "cache-control": "no-store" })
      return response.end()
    }
    if (request.url?.startsWith("/checkpoint") && request.headers.cookie?.split(/;\s*/).includes(cookie)) {
      authenticatedRequests += 1
      response.writeHead(200, { "content-type": "application/json", "cache-control": "no-store" })
      return response.end(JSON.stringify({ authenticated: true, request: authenticatedRequests }))
    }
    if (request.url?.startsWith("/authenticated") && request.headers.cookie?.split(/;\s*/).includes(cookie)) {
      const bootstrap = new URL(request.url, "http://fixture.invalid").searchParams.get("bootstrap")
      response.writeHead(200, { "content-type": "text/html", "cache-control": "no-store" })
      return response.end(`<!doctype html><title>Chariox synthetic auth</title><main><h1>Authenticated synthetic session</h1><p id="profile-marker"></p><p id="checkpoint">waiting</p></main><script>
const expected = ${JSON.stringify(markerDigest)};
const bootstrap = ${JSON.stringify(bootstrap)};
if (bootstrap === expected) localStorage.setItem("chariox-idle-profile-marker", expected);
document.querySelector("#profile-marker").textContent = localStorage.getItem("chariox-idle-profile-marker") || "missing";
async function checkpoint() { const response = await fetch("/checkpoint", { cache: "no-store" }); if (response.ok) document.querySelector("#checkpoint").textContent = "fresh-" + (await response.json()).request; }
checkpoint(); setInterval(checkpoint, ${Math.max(1_000, Math.floor(healthIntervalMs * 0.75))});
</script>`)
    }
    response.writeHead(401, { "content-type": "text/plain", "cache-control": "no-store" })
    response.end("unauthenticated")
  })
  await new Promise((resolve, reject) => server.once("error", reject).listen(0, "127.0.0.1", resolve))
  const baseUrl = `http://127.0.0.1:${server.address().port}`
  return { baseUrl, port: server.address().port, loginUrl: `${baseUrl}/login`, authenticatedUrl: `${baseUrl}/authenticated`,
    get authenticatedRequests() { return authenticatedRequests },
    close: () => new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())) }
}

async function cleanupOwned({ controller, chromium, ownedProcesses, fixture, stateRoot, allocation, requiredTerminalIdentities }) {
  const actions = []
  for (const identity of requiredTerminalIdentities) {
    const current = identity && await currentProcessIdentity(identity.pid)
    actions.push({ name: `terminal-process-${identity?.pid ?? "missing"}`, ok: Boolean(current && processIdentityMatches(identity, current)) })
  }
  try { await controller?.shutdown(); actions.push({ name: "controller-shutdown", ok: true }) } catch (error) { actions.push({ name: "controller-shutdown", ok: false, error: bounded(error) }) }
  for (const identity of [...ownedProcesses].reverse()) {
    try { actions.push(await terminateOwnedTree(identity.name, identity)) }
    catch (error) { actions.push({ name: identity.name, ok: false, error: bounded(error), survivors: [identity.pid] }) }
  }
  try { await Promise.all([waitForLogClosed(controller?.child), waitForLogClosed(chromium)]); actions.push({ name: "process-logs-closed", ok: true }) }
  catch (error) { actions.push({ name: "process-logs-closed", ok: false, error: bounded(error) }) }
  try { await fixture?.close(); actions.push({ name: "fixture-close", ok: true }) } catch (error) { actions.push({ name: "fixture-close", ok: false, error: bounded(error) }) }
  try { if (stateRoot) await rm(stateRoot, { recursive: true, force: true }); actions.push({ name: "state-remove", ok: true }) } catch (error) { actions.push({ name: "state-remove", ok: false, error: bounded(error) }) }
  await sleep(250)
  const remainingPids = []
  for (const identity of ownedProcesses) {
    const current = await currentProcessIdentity(identity.pid)
    if (current && processIdentityMatches(identity, current)) remainingPids.push(identity.pid)
  }
  for (const action of actions) for (const pid of action.survivors ?? []) if (!remainingPids.includes(pid)) remainingPids.push(pid)
  const remainingListeners = []
  if (!await portAvailable(allocation.debugPort)) remainingListeners.push({ port: allocation.debugPort, purpose: "chromium-debug" })
  if (fixture?.port && !await portAvailable(fixture.port)) remainingListeners.push({ port: fixture.port, purpose: "synthetic-fixture" })
  return { schema: "chariox.idle_authenticated_browser_soak_cleanup.v1", at: new Date().toISOString(), actions,
    remainingPids, remainingListeners, debugPortReleased: !remainingListeners.some(entry => entry.purpose === "chromium-debug"),
    stateRemoved: !stateRoot || !await exists(stateRoot),
    clean: remainingPids.length === 0 && remainingListeners.length === 0 && (!stateRoot || !await exists(stateRoot)) && actions.every(action => action.ok) }
}

async function resourceSnapshot(label, rootPids, diskPath) {
  const { stdout } = await execFileAsync("ps", ["-eo", "pid=,ppid=,rss=,%cpu=,comm="], { timeout: 10_000 })
  const rows = stdout.split("\n").map(line => line.trim().split(/\s+/, 5)).filter(parts => parts.length === 5)
    .map(([pid, ppid, rss, cpu, command]) => ({ pid: Number(pid), ppid: Number(ppid), rssKb: Number(rss), cpuPercent: Number(cpu), command }))
  const ids = descendantIds(rows, rootPids)
  const owned = rows.filter(row => ids.has(row.pid))
  const disk = await statfs(diskPath)
  return { label, at: new Date().toISOString(), host: { totalMemoryBytes: os.totalmem(), freeMemoryBytes: os.freemem(), loadAverage: os.loadavg() },
    disk: { path: diskPath, availableBytes: Number(disk.bavail) * Number(disk.bsize), totalBytes: Number(disk.blocks) * Number(disk.bsize) },
    owned: { rootPids, processCount: owned.length, rssBytes: owned.reduce((sum, row) => sum + row.rssKb * 1024, 0),
      cpuPercent: owned.reduce((sum, row) => sum + row.cpuPercent, 0), processes: owned } }
}

function descendantIds(rows, roots) {
  const ids = new Set(roots); let changed = true
  while (changed) { changed = false; for (const row of rows) if (ids.has(row.ppid) && !ids.has(row.pid)) { ids.add(row.pid); changed = true } }
  return ids
}

async function assertNoOwnedRun(evidenceRoot, currentRunDir) {
  for (const entry of await readdir(evidenceRoot, { withFileTypes: true }).catch(() => [])) {
    if (!entry.isDirectory()) continue
    const runDir = path.join(evidenceRoot, entry.name)
    if (runDir === currentRunDir) continue
    const pid = Number((await readFile(path.join(runDir, "runner.pid"), "utf8").catch(() => "")).trim())
    const status = JSON.parse(await readFile(path.join(runDir, "status.json"), "utf8").catch(() => "{}"))
    const ownership = JSON.parse(await readFile(path.join(runDir, "process-ownership.json"), "utf8").catch(() => "{}"))
    const expected = status.processIdentity ?? ownership.processes?.find(entry => entry.name === "runner")
    const current = Number.isSafeInteger(pid) ? await currentProcessIdentity(pid) : null
    if (current && processIdentityMatches(expected, current) && ["starting", "running", "healthy"].includes(status.status)) {
      throw new Error(`owned idle soak already active at ${runDir} with pid ${pid}`)
    }
  }
}

async function assertEvidenceRedacted(paths, forbidden, _logsRoot, { terminal = false } = {}) {
  if (terminal) {
    for (const required of [paths.status, paths.result]) await stat(required)
  }
  const texts = await retainedEvidenceTexts(paths.runDir)
  assertRetainedEvidenceRedacted(texts, forbidden)
}

async function retainedEvidenceTexts(runDir) {
  const texts = []
  const pending = [runDir]
  let totalBytes = 0
  while (pending.length > 0) {
    const directory = pending.pop()
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const candidate = path.join(directory, entry.name)
      if (entry.isSymbolicLink()) throw new Error("retained evidence must not contain symbolic links")
      if (entry.isDirectory()) { pending.push(candidate); continue }
      if (!entry.isFile()) throw new Error("retained evidence contains an unsupported filesystem entry")
      const metadata = await stat(candidate)
      if (metadata.size > 16 * 1024 ** 2) throw new Error("retained evidence file exceeds the 16 MiB scan bound")
      totalBytes += metadata.size
      if (totalBytes > 64 * 1024 ** 2) throw new Error("retained evidence exceeds the 64 MiB scan bound")
      texts.push(await readFile(candidate, "utf8"))
    }
  }
  return texts
}

async function sourceIdentity(repoRoot) {
  const [{ stdout: commit }, { stdout: branch }, { stdout: status }] = await Promise.all([
    execFileAsync("git", ["rev-parse", "HEAD"], { cwd: repoRoot }), execFileAsync("git", ["branch", "--show-current"], { cwd: repoRoot }),
    execFileAsync("git", ["status", "--short"], { cwd: repoRoot }),
  ])
  return { commit: commit.trim(), branch: branch.trim(), dirty: status.trim() !== "" }
}

async function imageIdentity() {
  for (const candidate of ["/etc/chariox/release-manifest.json", "/usr/lib/chariox/current/usr/lib/chariox/release-manifest.json"]) {
    try {
      const value = JSON.parse(await readFile(candidate, "utf8"))
      return { manifestPath: candidate, imageDigest: value.image_digest ?? value.imageDigest ?? null,
        sourceCommit: value.source_commit ?? value.sourceCommit ?? value.git_sha ?? null, available: true }
    } catch {}
  }
  return { available: false, reason: "release manifest not present" }
}

function detachedArgs(options, paths, allocation) {
  return ["--internal-run", ...(options.smoke ? ["--smoke"] : ["--duration-seconds", String(options.durationSeconds),
    "--health-interval-seconds", String(options.healthIntervalSeconds), "--sample-interval-seconds", String(options.sampleIntervalSeconds)]),
    "--max-cpu-percent", String(options.maxCpuPercent), "--max-rss-mb", String(options.maxRssMb),
    "--max-processes", String(options.maxProcesses), "--min-free-disk-mb", String(options.minFreeDiskMb),
    "--evidence-root", options.evidenceRoot, "--run-dir", paths.runDir, "--debug-port", String(allocation.debugPort)]
}

function commandEvidence(options, paths, allocation) {
  return `node apps/cli/scripts/live-idle-authenticated-browser-soak.mjs --detach --duration-seconds ${options.durationSeconds} --health-interval-seconds ${options.healthIntervalSeconds} --sample-interval-seconds ${options.sampleIntervalSeconds} --max-cpu-percent ${options.maxCpuPercent} --max-rss-mb ${options.maxRssMb} --max-processes ${options.maxProcesses} --min-free-disk-mb ${options.minFreeDiskMb} --evidence-root ${JSON.stringify(options.evidenceRoot)} --run-dir ${JSON.stringify(paths.runDir)} --debug-port ${allocation.debugPort}`
}

function spawnLogged(name, command, args, { cwd, logsRoot, env = process.env }) {
  const log = createWriteStream(path.join(logsRoot, `${name}.log`), { flags: "a", mode: 0o600 })
  const child = spawn(command, args, { cwd, env, detached: true, stdio: ["ignore", "pipe", "pipe"] })
  child.logClosed = new Promise(resolve => log.once("close", resolve))
  child.stdout.pipe(log, { end: false }); child.stderr.pipe(log, { end: false }); child.once("exit", () => log.end())
  return child
}

function spawnProtocol(name, command, args, { cwd, logsRoot, env }) {
  const log = createWriteStream(path.join(logsRoot, `${name}.stderr.log`), { flags: "a", mode: 0o600 })
  const child = spawn(command, args, { cwd, env, detached: true, stdio: ["pipe", "pipe", "pipe"] })
  child.logClosed = new Promise(resolve => log.once("close", resolve))
  child.stderr.pipe(log); child.once("exit", () => log.end()); return child
}

function readJsonLines(stream, consume, fail) {
  let buffer = ""; stream.setEncoding("utf8")
  stream.on("data", chunk => { buffer += chunk; for (;;) { const index = buffer.indexOf("\n"); if (index < 0) break
    const line = buffer.slice(0, index).trim(); buffer = buffer.slice(index + 1); if (line) {
      try { consume(parseControllerResponseLine(line)) } catch (error) { fail(error); return }
    } } })
}

export function parseControllerResponseLine(line) {
  try {
    const value = JSON.parse(line)
    if (!value || !Number.isSafeInteger(value.id)) throw new Error("response id is invalid")
    return value
  } catch (error) {
    throw new Error(`Browser Controller emitted invalid JSON: ${bounded(error)}`)
  }
}

export function assertControllerReady(health, expectedPid) {
  if (health?.state !== "ready" || health?.process_id !== expectedPid) {
    throw new Error(`Browser Controller ready state/PID mismatch`)
  }
  return health
}

export function processIdentityMatches(expected, actual) {
  return Number.isSafeInteger(expected?.pid) && expected.pid === actual?.pid
    && String(expected.startTime) === String(actual?.startTime) && expected.pgid === actual?.pgid
}

export async function validateTerminalIdleSoakEvidence(paths, forbidden, logsRoot) {
  await assertEvidenceRedacted(paths, forbidden, logsRoot, { terminal: true })
  return true
}

function spawnSoakChromium({ profileRoot, debugPort, url, cwd, logsRoot }) {
  return spawnLogged("chromium", "chromium", [
    "--headless=new", `--user-data-dir=${profileRoot}`, "--password-store=basic", "--no-first-run",
    "--no-default-browser-check", "--disable-sync", "--disable-dev-shm-usage", "--disable-gpu",
    "--remote-debugging-address=127.0.0.1", `--remote-debugging-port=${debugPort}`, url,
  ], { cwd, logsRoot })
}

export async function captureProcessIdentity(pid) {
  if (!Number.isSafeInteger(pid) || pid <= 0) throw new Error("process identity requires a positive PID")
  const value = await readFile(`/proc/${pid}/stat`, "utf8")
  const close = value.lastIndexOf(")")
  if (close < 0) throw new Error(`cannot parse process identity for ${pid}`)
  const fields = value.slice(close + 2).trim().split(/\s+/)
  const identity = { pid, state: fields[0], ppid: Number(fields[1]), pgid: Number(fields[2]), startTime: fields[19] }
  if (!Number.isSafeInteger(identity.ppid) || !Number.isSafeInteger(identity.pgid) || !/^\d+$/.test(identity.startTime ?? "")) {
    throw new Error(`cannot parse process identity for ${pid}`)
  }
  return identity
}

async function currentProcessIdentity(pid) {
  try { const identity = await captureProcessIdentity(pid); return identity.state === "Z" ? null : identity } catch { return null }
}

async function assertOwnedAlive(child, identity, name) {
  const current = await currentProcessIdentity(identity?.pid)
  if (!child || child.exitCode !== null || !current || !processIdentityMatches(identity, current)) {
    throw new Error(`${name} exact owned process exited during soak`)
  }
}

async function ownedTree(rootIdentity) {
  const { stdout } = await execFileAsync("ps", ["-eo", "pid=,ppid=,pgid="], { timeout: 10_000 })
  const rows = stdout.split("\n").map(line => line.trim().split(/\s+/, 3).map(Number))
    .filter(([pid, ppid, pgid]) => [pid, ppid, pgid].every(Number.isSafeInteger))
    .map(([pid, ppid, pgid]) => ({ pid, ppid, pgid }))
  const ids = descendantIds(rows, [rootIdentity.pid])
  for (const row of rows) if (row.pgid === rootIdentity.pgid) ids.add(row.pid)
  const identities = []
  for (const pid of ids) {
    const identity = await currentProcessIdentity(pid)
    if (identity) identities.push(identity)
  }
  return identities.sort((left, right) => right.pid === rootIdentity.pid ? -1 : left.pid === rootIdentity.pid ? 1 : 0)
}

export async function terminateOwnedTree(name, rootIdentity) {
  const current = await currentProcessIdentity(rootIdentity?.pid)
  if (current && !processIdentityMatches(rootIdentity, current)) return { name, ok: true, pidReused: true, survivors: [] }
  let identities = await ownedTree(rootIdentity)
  if (identities.length === 0) return { name, ok: true, alreadyExited: true, survivors: [] }
  await signalExact(identities, "SIGTERM")
  for (let index = 0; index < 50; index += 1) {
    identities = mergeIdentities(identities, await ownedTree(rootIdentity))
    const alive = await exactSurvivors(identities)
    if (alive.length === 0) return { name, ok: true, forced: false, pids: identities.map(entry => entry.pid), survivors: [] }
    await sleep(100)
  }
  await signalExact(await exactSurvivors(identities), "SIGKILL")
  await sleep(100)
  identities = mergeIdentities(identities, await ownedTree(rootIdentity))
  const survivors = (await exactSurvivors(identities)).map(entry => entry.pid)
  return { name, ok: survivors.length === 0, forced: true, pids: identities.map(entry => entry.pid), survivors }
}

async function signalExact(identities, signal) {
  for (const identity of identities) {
    const current = await currentProcessIdentity(identity.pid)
    if (current && processIdentityMatches(identity, current)) try { process.kill(identity.pid, signal) } catch {}
  }
}

function mergeIdentities(existing, discovered) {
  const merged = new Map(existing.map(identity => [`${identity.pid}:${identity.startTime}`, identity]))
  for (const identity of discovered) merged.set(`${identity.pid}:${identity.startTime}`, identity)
  return [...merged.values()]
}

async function exactSurvivors(identities) {
  const survivors = []
  for (const identity of identities) {
    const current = await currentProcessIdentity(identity.pid)
    if (current && processIdentityMatches(identity, current)) survivors.push(identity)
  }
  return survivors
}

async function waitForFreshAuthenticatedRequest(fixture, before, timeoutMs) {
  const deadline = monotonicMs() + timeoutMs
  while (monotonicMs() < deadline) {
    if (fixture.authenticatedRequests > before) return fixture.authenticatedRequests
    await sleep(50)
  }
  throw new Error("fresh authenticated fixture request did not arrive before checkpoint deadline")
}

export async function waitForDetachContract(paths, source, childPid) {
  const deadline = monotonicMs() + 10_000
  while (monotonicMs() < deadline) {
    try {
      const contract = JSON.parse(await readFile(paths.detachContract, "utf8"))
      return validateIdleSoakDetachContract(contract, { source, childPid })
    } catch (error) {
      if (await exists(paths.detachContract)) throw error
    }
    await sleep(50)
  }
  throw new Error("detached runner did not receive its validated preflight/smoke contract")
}

async function findSameSourceSmoke(evidenceRoot, currentRunDir, source) {
  const candidates = []
  for (const entry of await readdir(evidenceRoot, { withFileTypes: true }).catch(() => [])) {
    if (!entry.isDirectory()) continue
    const runDir = path.join(evidenceRoot, entry.name)
    if (runDir === currentRunDir) continue
    try {
      const resultPath = path.join(runDir, "result.json")
      const result = JSON.parse(await readFile(resultPath, "utf8"))
      if (result.smoke === true && result.status === "passed" && result.source?.commit === source.commit
        && result.source?.dirty === false && source.dirty === false) {
        validateCompletedIdleSoakResult(result)
        const smokePaths = buildIdleSoakPaths(evidenceRoot, entry.name)
        await validateTerminalIdleSoakEvidence(smokePaths, [IDLE_SOAK_SYNTHETIC_SECRET_MARKER], path.join(runDir, "process-logs"))
        candidates.push({ resultPath, completedAt: result.completedAt, result })
      }
    } catch {}
  }
  candidates.sort((left, right) => Date.parse(right.completedAt) - Date.parse(left.completedAt))
  if (!candidates[0]) throw new Error("detach requires a completed same-source smoke result under evidence-root")
  return { status: "passed", smoke: true, source: candidates[0].result.source,
    completedAt: candidates[0].completedAt, resultPath: candidates[0].resultPath }
}

async function materializeLaunchFailure(paths, options, source, error, ownedIdentity) {
  let action = null
  if (ownedIdentity) {
    try { action = await terminateOwnedTree("detached-runner", ownedIdentity) }
    catch (cleanupError) { action = { name: "detached-runner", ok: false, error: bounded(cleanupError), survivors: [ownedIdentity.pid] } }
  }
  const cleanup = { schema: "chariox.idle_authenticated_browser_soak_cleanup.v1", at: new Date().toISOString(),
    actions: action ? [action] : [], remainingPids: action?.survivors ?? [], remainingListeners: [], debugPortReleased: true,
    stateRemoved: true, clean: !action || action.ok }
  const completedAt = new Date().toISOString()
  const failure = bounded(error)
  await writeJson(paths.cleanup, cleanup)
  await writeJson(paths.failure, { schema: "chariox.idle_authenticated_browser_soak_failure.v1", status: "failed", at: completedAt, marker: failure })
  await writeJson(paths.result, { schema, status: "failed", pid: process.pid, completedAt, source: source ?? null,
    command: options ? { mode: options.mode } : null, failure, cleanup, redaction: { passed: true } })
  await writeJson(paths.status, { schema, status: "failed", pid: process.pid, completedAt, failureMarker: paths.failure,
    cleanupReady: cleanup.clean, result: paths.result })
  await validateTerminalIdleSoakEvidence(paths, [IDLE_SOAK_SYNTHETIC_SECRET_MARKER])
}

async function waitForHttp(url, timeoutMs) {
  const deadline = monotonicMs() + timeoutMs
  while (monotonicMs() < deadline) { try { const response = await fetch(url, { signal: AbortSignal.timeout(1_000) }); if (response.ok) return } catch {} await sleep(100) }
  throw new Error(`${url} did not become ready`)
}

async function waitForPortAvailable(port, timeoutMs) {
  const deadline = monotonicMs() + timeoutMs
  while (monotonicMs() < deadline) {
    if (await portAvailable(port)) return
    await sleep(50)
  }
  throw new Error(`owned listener on port ${port} did not terminate`)
}

async function waitForLogClosed(child) {
  if (!child?.logClosed) return
  await Promise.race([child.logClosed, new Promise((_, reject) => setTimeout(() => reject(new Error("owned process log did not close")), 5_000))])
}

async function availablePort(minimum, maximum) {
  for (let port = minimum; port <= maximum; port += 1) if (await portAvailable(port)) return port
  throw new Error(`no loopback port available from ${minimum} to ${maximum}`)
}

function portAvailable(port) {
  return new Promise(resolve => { const server = net.createServer(); server.unref(); server.once("error", () => resolve(false));
    server.listen(port, "127.0.0.1", () => server.close(() => resolve(true))) })
}

async function exists(candidate) { try { await stat(candidate); return true } catch { return false } }
function limits(options) { return { maxCpuPercent: options.maxCpuPercent, maxRssMb: options.maxRssMb, maxProcesses: options.maxProcesses, minFreeDiskMb: options.minFreeDiskMb } }
function digest(value) { return createHash("sha256").update(value).digest("hex") }
function bounded(value) { return String(value?.message ?? value).replace(/[\r\n]+/g, " ").slice(0, 2_000) }
function monotonicMs() { return Number(process.hrtime.bigint() / 1_000_000n) }
function sleep(ms) { return new Promise(resolve => setTimeout(resolve, ms)) }
async function writeJson(file, value) { const temporary = `${file}.tmp-${process.pid}`; await writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 }); await rename(temporary, file) }
