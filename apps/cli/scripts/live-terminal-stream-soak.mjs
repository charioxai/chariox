#!/usr/bin/env node
import assert from "node:assert/strict"
import { execFileSync, spawn, spawnSync } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { createHash } from "node:crypto"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { performance } from "node:perf_hooks"
import { fileURLToPath, pathToFileURL } from "node:url"

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..")
const args = process.argv.slice(2)
const durationSeconds = argNumber("--duration-seconds", 1_800)
const bytesPerSecond = argNumber("--bytes-per-second", 1_048_576)
const maxRssMb = argNumber("--max-rss-mb", 1_024)
const maxCpuPercent = argNumber("--max-cpu-percent", 150)
const output = path.resolve(argValue("--output") ?? path.join(os.homedir(), ".codex", "evidence", "terminal-stream-soak", `run-${process.pid}.json`))
const buildProfile = buildProfileArg()
const cargoTargetDir = cargoTargetPath()
const kernelBinary = absolutePathArg("--kernel-binary", path.join(cargoTargetDir, buildProfile, "chariox-kernel"))
const clientRoot = absolutePathArg("--client-root", path.join(repoRoot, "packages", "kernel-client", "dist"))
const dryRun = args.includes("--dry-run")
const chunkBytes = 65_536
const chunksPerTick = Math.ceil(bytesPerSecond / chunkBytes)
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (response, key) => response?.[key] ?? response

if (args.includes("--help")) {
  console.log("Usage: live-terminal-stream-soak.mjs [--duration-seconds 1800] [--bytes-per-second 1048576] [--max-rss-mb 1024] [--max-cpu-percent 150] [--build-profile release|debug] [--kernel-binary PATH] [--client-root PATH] [--output PATH] [--dry-run]")
  process.exit(0)
}
if (dryRun) {
  console.log(JSON.stringify({ durationSeconds, bytesPerSecond, totalBytes: durationSeconds * bytesPerSecond, maxRssMb, maxCpuPercent, buildProfile, cargoTargetDir, kernelBinary, clientRoot, output, release: buildProfile === "release" }, null, 2))
  process.exit(0)
}

const { LocalIpcClient } = await import(pathToFileURL(path.join(clientRoot, "ipc.js")))
const requests = await import(pathToFileURL(path.join(clientRoot, "ipc-requests.js")))
const kernelSha256 = createHash("sha256").update(await readFile(kernelBinary)).digest("hex")

const port = await availablePort()
const scratchParent = path.join(os.homedir(), ".chariox", "dev", "terminal-stream-soak")
await mkdir(scratchParent, { recursive: true, mode: 0o700 })
const root = await mkdtemp(path.join(scratchParent, "run-"))
const kernel = spawn(kernelBinary, [], {
  cwd: repoRoot,
  detached: true,
  stdio: "ignore",
  env: {
    ...process.env,
    HOME: root,
    CHARIOX_HOME: path.join(root, "state"),
    CHARIOX_LOG_DIR: path.join(root, "logs"),
    XDG_CONFIG_HOME: path.join(root, "config"),
    XDG_STATE_HOME: path.join(root, "state"),
    XDG_CACHE_HOME: path.join(root, "cache"),
    CHARIOX_KERNEL_PORT: String(port),
    CHARIOX_MCP_PORT: String(port + 1),
    CHARIOX_OPENCODE_PORT: String(port + 2),
    CHARIOX_CODEX_PORT: String(port + 3),
    CHARIOX_DAEMON_ID: `stream-soak-${process.pid}`,
    CHARIOX_DAEMON_SOCKET: path.join(root, "daemon.sock"),
    CHARIOX_SESSION_HISTORY_DIR: path.join(root, "history"),
  },
})
let client
let report

try {
  await waitForKernel(port)
  client = new LocalIpcClient(`ws://127.0.0.1:${port}`)
  const created = unwrap(await client.send(requests.createSessionRequest(root, root)), "SessionCreated")
  const sessionId = created.session.id
  const agentId = created.agent.id
  const attached = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `stream-soak-${process.pid}`)), "SessionAttached")
  const attachmentId = attached.attachment.id
  let observedEvents = 0
  let observedRecords = 0
  client.onKernelEvent((event) => {
    if (event?.event !== "terminal_output") return
    observedEvents += 1
    observedRecords += event.records?.length ?? 0
  })
  await client.subscribeToKernelEvents(sessionId, attachmentId)
  const launched = unwrap(await client.send(requests.launchProviderRunsRequest([{
    sessionId,
    provider: "dev-stub",
    accountProfile: "default",
    model: "default",
    effort: "low",
    agentId,
    native: { nativeTui: true },
  }], 1)), "ProviderRunsLaunchAccepted")
  assert.equal(launched.failures?.length ?? 0, 0)
  const providerRunId = launched.provider_runs[0].provider_run.id

  const payload = "x".repeat(chunkBytes)
  const startedAt = performance.now()
  const latencies = []
  const cpuSamples = []
  let sentBytes = 0
  for (let second = 0; second < durationSeconds; second += 1) {
    const tickDeadline = startedAt + (second + 1) * 1_000
    const tickStartedAt = performance.now()
    const remaining = Math.min(bytesPerSecond, durationSeconds * bytesPerSecond - sentBytes)
    const outputs = []
    let tickBytes = 0
    for (let chunk = 0; chunk < chunksPerTick && tickBytes < remaining; chunk += 1) {
      const size = Math.min(chunkBytes, remaining - tickBytes)
      outputs.push({
        providerRunId,
        kind: "prompt_echo",
        mergeKey: `soak-${second}-${chunk}`,
        text: size === chunkBytes ? payload : payload.slice(0, size),
      })
      tickBytes += size
    }
    const response = unwrap(await client.send(requests.appendNativeProviderOutputBatchRequest(sessionId, attachmentId, outputs)), "TerminalOutput")
    assert.equal(response.records.length, outputs.length)
    sentBytes += tickBytes
    latencies.push(performance.now() - tickStartedAt)
    if (second % 10 === 0) cpuSamples.push(processMetric(kernel.pid))
    const waitMs = tickDeadline - performance.now()
    if (waitMs > 0) await sleep(waitMs)
  }
  const streamElapsedMs = performance.now() - startedAt
  await sleep(1_000)
  const sorted = [...latencies].sort((left, right) => left - right)
  const finalMetric = processMetric(kernel.pid)
  const resourceSamples = [...cpuSamples, finalMetric]
  const peakRssMb = Math.max(...resourceSamples.map((sample) => sample.rssKb / 1024))
  const cpuP95Percent = percentile(
    resourceSamples.map((sample) => sample.cpuPercent).sort((left, right) => left - right),
    0.95,
  )
  report = {
    ok: peakRssMb <= maxRssMb && cpuP95Percent <= maxCpuPercent,
    buildProfile,
    cargoTargetDir,
    kernelBinary,
    kernelSha256,
    clientRoot,
    durationSeconds,
    bytesPerSecond,
    sentBytes,
    elapsedMs: streamElapsedMs,
    effectiveBytesPerSecond: Math.round((sentBytes * 1_000) / streamElapsedMs),
    appendLatencyMs: {
      p50: percentile(sorted, 0.5),
      p95: percentile(sorted, 0.95),
      p99: percentile(sorted, 0.99),
      max: sorted.at(-1) ?? 0,
    },
    appendLatencySamplesMs: latencies,
    observedEvents,
    observedRecords,
    cpuSamples,
    finalMetric,
    resourceBudget: {
      maxRssMb,
      maxCpuPercent,
      peakRssMb: Math.round(peakRssMb * 10) / 10,
      cpuP95Percent,
    },
    port,
  }
  if (!report.ok) process.exitCode = 1
} catch (error) {
  report = { ok: false, error: String(error?.stack ?? error), durationSeconds, bytesPerSecond, buildProfile, cargoTargetDir, kernelBinary, kernelSha256, clientRoot, port }
  process.exitCode = 1
} finally {
  await client?.close?.().catch(() => undefined)
  const cleanup = await terminateTree(kernel.pid)
  await mkdir(path.dirname(output), { recursive: true })
  await writeFile(output, `${JSON.stringify({ ...report, cleanup }, null, 2)}\n`)
  await rm(root, { recursive: true, force: true })
  if (cleanup.remaining.length) process.exitCode = 1
}
console.log(JSON.stringify({ ...report, output }, null, 2))

function argValue(flag) {
  const index = args.indexOf(flag)
  return index >= 0 ? args[index + 1] : null
}
function argNumber(flag, fallback) {
  const value = Number(argValue(flag) ?? fallback)
  if (!Number.isInteger(value) || value <= 0) throw new Error(`${flag} must be a positive integer`)
  return value
}
function buildProfileArg() {
  const value = argValue("--build-profile") ?? "release"
  if (value !== "debug" && value !== "release") throw new Error("--build-profile must be debug or release")
  return value
}
function absolutePathArg(flag, fallback) {
  const value = argValue(flag) ?? fallback
  if (!path.isAbsolute(value)) throw new Error(`${flag} must be absolute`)
  return value
}
function cargoTargetPath() {
  const value = process.env.CARGO_TARGET_DIR?.trim() || path.join(repoRoot, "target")
  if (!path.isAbsolute(value)) throw new Error("CARGO_TARGET_DIR must be absolute")
  return value
}
async function availablePort() {
  for (let candidate = 54_000; candidate < 60_000; candidate += 10) {
    const servers = []
    try {
      for (let offset = 0; offset < 4; offset += 1) {
        const server = net.createServer()
        await new Promise((resolve, reject) => server.once("error", reject).listen(candidate + offset, "127.0.0.1", resolve))
        servers.push(server)
      }
      return candidate
    } catch {
      // Try the next isolated band.
    } finally {
      await Promise.all(servers.map((server) => new Promise((resolve) => server.close(resolve))))
    }
  }
  throw new Error("no empty stream-soak port band")
}
async function waitForKernel(kernelPort) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const probe = new LocalIpcClient(`ws://127.0.0.1:${kernelPort}`)
    try { await probe.send(requests.listSessionsRequest()); await probe.close(); return } catch { await probe.close().catch(() => undefined); await sleep(100) }
  }
  throw new Error("stream-soak kernel did not become ready")
}
function percentile(sorted, ratio) {
  if (!sorted.length) return 0
  return sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * ratio))]
}
function processMetric(pid) {
  const text = execFileSync("ps", ["-o", "%cpu=,rss=", "-p", String(pid)], { encoding: "utf8" }).trim()
  const [cpuPercent, rssKb] = text.split(/\s+/).map(Number)
  return { at: Date.now(), cpuPercent, rssKb }
}
async function terminateTree(rootPid) {
  const result = spawnSync("ps", ["-axo", "pid=,ppid="], { encoding: "utf8" })
  const byParent = new Map()
  for (const line of result.stdout.split("\n")) {
    const [pid, parent] = line.trim().split(/\s+/).map(Number)
    if (!Number.isInteger(pid) || !Number.isInteger(parent)) continue
    byParent.set(parent, [...(byParent.get(parent) ?? []), pid])
  }
  const descendants = []
  const queue = [rootPid]
  while (queue.length) for (const pid of byParent.get(queue.shift()) ?? []) { descendants.push(pid); queue.push(pid) }
  for (const pid of [...descendants.reverse(), rootPid]) { try { process.kill(pid, "SIGTERM") } catch {} }
  await sleep(2_000)
  const remaining = [...descendants, rootPid].filter(running)
  for (const pid of remaining) { try { process.kill(pid, "SIGKILL") } catch {} }
  await sleep(100)
  return { rootPid, descendantPids: descendants, forced: remaining, remaining: [...descendants, rootPid].filter(running) }
}
function running(pid) { try { process.kill(pid, 0); return true } catch { return false } }
