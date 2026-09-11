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
  assertResourceCeilings,
  assertRetainedEvidenceRedacted,
  buildIdleSoakPaths,
  validateCompletedIdleSoakResult,
} from "./idle-authenticated-browser-soak.mjs"

const execFileAsync = promisify(execFile)
const schema = "chariox.idle_authenticated_browser_soak.v1"

export async function runIdleAuthenticatedBrowserSoak({ options, repoRoot, scriptPath }) {
  const runId = new Date().toISOString().replace(/[:.]/g, "-")
  const paths = options.runDir
    ? buildIdleSoakPaths(path.dirname(options.runDir), path.basename(options.runDir))
    : buildIdleSoakPaths(options.evidenceRoot, runId)
  await mkdir(paths.runDir, { recursive: true, mode: 0o700 })
  const allocation = { debugPort: options.debugPort ?? await availablePort(52_000, 55_000) }
  const source = await sourceIdentity(repoRoot)
  const image = await imageIdentity()
  const baseline = await resourceSnapshot("preflight", [process.pid], paths.runDir)
  const preflight = await runPreflight({ options, paths, allocation, source, image, baseline, repoRoot })
  await writeJson(paths.preflight, preflight)
  if (options.mode === "preflight") {
    console.log(JSON.stringify({ status: "passed", runDir: paths.runDir, preflight: paths.preflight }))
    return
  }
  if (options.mode === "detach") {
    const descriptor = await open(paths.log, "a", 0o600)
    const child = spawn(process.execPath, [scriptPath, ...detachedArgs(options, paths, allocation)], {
      cwd: repoRoot, env: process.env, detached: true, stdio: ["ignore", descriptor.fd, descriptor.fd],
    })
    child.unref()
    await descriptor.close()
    await writeFile(paths.pid, `${child.pid}\n`, { mode: 0o600 })
    await writeJson(paths.status, {
      schema, status: "starting", pid: child.pid, startedAt: new Date().toISOString(), runDir: paths.runDir,
      command: commandEvidence(options, paths, allocation),
    })
    console.log(JSON.stringify({ status: "started", pid: child.pid, runDir: paths.runDir, statusPath: paths.status }))
    return
  }
  await executeSoak({ options, paths, allocation, source, image, baseline, repoRoot })
}

async function runPreflight({ options, paths, allocation, source, image, baseline, repoRoot }) {
  if (process.getuid?.() === 0) throw new Error("preflight refuses root Chromium; run as the non-root disposable slice owner")
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
  const stateRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-idle-auth-soak-"))
  const profileRoot = path.join(stateRoot, "chromium-profile")
  const logsRoot = path.join(paths.runDir, "process-logs")
  await Promise.all([mkdir(profileRoot, { recursive: true, mode: 0o700 }), mkdir(logsRoot, { recursive: true, mode: 0o700 })])
  await writeFile(paths.pid, `${process.pid}\n`, { mode: 0o600 })
  const sessionSecret = randomBytes(32).toString("hex")
  const markerSecret = randomBytes(32).toString("hex")
  const forbidden = [sessionSecret, markerSecret]
  const markerPath = path.join(profileRoot, ".chariox-synthetic-auth-profile-marker")
  await writeFile(markerPath, markerSecret, { mode: 0o600 })
  const markerDigest = digest(markerSecret)
  let fixture
  let chromium
  let controller
  let interrupted
  let status = "running"
  let failure = null
  let previousCounters = null
  let counters = { healthChecks: 0, controllerRequests: 0, authenticatedSessionChecks: 0, profileMarkerChecks: 0 }
  let sampleCount = 0
  let peakOwnedRssBytes = 0
  let peakOwnedCpuPercent = 0
  const signalHandler = signal => { interrupted ??= signal }
  process.once("SIGINT", signalHandler)
  process.once("SIGTERM", signalHandler)

  try {
    fixture = await startFixtureServer(sessionSecret)
    chromium = spawnLogged("chromium", "chromium", [
      "--headless=new", `--user-data-dir=${profileRoot}`, "--password-store=basic", "--no-first-run",
      "--no-default-browser-check", "--disable-sync", "--disable-dev-shm-usage", "--disable-gpu",
      "--remote-debugging-address=127.0.0.1", `--remote-debugging-port=${allocation.debugPort}`, fixture.loginUrl,
    ], { cwd: repoRoot, logsRoot })
    await waitForHttp(`http://127.0.0.1:${allocation.debugPort}/json/version`, 20_000)
    controller = new ControllerClient(spawnProtocol("browser-controller", process.execPath, [
      path.join(repoRoot, "apps", "kernel", "slice-linux-docker", "docker", "browser-controller.mjs"), "stdio",
    ], { cwd: repoRoot, logsRoot, env: { ...process.env, CHARIOX_BROWSER_DEBUGGER_ENDPOINT: `http://127.0.0.1:${allocation.debugPort}` } }))
    await writeJson(paths.ownership, {
      schema: "chariox.idle_authenticated_browser_soak_ownership.v1", ownerPid: process.pid,
      processes: [{ name: "runner", pid: process.pid, pgid: process.pid }, { name: "chromium", pid: chromium.pid, pgid: chromium.pid },
        { name: "browser-controller", pid: controller.child.pid, pgid: controller.child.pid }],
      cleanupPolicy: "exact recorded process groups only",
    })
    await writeJson(paths.profile, { schema: "chariox.synthetic_profile_marker.v1", markerDigest, markerPath, secretRetained: false })
    await checkpoint("initial")
    const deadline = Date.now() + options.durationSeconds * 1_000
    let nextHealth = Date.now() + options.healthIntervalSeconds * 1_000
    let nextSample = Date.now() + options.sampleIntervalSeconds * 1_000
    while (Date.now() < deadline) {
      if (interrupted) throw new Error(`soak interrupted by ${interrupted}`)
      assertAlive(chromium, "chromium")
      assertAlive(controller.child, "browser-controller")
      const now = Date.now()
      if (now >= nextHealth) {
        await checkpoint("periodic")
        nextHealth += options.healthIntervalSeconds * 1_000
      }
      if (now >= nextSample) {
        await sample("periodic")
        nextSample += options.sampleIntervalSeconds * 1_000
      }
      await sleep(Math.min(250, Math.max(1, Math.min(nextHealth, nextSample, deadline) - Date.now())))
    }
    await sample("final")
    status = "passed"
  } catch (error) {
    status = interrupted ? "interrupted" : "failed"
    failure = bounded(error?.stack ?? error)
  } finally {
    const cleanup = await cleanupOwned({ controller, chromium, fixture, stateRoot, allocation })
    await writeJson(paths.cleanup, cleanup)
    const result = {
      schema, status, pid: process.pid, startedAt, completedAt: new Date().toISOString(),
      durationSeconds: options.durationSeconds, command: commandEvidence(options, paths, allocation), source, image,
      fixture: { synthetic: true, externalNetworkUsed: false }, counters,
      resources: { sampleCount, peakOwnedRssBytes, peakOwnedCpuPercent, ceilingsRespected: status === "passed", limits: limits(options), baseline },
      profile: { markerDigest, checks: counters.profileMarkerChecks, secretRetained: false },
      redaction: { passed: false }, cleanup,
    }
    if (failure) result.failure = failure
    try {
      await assertEvidenceRedacted(paths, forbidden, logsRoot)
      result.redaction.passed = true
    } catch (error) {
      result.status = "failed"
      result.failure = bounded(error?.message ?? error)
    }
    if (result.status === "passed") {
      try { validateCompletedIdleSoakResult(result) } catch (error) { result.status = "failed"; result.failure = bounded(error?.message ?? error) }
    }
    if (result.status !== "passed") await writeJson(paths.failure, {
      schema: "chariox.idle_authenticated_browser_soak_failure.v1", status: result.status, at: result.completedAt,
      marker: result.failure ?? "idle authenticated soak did not pass",
    })
    await writeJson(paths.result, result)
    await writeJson(paths.status, { schema, status: result.status, pid: process.pid, startedAt, completedAt: result.completedAt,
      counters, resources: result.resources, failureMarker: result.status === "passed" ? null : paths.failure,
      cleanupReady: cleanup.clean, result: paths.result })
    process.removeListener("SIGINT", signalHandler)
    process.removeListener("SIGTERM", signalHandler)
    console.log(JSON.stringify({ status: result.status, pid: process.pid, runDir: paths.runDir, result: paths.result }))
    if (result.status !== "passed") process.exitCode = 1
  }

  async function checkpoint(label) {
    await controller.request("health")
    const reconciled = await controller.request("browser.reconcile", { viewport: { css_width: 800, css_height: 600, device_scale_factor: 1 } })
    const tab = reconciled.tabs.find(entry => entry.url?.startsWith(fixture.baseUrl)) ?? reconciled.tabs[0]
    if (!tab) throw new Error("Browser Controller returned no Chromium tab")
    await controller.request("browser.navigate", { target_id: tab.target_id, document_id: tab.document_id, url: fixture.authenticatedUrl })
    const refreshed = await controller.request("browser.reconcile", { viewport: { css_width: 800, css_height: 600, device_scale_factor: 1 } })
    const current = refreshed.tabs.find(entry => entry.url?.startsWith(fixture.baseUrl)) ?? refreshed.tabs[0]
    const snapshot = await controller.request("browser.snapshot", { target_id: current.target_id, document_id: current.document_id })
    if (!JSON.stringify(snapshot).includes("Authenticated synthetic session")) throw new Error("synthetic authenticated page was not observable")
    if (fixture.authenticatedRequests <= counters.authenticatedSessionChecks) throw new Error("synthetic authenticated session did not advance")
    if (digest(await readFile(markerPath, "utf8")) !== markerDigest) throw new Error("persistent profile marker changed")
    counters = {
      healthChecks: counters.healthChecks + 1,
      controllerRequests: controller.requestCount,
      authenticatedSessionChecks: counters.authenticatedSessionChecks + 1,
      profileMarkerChecks: counters.profileMarkerChecks + 1,
    }
    assertCheckpointAdvanced(previousCounters, counters)
    previousCounters = { ...counters }
    await sample(label)
    await writeJson(paths.status, { schema, status: "healthy", pid: process.pid, startedAt, updatedAt: new Date().toISOString(),
      elapsedSeconds: Math.floor((Date.now() - Date.parse(startedAt)) / 1_000), counters,
      resources: { sampleCount, peakOwnedRssBytes, peakOwnedCpuPercent, limits: limits(options) },
      profile: { markerDigest, persisted: true }, failureMarker: null, cleanupReady: true, runDir: paths.runDir })
    await assertEvidenceRedacted(paths, forbidden, logsRoot)
  }

  async function sample(label) {
    const value = await resourceSnapshot(label, [process.pid, chromium?.pid, controller?.child?.pid].filter(Number.isSafeInteger), paths.runDir)
    assertResourceCeilings(value, options)
    sampleCount += 1
    peakOwnedRssBytes = Math.max(peakOwnedRssBytes, value.owned.rssBytes)
    peakOwnedCpuPercent = Math.max(peakOwnedCpuPercent, value.owned.cpuPercent)
    await appendFile(paths.samples, `${JSON.stringify(value)}\n`, { mode: 0o600 })
  }
}

class ControllerClient {
  constructor(child) {
    this.child = child; this.requestCount = 0; this.nextId = 0; this.pending = new Map()
    readJsonLines(child.stdout, value => {
      const pending = this.pending.get(value.id)
      if (!pending) return
      this.pending.delete(value.id)
      value.ok ? pending.resolve(value.result) : pending.reject(new Error(`Browser Controller: ${value.error?.message ?? "request failed"}`))
    })
    child.once("exit", () => { for (const pending of this.pending.values()) pending.reject(new Error("Browser Controller exited")); this.pending.clear() })
  }
  request(method, params) {
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

async function startFixtureServer(sessionSecret) {
  let authenticatedRequests = 0
  const cookie = `chariox_synthetic_session=${sessionSecret}`
  const server = http.createServer((request, response) => {
    if (request.url === "/login") {
      response.writeHead(302, { location: "/authenticated", "set-cookie": `${cookie}; HttpOnly; SameSite=Strict; Path=/`, "cache-control": "no-store" })
      return response.end()
    }
    if (request.url === "/authenticated" && request.headers.cookie?.split(/;\s*/).includes(cookie)) {
      authenticatedRequests += 1
      response.writeHead(200, { "content-type": "text/html", "cache-control": "no-store" })
      return response.end("<!doctype html><title>Chariox synthetic auth</title><main><h1>Authenticated synthetic session</h1><p>Idle browser health fixture</p></main>")
    }
    response.writeHead(401, { "content-type": "text/plain", "cache-control": "no-store" })
    response.end("unauthenticated")
  })
  await new Promise((resolve, reject) => server.once("error", reject).listen(0, "127.0.0.1", resolve))
  const baseUrl = `http://127.0.0.1:${server.address().port}`
  return { baseUrl, loginUrl: `${baseUrl}/login`, authenticatedUrl: `${baseUrl}/authenticated`,
    get authenticatedRequests() { return authenticatedRequests },
    close: () => new Promise((resolve, reject) => server.close(error => error ? reject(error) : resolve())) }
}

async function cleanupOwned({ controller, chromium, fixture, stateRoot, allocation }) {
  const actions = []
  try { await controller?.shutdown(); actions.push({ name: "controller-shutdown", ok: true }) } catch (error) { actions.push({ name: "controller-shutdown", ok: false, error: bounded(error) }) }
  if (controller?.child) actions.push(await terminateGroup("browser-controller", controller.child))
  if (chromium) actions.push(await terminateGroup("chromium", chromium))
  try { await fixture?.close(); actions.push({ name: "fixture-close", ok: true }) } catch (error) { actions.push({ name: "fixture-close", ok: false, error: bounded(error) }) }
  try { await rm(stateRoot, { recursive: true, force: true }); actions.push({ name: "state-remove", ok: true }) } catch (error) { actions.push({ name: "state-remove", ok: false, error: bounded(error) }) }
  await sleep(250)
  const remainingPids = [controller?.child?.pid, chromium?.pid].filter(pid => Number.isSafeInteger(pid) && running(pid))
  return { schema: "chariox.idle_authenticated_browser_soak_cleanup.v1", at: new Date().toISOString(), actions,
    remainingPids, debugPortReleased: await portAvailable(allocation.debugPort), stateRemoved: !await exists(stateRoot),
    clean: remainingPids.length === 0 && await portAvailable(allocation.debugPort) && !await exists(stateRoot) && actions.every(action => action.ok) }
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
    if (Number.isSafeInteger(pid) && running(pid) && ["starting", "running", "healthy"].includes(status.status)) {
      throw new Error(`owned idle soak already active at ${runDir} with pid ${pid}`)
    }
  }
}

async function assertEvidenceRedacted(paths, forbidden, logsRoot) {
  const texts = []
  for (const candidate of [paths.status, paths.preflight, paths.ownership, paths.profile, paths.samples, paths.cleanup, paths.failure, paths.result]) {
    texts.push(await readFile(candidate, "utf8").catch(() => ""))
  }
  if (logsRoot) {
    for (const entry of await readdir(logsRoot, { withFileTypes: true }).catch(() => [])) {
      if (entry.isFile()) texts.push(await readFile(path.join(logsRoot, entry.name), "utf8").catch(() => ""))
    }
  }
  assertRetainedEvidenceRedacted(texts, forbidden)
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
  child.stdout.pipe(log, { end: false }); child.stderr.pipe(log, { end: false }); child.once("exit", () => log.end())
  return child
}

function spawnProtocol(name, command, args, { cwd, logsRoot, env }) {
  const log = createWriteStream(path.join(logsRoot, `${name}.stderr.log`), { flags: "a", mode: 0o600 })
  const child = spawn(command, args, { cwd, env, detached: true, stdio: ["pipe", "pipe", "pipe"] })
  child.stderr.pipe(log); child.once("exit", () => log.end()); return child
}

function readJsonLines(stream, consume) {
  let buffer = ""; stream.setEncoding("utf8")
  stream.on("data", chunk => { buffer += chunk; for (;;) { const index = buffer.indexOf("\n"); if (index < 0) break
    const line = buffer.slice(0, index).trim(); buffer = buffer.slice(index + 1); if (line) consume(JSON.parse(line)) } })
}

async function terminateGroup(name, child) {
  if (!child || child.exitCode !== null) return { name, ok: true, alreadyExited: true }
  let forced = false
  try { process.kill(-child.pid, "SIGTERM") } catch {}
  for (let i = 0; i < 50 && running(child.pid); i += 1) await sleep(100)
  if (running(child.pid)) { forced = true; try { process.kill(-child.pid, "SIGKILL") } catch {} }
  return { name, ok: !running(child.pid), forced }
}

async function waitForHttp(url, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) { try { const response = await fetch(url, { signal: AbortSignal.timeout(1_000) }); if (response.ok) return } catch {} await sleep(100) }
  throw new Error(`${url} did not become ready`)
}

async function availablePort(minimum, maximum) {
  for (let port = minimum; port <= maximum; port += 1) if (await portAvailable(port)) return port
  throw new Error(`no loopback port available from ${minimum} to ${maximum}`)
}

function portAvailable(port) {
  return new Promise(resolve => { const server = net.createServer(); server.unref(); server.once("error", () => resolve(false));
    server.listen(port, "127.0.0.1", () => server.close(() => resolve(true))) })
}

function assertAlive(child, name) { if (!child || child.exitCode !== null) throw new Error(`${name} exited during soak`) }
function running(pid) { try { process.kill(pid, 0); return true } catch { return false } }
async function exists(candidate) { try { await stat(candidate); return true } catch { return false } }
function limits(options) { return { maxCpuPercent: options.maxCpuPercent, maxRssMb: options.maxRssMb, maxProcesses: options.maxProcesses, minFreeDiskMb: options.minFreeDiskMb } }
function digest(value) { return createHash("sha256").update(value).digest("hex") }
function bounded(value) { return String(value?.message ?? value).replace(/[\r\n]+/g, " ").slice(0, 2_000) }
function sleep(ms) { return new Promise(resolve => setTimeout(resolve, ms)) }
async function writeJson(file, value) { const temporary = `${file}.tmp-${process.pid}`; await writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 }); await rename(temporary, file) }
