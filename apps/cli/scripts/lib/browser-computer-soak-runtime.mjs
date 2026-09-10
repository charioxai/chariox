import { appendFile, chmod, mkdir, mkdtemp, open, readFile, rename, rm, stat, writeFile } from "node:fs/promises"
import { createWriteStream } from "node:fs"
import { createHash } from "node:crypto"
import { execFile, spawn } from "node:child_process"
import http from "node:http"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { promisify } from "node:util"

import { buildSoakPaths, detachedLaunchSummary, validateCompletedSoakResult } from "./browser-computer-soak.mjs"

const execFileAsync = promisify(execFile)
const schema = "chariox.browser_computer_soak.v1"
const minimumFreeBytes = 512 * 1024 * 1024
const viewport = {
  css_width: 800,
  css_height: 600,
  device_scale_factor: 1,
  desktop_pixel_width: 800,
  desktop_pixel_height: 600,
}

export async function runBrowserComputerSoak({ options, repoRoot, scriptPath }) {
  const runId = new Date().toISOString().replace(/[:.]/g, "-")
  const paths = options.runDir
    ? buildSoakPaths(path.dirname(options.runDir), path.basename(options.runDir))
    : buildSoakPaths(options.evidenceRoot, runId)
  await mkdir(paths.runDir, { recursive: true, mode: 0o700 })
  const startedAt = new Date().toISOString()
  let allocation = null
  let source = null
  let baseline = null
  try {
    if (options.mode === "run") {
      await writeFile(paths.pid, `${process.pid}\n`, { mode: 0o600 })
      await writeJson(paths.status, { schema, status: "starting", pid: process.pid, startedAt, runDir: paths.runDir })
    }
    allocation = await allocateRuntime(options)
    source = await sourceIdentity(repoRoot)
    baseline = await baselineResourceSnapshot(paths)
    const preflight = await runPreflight({ options, allocation, paths, repoRoot, source, baseline })
    await writeJson(paths.preflight, preflight)
    if (options.mode === "preflight") {
      console.log(JSON.stringify({ status: "passed", runDir: paths.runDir, preflight: paths.preflight, allocation }))
      return
    }
    if (options.mode === "detach") {
      const args = detachedArgs({ options, paths, allocation })
      const descriptor = await open(paths.log, "a", 0o600)
      const child = spawn(process.execPath, [scriptPath, ...args], {
        cwd: repoRoot,
        env: process.env,
        detached: true,
        stdio: ["ignore", descriptor.fd, descriptor.fd],
      })
      child.unref()
      await descriptor.close()
      await writeFile(paths.pid, `${child.pid}\n`, { mode: 0o600 })
      await writeJson(paths.status, {
        schema,
        status: "starting",
        pid: child.pid,
        startedAt,
        runDir: paths.runDir,
      })
      console.log(JSON.stringify(detachedLaunchSummary(paths, child.pid)))
      return
    }
    await executeSoak({ options, allocation, paths, repoRoot, source, baseline })
  } catch (error) {
    if (!await exists(paths.result)) {
      await persistStartupFailure({ error, options, allocation, paths, source, baseline, startedAt })
    }
    throw error
  }
}

async function executeSoak({ options, allocation, paths, repoRoot, source, baseline }) {
  const startedAt = new Date().toISOString()
  const sourceRoot = path.join(repoRoot, "apps", "kernel", "slice-linux-docker", "docker")
  const stateRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-browser-computer-soak-"))
  const runtimeRoot = path.join(stateRoot, "runtime")
  const profileRoot = path.join(stateRoot, "chromium-profile")
  const logsRoot = path.join(paths.runDir, "process-logs")
  await Promise.all([
    mkdir(runtimeRoot, { recursive: true, mode: 0o700 }),
    mkdir(profileRoot, { recursive: true, mode: 0o700 }),
    mkdir(logsRoot, { recursive: true, mode: 0o700 }),
  ])
  await writeFile(paths.pid, `${process.pid}\n`, { mode: 0o600 })
  const owned = new Map()
  let fixture = null
  let controller = null
  let stream = null
  let selkiesPid = null
  let interrupted = null
  let sampleCount = 0
  let peakOwnedRssBytes = 0
  let peakOwnedCpuPercent = 0
  let screenshotDigests = new Set()
  let iterations = 0
  let chromiumMutations = 0
  let firstHealth = null
  let failure = null
  let status = "running"

  const signalHandler = (signal) => { interrupted ??= signal }
  process.once("SIGINT", signalHandler)
  process.once("SIGTERM", signalHandler)

  const display = `:${allocation.displayNumber}`
  const environment = {
    ...process.env,
    DISPLAY: display,
    XDG_RUNTIME_DIR: runtimeRoot,
    CHARIOX_SLICE_DISPLAY: display,
    CHARIOX_SLICE_NOVNC_PORT: String(allocation.viewerPort),
    CHARIOX_SLICE_VIEWER_BACKEND: "selkies",
    CHARIOX_BROWSER_DEBUGGER_ENDPOINT: `http://127.0.0.1:${allocation.debugPort}`,
    CHARIOX_SLICE_ROOT: sourceRoot,
    OMP_NUM_THREADS: "1",
    PYTHONDONTWRITEBYTECODE: "1",
  }
  const command = commandEvidence({ options, paths, allocation })
  const result = {
    schema,
    status,
    pid: process.pid,
    startedAt,
    completedAt: null,
    durationSeconds: options.durationSeconds,
    smoke: options.smoke,
    command,
    source,
    allocation,
    paths,
    firstHealth: null,
    activity: { iterations: 0, controllerRequests: 0, chromiumMutations: 0, screenshotDigests: 0 },
    stream: { binaryFrames: 0, binaryBytes: 0, changingFrameDigests: 0, textMarkers: [] },
    resources: { sampleCount: 0, peakOwnedRssBytes: 0, peakOwnedCpuPercent: 0, baseline },
    cleanup: null,
  }

  try {
    await assertRuntimeStillAvailable(allocation)
    fixture = await startFixtureServer()
    const xvfb = spawnLogged("xvfb", "Xvfb", [display, "-screen", "0", "800x600x24", "-ac", "+extension", "RANDR", "+extension", "XTEST"], {
      env: environment, cwd: repoRoot, logsRoot,
    })
    owned.set("xvfb", xvfb)
    await waitForDisplay(display, environment)
    owned.set("openbox", spawnLogged("openbox", "openbox", [], { env: environment, cwd: repoRoot, logsRoot }))
    owned.set("tint2", spawnLogged("tint2", "tint2", ["-c", path.join(sourceRoot, "tint2rc")], { env: environment, cwd: repoRoot, logsRoot }))

    const chromiumArgs = [
      `--user-data-dir=${profileRoot}`,
      "--password-store=basic",
      "--no-first-run",
      "--no-default-browser-check",
      "--disable-sync",
      "--disable-dev-shm-usage",
      "--disable-gpu",
      "--remote-debugging-address=127.0.0.1",
      `--remote-debugging-port=${allocation.debugPort}`,
      "--window-size=800,600",
      fixture.url,
    ]
    owned.set("chromium", spawnLogged("chromium", "chromium", chromiumArgs, { env: environment, cwd: repoRoot, logsRoot }))
    await waitForHttp(`http://127.0.0.1:${allocation.debugPort}/json/version`, 20_000)

    const selkies = await execJson("/opt/chariox-selkies/bin/python", [path.join(sourceRoot, "slice-selkies.py"), "start"], {
      cwd: sourceRoot, env: environment, timeout: 30_000,
    })
    if (selkies.available !== true || !Number.isSafeInteger(selkies.pid)) throw new Error("Selkies did not report a healthy owned process")
    selkiesPid = selkies.pid

    controller = new ControllerClient(spawnProtocol("browser-controller", process.execPath, [path.join(sourceRoot, "browser-controller.mjs"), "stdio"], {
      env: environment, cwd: repoRoot, logsRoot,
    }))
    stream = new StreamClient(spawnProtocol("selkies-stream", "/opt/chariox-selkies/bin/python", [
      path.join(sourceRoot, "slice-selkies-stream.py"), "--lease-ms", "60000",
    ], { env: environment, cwd: sourceRoot, logsRoot }))
    await stream.waitReady(15_000)
    stream.startRenewal()
    stream.requestKeyframe()
    await stream.waitForFrames(2, 10_000)

    await recordSample("initial", [process.pid, ...pids(owned), selkiesPid])
    const deadline = Date.now() + options.durationSeconds * 1_000
    let nextActivity = Date.now()
    let nextSample = Date.now() + options.sampleIntervalSeconds * 1_000
    while (Date.now() < deadline) {
      if (interrupted) throw new Error(`soak interrupted by ${interrupted}`)
      assertProcessHealth(owned, controller, stream)
      const now = Date.now()
      if (now >= nextActivity) {
        const marker = `SOAK-${String(iterations + 1).padStart(8, "0")}`
        const health = await controller.request("health")
        firstHealth ??= health
        const reconciled = await controller.request("browser.reconcile", { viewport })
        const tab = reconciled.tabs.find((entry) => entry.url === fixture.url) ?? reconciled.tabs[0]
        if (!tab) throw new Error("Browser Controller returned no Chromium tab")
        const snapshot = await controller.request("browser.snapshot", {
          target_id: tab.target_id,
          document_id: tab.document_id,
        })
        const field = snapshot.accessibility_nodes.find((entry) =>
          !entry.ignored && entry.role === "textbox" && entry.name.trim() === "Soak marker")
        if (!field?.node_ref) throw new Error("Browser Controller did not discover the soak field")
        await controller.request("browser.action", {
          target_id: tab.target_id,
          document_id: tab.document_id,
          node_ref: field.node_ref,
          action: { kind: "fill", text: marker },
        })
        const observedMarker = await fetch(`${fixture.url}health`, { signal: AbortSignal.timeout(2_000) })
          .then((response) => response.text())
        if (observedMarker !== marker) throw new Error("Chromium mutation did not reach the active fixture")
        chromiumMutations += 1
        stream.requestKeyframe()
        const screenshotPath = path.join(paths.runDir, "latest-screen.png")
        await execFileAsync("scrot", [screenshotPath], { cwd: repoRoot, env: environment, timeout: 10_000 })
        screenshotDigests.add(createHash("sha256").update(await readFile(screenshotPath)).digest("hex"))
        iterations += 1
        await writeJson(paths.status, {
          schema,
          status: "healthy",
          pid: process.pid,
          startedAt,
          updatedAt: new Date().toISOString(),
          firstHealth,
          activity: { iterations, controllerRequests: controller.requestCount, chromiumMutations },
          stream: stream.metrics(),
          resources: { sampleCount, peakOwnedRssBytes, peakOwnedCpuPercent },
          runDir: paths.runDir,
        })
        nextActivity += options.activityIntervalSeconds * 1_000
      }
      if (now >= nextSample) {
        await recordSample("during", [process.pid, ...pids(owned), selkiesPid])
        nextSample += options.sampleIntervalSeconds * 1_000
      }
      await sleep(Math.min(250, Math.max(1, Math.min(nextActivity, nextSample, deadline) - Date.now())))
    }
    await stream.waitForFrames(2, 10_000)
    status = "passed"
  } catch (error) {
    status = interrupted ? "interrupted" : "failed"
    failure = bounded(error?.stack ?? error)
  } finally {
    stream?.stopRenewal()
    const cleanup = await cleanupRuntime({ controller, stream, owned, selkiesPid, environment, sourceRoot, fixture, stateRoot, allocation })
    await writeJson(paths.cleanup, cleanup)
    const streamMetrics = stream?.metrics() ?? result.stream
    result.status = status
    result.completedAt = new Date().toISOString()
    result.firstHealth = firstHealth
    result.activity = {
      iterations,
      controllerRequests: controller?.requestCount ?? 0,
      chromiumMutations,
      screenshotDigests: screenshotDigests.size,
    }
    result.stream = streamMetrics
    result.resources = { sampleCount, peakOwnedRssBytes, peakOwnedCpuPercent, baseline }
    result.cleanup = cleanup
    if (failure) result.failure = failure
    if (status === "passed") {
      try { validateCompletedSoakResult(result) } catch (error) {
        result.status = "failed"
        result.failure = bounded(error?.message ?? error)
      }
    }
    if (result.status !== "passed") {
      await writeJson(paths.failure, {
        schema: "chariox.browser_computer_soak_failure.v1",
        status: result.status,
        at: result.completedAt,
        marker: result.failure ?? "soak did not pass",
      })
    }
    await writeJson(paths.result, result)
    await writeJson(paths.status, {
      schema,
      status: result.status,
      pid: process.pid,
      startedAt,
      completedAt: result.completedAt,
      result: paths.result,
      cleanup: paths.cleanup,
    })
    process.removeListener("SIGINT", signalHandler)
    process.removeListener("SIGTERM", signalHandler)
    console.log(JSON.stringify({ status: result.status, pid: process.pid, runDir: paths.runDir, result: paths.result }))
    if (result.status !== "passed") process.exitCode = 1
  }

  async function recordSample(label, roots) {
    const sample = await resourceSnapshot(label, roots.filter(Number.isSafeInteger), paths.runDir)
    sampleCount += 1
    peakOwnedRssBytes = Math.max(peakOwnedRssBytes, sample.owned.rssBytes)
    peakOwnedCpuPercent = Math.max(peakOwnedCpuPercent, sample.owned.cpuPercent)
    await appendFile(paths.samples, `${JSON.stringify(sample)}\n`, { mode: 0o600 })
    await chmod(paths.samples, 0o600)
  }
}

async function runPreflight({ options, allocation, paths, repoRoot, source, baseline }) {
  assertSandboxCapableChromiumIdentity()
  const sourceRoot = path.join(repoRoot, "apps", "kernel", "slice-linux-docker", "docker")
  const commands = ["Xvfb", "openbox", "tint2", "chromium", "scrot", "xdpyinfo"]
  const commandPaths = {}
  for (const command of commands) commandPaths[command] = (await execFileAsync("which", [command], { timeout: 5_000 })).stdout.trim()
  const files = [
    "browser-controller.mjs",
    "browser-controller-cdp.mjs",
    "slice-selkies.py",
    "slice-selkies-stream.py",
    "selkies_viewers.py",
    "tint2rc",
  ]
  for (const file of files) await stat(path.join(sourceRoot, file))
  await Promise.all([
    stat("/opt/chariox-selkies/bin/python"),
    stat("/opt/chariox-selkies/bin/selkies"),
  ])
  if (baseline.host.freeMemoryBytes < minimumFreeBytes) throw new Error("preflight requires at least 512 MiB free RAM")
  if (baseline.disk.availableBytes < minimumFreeBytes) throw new Error("preflight requires at least 512 MiB free disk")
  await assertRuntimeStillAvailable(allocation)
  return {
    schema: "chariox.browser_computer_soak_preflight.v1",
    status: "passed",
    at: new Date().toISOString(),
    durationSeconds: options.durationSeconds,
    smoke: options.smoke,
    source,
    allocation,
    commandPaths,
    sourceFiles: files,
    baseline,
    paths,
  }
}

export function assertSandboxCapableChromiumIdentity({
  uid = process.getuid?.(),
  managedProviderIsolationActive = process.env.CHARIOX_MANAGED_PROVIDER_ISOLATION_ACTIVE,
} = {}) {
  if (uid !== 0) return
  const launchContext = managedProviderIsolationActive === "1"
    ? "launch the soak from the slice-owner desktop/runtime instead of the provider sandbox"
    : "launch the soak as the non-root slice user"
  throw new Error(`preflight refuses root Chromium because the renderer sandbox would be disabled; ${launchContext}`)
}

async function allocateRuntime(options) {
  const debugPort = options.debugPort ?? await availablePort(52_000, 55_000)
  const viewerPort = options.viewerPort ?? await availablePort(55_000, 58_000, new Set([debugPort]))
  if (debugPort === viewerPort) throw new Error("debug-port and viewer-port must differ")
  const displayNumber = options.displayNumber ?? await availableDisplay(70, 120)
  return { debugPort, viewerPort, displayNumber }
}

async function assertRuntimeStillAvailable({ debugPort, viewerPort, displayNumber }) {
  for (const port of [debugPort, viewerPort]) {
    if (!await portAvailable(port)) throw new Error(`preflight port ${port} is already in use`)
  }
  for (const candidate of [`/tmp/.X11-unix/X${displayNumber}`, `/tmp/.X${displayNumber}-lock`]) {
    if (await exists(candidate)) throw new Error(`preflight display :${displayNumber} is already in use`)
  }
}

async function cleanupRuntime({ controller, stream, owned, selkiesPid, environment, sourceRoot, fixture, stateRoot, allocation }) {
  const actions = []
  try { await controller?.shutdown(); actions.push({ name: "controller-shutdown", ok: true }) } catch (error) {
    actions.push({ name: "controller-shutdown", ok: false, error: bounded(error?.message ?? error) })
  }
  try { await stream?.close(); actions.push({ name: "stream-close", ok: true }) } catch (error) {
    actions.push({ name: "stream-close", ok: false, error: bounded(error?.message ?? error) })
  }
  if (stream?.child) actions.push(await terminateGroup("selkies-stream-process", stream.child))
  if (controller?.child) actions.push(await terminateGroup("browser-controller-process", controller.child))
  try {
    const stopped = await execJson("/opt/chariox-selkies/bin/python", [path.join(sourceRoot, "slice-selkies.py"), "stop", "--allow-forced"], {
      cwd: sourceRoot, env: environment, timeout: 20_000,
    })
    actions.push({ name: "selkies-stop", ok: stopped.stopped === true, forced: stopped.forced === true })
  } catch (error) {
    actions.push({ name: "selkies-stop", ok: false, error: bounded(error?.message ?? error) })
  }
  for (const [name, child] of [...owned.entries()].reverse()) actions.push(await terminateGroup(name, child))
  try { await fixture?.close(); actions.push({ name: "fixture-close", ok: true }) } catch (error) {
    actions.push({ name: "fixture-close", ok: false, error: bounded(error?.message ?? error) })
  }
  try { await rm(stateRoot, { recursive: true, force: true }); actions.push({ name: "state-remove", ok: true }) } catch (error) {
    actions.push({ name: "state-remove", ok: false, error: bounded(error?.message ?? error) })
  }
  await sleep(250)
  const remainingPids = [
    ...pids(owned), controller?.child?.pid, stream?.child?.pid, selkiesPid,
  ].filter((pid) => Number.isSafeInteger(pid) && running(pid))
  const portsReleased = await Promise.all([portAvailable(allocation.debugPort), portAvailable(allocation.viewerPort)])
  const displayReleased = !await exists(`/tmp/.X11-unix/X${allocation.displayNumber}`) && !await exists(`/tmp/.X${allocation.displayNumber}-lock`)
  const stateRemoved = !await exists(stateRoot)
  return {
    schema: "chariox.browser_computer_soak_cleanup.v1",
    at: new Date().toISOString(),
    actions,
    remainingPids,
    portsReleased: portsReleased.every(Boolean),
    displayReleased,
    stateRemoved,
    clean: remainingPids.length === 0 && portsReleased.every(Boolean) && displayReleased && stateRemoved && actions.every((entry) => entry.ok),
  }
}

class ControllerClient {
  constructor(child) {
    this.child = child
    this.requestCount = 0
    this.nextId = 0
    this.pending = new Map()
    readJsonLines(child.stdout, (value) => {
      const pending = this.pending.get(value.id)
      if (!pending) return
      this.pending.delete(value.id)
      if (value.ok) pending.resolve(value.result)
      else pending.reject(new Error(`Browser Controller ${value.error?.code ?? "error"}: ${value.error?.message ?? "request failed"}`))
    })
    child.once("exit", () => {
      for (const pending of this.pending.values()) pending.reject(new Error("Browser Controller exited with pending requests"))
      this.pending.clear()
    })
  }

  request(method, params) {
    const id = ++this.nextId
    this.requestCount += 1
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`Browser Controller ${method} timed out`))
      }, 15_000)
      this.pending.set(id, {
        resolve: (value) => { clearTimeout(timer); resolve(value) },
        reject: (error) => { clearTimeout(timer); reject(error) },
      })
      this.child.stdin.write(`${JSON.stringify({ id, method, params })}\n`)
    })
  }

  async shutdown() {
    if (childExited(this.child)) return
    await this.request("shutdown")
    this.child.stdin.end()
    await waitForExit(this.child, 5_000)
  }
}

class StreamClient {
  constructor(child) {
    this.child = child
    this.ready = false
    this.readyAt = null
    this.binaryFrames = 0
    this.binaryBytes = 0
    this.digests = new Set()
    this.textMarkers = new Set()
    this.lastBinaryFrameAt = null
    this.waiters = new Set()
    this.renewal = null
    readJsonLines(child.stdout, (value) => {
      if (value.kind === "ready") {
        this.ready = true
        this.readyAt ??= Date.now()
      }
      if (value.kind === "binary" && typeof value.data_base64 === "string") {
        this.binaryFrames += 1
        this.binaryBytes += Buffer.byteLength(value.data_base64, "base64")
        this.digests.add(createHash("sha256").update(value.data_base64).digest("hex"))
        this.lastBinaryFrameAt = Date.now()
      }
      if (value.kind === "text" && typeof value.text === "string") this.textMarkers.add(value.text)
      for (const wake of this.waiters) wake()
    })
  }

  async waitReady(timeoutMs) {
    await waitUntil(() => this.ready, timeoutMs, this.waiters, "Selkies stream did not become ready")
  }

  async waitForFrames(count, timeoutMs) {
    await waitUntil(() => this.binaryFrames >= count && this.digests.size >= 2, timeoutMs, this.waiters, "Selkies stream did not deliver changing video frames")
  }

  startRenewal() {
    this.renewal = setInterval(() => {
      if (!childExited(this.child)) this.child.stdin.write('{"kind":"renew"}\n')
    }, 20_000)
    this.renewal.unref()
  }

  stopRenewal() { if (this.renewal) clearInterval(this.renewal) }
  requestKeyframe() { if (!childExited(this.child)) this.child.stdin.write('{"kind":"control","text":"REQUEST_KEYFRAME"}\n') }
  metrics() {
    return {
      binaryFrames: this.binaryFrames,
      binaryBytes: this.binaryBytes,
      changingFrameDigests: this.digests.size,
      textMarkers: [...this.textMarkers].sort(),
      lastBinaryFrameAt: this.lastBinaryFrameAt ? new Date(this.lastBinaryFrameAt).toISOString() : null,
    }
  }

  async close() {
    if (childExited(this.child)) return
    this.child.stdin.write('{"kind":"close"}\n')
    this.child.stdin.end()
    await waitForExit(this.child, 5_000)
  }
}

function spawnLogged(name, command, args, { env, cwd, logsRoot }) {
  const log = createWriteStream(path.join(logsRoot, `${name}.log`), { flags: "a", mode: 0o600 })
  const child = spawn(command, args, { cwd, env, detached: true, stdio: ["ignore", "pipe", "pipe"] })
  child.stdout.pipe(log, { end: false })
  child.stderr.pipe(log, { end: false })
  child.once("exit", () => log.end())
  return child
}

function spawnProtocol(name, command, args, { env, cwd, logsRoot }) {
  const log = createWriteStream(path.join(logsRoot, `${name}.stderr.log`), { flags: "a", mode: 0o600 })
  const child = spawn(command, args, { cwd, env, detached: true, stdio: ["pipe", "pipe", "pipe"] })
  child.stderr.pipe(log)
  child.once("exit", () => log.end())
  return child
}

async function terminateGroup(name, child) {
  if (!child || childExited(child)) return { name, ok: true, alreadyExited: true }
  let forced = false
  try { process.kill(-child.pid, "SIGTERM") } catch {}
  if (!await waitForExit(child, 5_000, false)) {
    forced = true
    try { process.kill(-child.pid, "SIGKILL") } catch {}
    await waitForExit(child, 2_000, false)
  }
  return { name, ok: !running(child.pid), forced }
}

async function resourceSnapshot(label, rootPids, diskPath) {
  const { stdout } = await execFileAsync("ps", ["-eo", "pid=,ppid=,rss=,%cpu=,comm="], { timeout: 10_000 })
  const rows = stdout.split("\n").map((line) => line.trim().split(/\s+/, 5)).filter((parts) => parts.length === 5).map(([pid, ppid, rss, cpu, command]) => ({
    pid: Number(pid), ppid: Number(ppid), rssKb: Number(rss), cpuPercent: Number(cpu), command,
  })).filter((row) => Number.isSafeInteger(row.pid) && Number.isSafeInteger(row.ppid))
  const ownedIds = descendantIds(rows, rootPids)
  const ownedRows = rows.filter((row) => ownedIds.has(row.pid))
  const disk = await import("node:fs/promises").then(({ statfs }) => statfs(diskPath))
  return {
    label,
    at: new Date().toISOString(),
    host: {
      totalMemoryBytes: os.totalmem(),
      freeMemoryBytes: os.freemem(),
      loadAverage: os.loadavg(),
      cpuCount: os.cpus().length,
    },
    disk: {
      path: diskPath,
      availableBytes: Number(disk.bavail) * Number(disk.bsize),
      totalBytes: Number(disk.blocks) * Number(disk.bsize),
    },
    owned: {
      rootPids,
      processCount: ownedRows.length,
      rssBytes: ownedRows.reduce((sum, row) => sum + row.rssKb * 1024, 0),
      cpuPercent: ownedRows.reduce((sum, row) => sum + row.cpuPercent, 0),
      processes: ownedRows,
    },
  }
}

function descendantIds(rows, roots) {
  const ids = new Set(roots.filter(Number.isSafeInteger))
  let changed = true
  while (changed) {
    changed = false
    for (const row of rows) if (ids.has(row.ppid) && !ids.has(row.pid)) { ids.add(row.pid); changed = true }
  }
  return ids
}

async function sourceIdentity(repoRoot) {
  const [{ stdout: commit }, { stdout: branch }, { stdout: status }] = await Promise.all([
    execFileAsync("git", ["rev-parse", "HEAD"], { cwd: repoRoot, timeout: 10_000 }),
    execFileAsync("git", ["branch", "--show-current"], { cwd: repoRoot, timeout: 10_000 }),
    execFileAsync("git", ["status", "--short"], { cwd: repoRoot, timeout: 10_000 }),
  ])
  return { commit: commit.trim(), branch: branch.trim(), dirty: status.trim() !== "" }
}

function detachedArgs({ options, paths, allocation }) {
  return [
    "--internal-run",
    ...(options.smoke ? ["--smoke"] : [
      "--duration-seconds", String(options.durationSeconds),
      "--activity-interval-seconds", String(options.activityIntervalSeconds),
      "--sample-interval-seconds", String(options.sampleIntervalSeconds),
    ]),
    "--evidence-root", options.evidenceRoot,
    "--run-dir", paths.runDir,
    "--display-number", String(allocation.displayNumber),
    "--debug-port", String(allocation.debugPort),
    "--viewer-port", String(allocation.viewerPort),
  ]
}

function commandEvidence({ options, paths, allocation }) {
  return [
    "node apps/cli/scripts/live-browser-computer-soak.mjs",
    options.smoke ? "--smoke" : `--duration-seconds ${options.durationSeconds}`,
    `--activity-interval-seconds ${options.activityIntervalSeconds}`,
    `--sample-interval-seconds ${options.sampleIntervalSeconds}`,
    `--evidence-root ${shellQuote(options.evidenceRoot)}`,
    `--run-dir ${shellQuote(paths.runDir)}`,
    `--display-number ${allocation.displayNumber}`,
    `--debug-port ${allocation.debugPort}`,
    `--viewer-port ${allocation.viewerPort}`,
  ].join(" ")
}

async function startFixtureServer() {
  let marker = "SOAK-00000000"
  const server = http.createServer((request, response) => {
    if (request.url === "/health") {
      response.writeHead(200, { "content-type": "text/plain", "cache-control": "no-store" })
      return response.end(marker)
    }
    if (request.url?.startsWith("/mark?")) {
      marker = new URL(request.url, "http://127.0.0.1").searchParams.get("value") ?? marker
      response.writeHead(204, { "cache-control": "no-store" })
      return response.end()
    }
    response.writeHead(200, { "content-type": "text/html", "cache-control": "no-store" })
    response.end(`<!doctype html><title>Chariox active soak</title><style>body{font:24px sans-serif;background:#14213d;color:#fff}main{padding:60px}input{font-size:28px;width:520px}.pulse{width:120px;height:120px;background:#fca311;animation:pulse 1s infinite alternate}@keyframes pulse{to{transform:translateX(320px);background:#2ec4b6}}</style><main><label>Soak marker <input id="marker" value="${marker}"></label><p id="echo">${marker}</p><div class="pulse"></div></main><script>const field=document.querySelector('#marker');field.addEventListener('input',()=>{document.querySelector('#echo').textContent=field.value;document.title=field.value;fetch('/mark?value='+encodeURIComponent(field.value)).catch(()=>{})})</script>`)
  })
  await new Promise((resolve, reject) => server.once("error", reject).listen(0, "127.0.0.1", resolve))
  const url = `http://127.0.0.1:${server.address().port}/`
  return { url, close: () => new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve())) }
}

async function waitForDisplay(display, env) {
  await retry(async () => {
    await execFileAsync("xdpyinfo", ["-display", display], { env, timeout: 2_000 })
  }, 10_000, `X display ${display} did not become ready`)
}

async function waitForHttp(url, timeoutMs) {
  await retry(async () => {
    const response = await fetch(url, { signal: AbortSignal.timeout(1_000) })
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
  }, timeoutMs, `${url} did not become ready`)
}

async function retry(operation, timeoutMs, message) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    try { return await operation() } catch (error) { last = error }
    await sleep(100)
  }
  throw new Error(`${message}: ${bounded(last?.message ?? last)}`)
}

export function assertProcessHealth(owned, controller, stream, now = Date.now()) {
  for (const [name, child] of owned) {
    if (childExited(child)) throw new Error(`${name} exited during soak with ${childExitDescription(child)}`)
  }
  if (childExited(controller.child)) throw new Error(`Browser Controller exited during soak with ${childExitDescription(controller.child)}`)
  if (childExited(stream.child)) throw new Error(`Selkies stream exited during soak with ${childExitDescription(stream.child)}`)
  if (stream.readyAt && !stream.lastBinaryFrameAt && now - stream.readyAt > 30_000) {
    throw new Error("Selkies stream did not deliver its first video frame")
  }
  if (stream.lastBinaryFrameAt && now - stream.lastBinaryFrameAt > 30_000) {
    throw new Error("Selkies stream stopped delivering video frames")
  }
}

export async function baselineResourceSnapshot(paths, {
  rootPids = [process.pid],
  snapshot = resourceSnapshot,
} = {}) {
  return snapshot("preflight", rootPids, paths.runDir)
}

function readJsonLines(stream, consume) {
  let buffer = ""
  stream.setEncoding("utf8")
  stream.on("data", (chunk) => {
    buffer += chunk
    for (;;) {
      const index = buffer.indexOf("\n")
      if (index < 0) break
      const line = buffer.slice(0, index)
      buffer = buffer.slice(index + 1)
      if (!line.trim()) continue
      try { consume(JSON.parse(line)) } catch {}
    }
  })
}

async function waitUntil(condition, timeoutMs, waiters, message) {
  const deadline = Date.now() + timeoutMs
  while (!condition()) {
    const remaining = deadline - Date.now()
    if (remaining <= 0) throw new Error(message)
    await new Promise((resolve) => {
      const timer = setTimeout(done, remaining)
      function done() { clearTimeout(timer); waiters.delete(done); resolve() }
      waiters.add(done)
    })
  }
}

async function execJson(command, args, options) {
  const { stdout } = await execFileAsync(command, args, options)
  return JSON.parse(stdout.trim().split("\n").at(-1))
}

async function writeJson(target, value) {
  const temporary = `${target}.${process.pid}.tmp`
  await writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 })
  await chmod(temporary, 0o600)
  await rename(temporary, target)
}

async function availablePort(start, end, excluded = new Set()) {
  for (let port = start; port < end; port += 1) if (!excluded.has(port) && await portAvailable(port)) return port
  throw new Error(`no available port from ${start} to ${end - 1}`)
}

function portAvailable(port) {
  return new Promise((resolve) => {
    const server = net.createServer()
    server.unref()
    server.once("error", () => resolve(false))
    server.listen(port, "127.0.0.1", () => server.close(() => resolve(true)))
  })
}

async function availableDisplay(start, end) {
  for (let display = start; display < end; display += 1) {
    if (!await exists(`/tmp/.X11-unix/X${display}`) && !await exists(`/tmp/.X${display}-lock`)) return display
  }
  throw new Error(`no available X display from :${start} to :${end - 1}`)
}

async function waitForExit(child, timeoutMs, reject = true) {
  if (childExited(child)) return true
  const exited = await Promise.race([
    new Promise((resolve) => child.once("exit", () => resolve(true))),
    sleep(timeoutMs).then(() => false),
  ])
  if (!exited && reject) throw new Error(`process ${child.pid} did not exit within ${timeoutMs}ms`)
  return exited
}

async function persistStartupFailure({ error, options, allocation, paths, source, baseline, startedAt }) {
  const completedAt = new Date().toISOString()
  const marker = bounded(error?.stack ?? error)
  const cleanup = {
    schema: "chariox.browser_computer_soak_cleanup.v1",
    at: completedAt,
    phase: "startup",
    actions: [],
    remainingPids: [],
    portsReleased: true,
    displayReleased: true,
    stateRemoved: true,
    clean: true,
  }
  const result = {
    schema,
    status: "failed",
    phase: "startup",
    pid: process.pid,
    startedAt,
    completedAt,
    durationSeconds: options.durationSeconds,
    smoke: options.smoke,
    source,
    allocation,
    paths,
    resources: { baseline },
    cleanup,
    failure: marker,
  }
  await writeFile(paths.pid, `${process.pid}\n`, { mode: 0o600 })
  await writeJson(paths.cleanup, cleanup)
  await writeJson(paths.failure, {
    schema: "chariox.browser_computer_soak_failure.v1",
    status: "failed",
    phase: "startup",
    at: completedAt,
    marker,
  })
  await writeJson(paths.result, result)
  await writeJson(paths.status, {
    schema,
    status: "failed",
    phase: "startup",
    pid: process.pid,
    startedAt,
    completedAt,
    result: paths.result,
    cleanup: paths.cleanup,
  })
}

function childExited(child) { return child?.exitCode != null || child?.signalCode != null }
function childExitDescription(child) {
  return child.signalCode ? `signal ${child.signalCode}` : `exit code ${child.exitCode}`
}

function pids(owned) { return [...owned.values()].map((child) => child?.pid).filter(Number.isSafeInteger) }
function running(pid) { try { process.kill(pid, 0); return true } catch { return false } }
async function exists(candidate) { try { await stat(candidate); return true } catch { return false } }
const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds))
const bounded = (value, limit = 2_000) => String(value ?? "").replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, "").slice(-limit)
const shellQuote = (value) => `'${String(value).replaceAll("'", "'\\''")}'`
