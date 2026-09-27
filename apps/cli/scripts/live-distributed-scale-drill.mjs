#!/usr/bin/env node
import assert from "node:assert/strict"
import { execFileSync, spawn, spawnSync } from "node:child_process"
import { mkdir, rm, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const args = process.argv.slice(2)
const workerCount = numberArg("--workers", 10)
const agentsPerWorker = numberArg("--agents-per-worker", 50)
const timeoutMs = numberArg("--timeout-ms", 300_000)
const output = stringArg("--output") || path.join(repoRoot, ".artifacts", "distributed-scale", `run-${process.pid}.json`)
const buildProfile = buildProfileArg()
const cargoTargetDir = cargoTargetPath()
const dryRun = args.includes("--dry-run")
const totalAgents = workerCount * agentsPerWorker
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (response, key) => response?.[key] ?? response

if (args.includes("--help")) {
  console.log("Usage: live-distributed-scale-drill.mjs [--workers 10] [--agents-per-worker 50] [--timeout-ms 300000] [--build-profile release|debug] [--output PATH] [--dry-run]")
  process.exit(0)
}

if (dryRun) {
  console.log(JSON.stringify({ workerCount, agentsPerWorker, totalAgents, buildProfile, cargoTargetDir, output, release: buildProfile === "release" }, null, 2))
  process.exit(0)
}

const { LocalIpcClient } = await import("../../../packages/kernel-client/dist/ipc.js")
const requests = await import("../../../packages/kernel-client/dist/ipc-requests.js")

const basePort = await reservePortBand(workerCount)
const ports = {
  relay: basePort,
  homeKernel: basePort + 1,
  homeMcp: basePort + 2,
  homeOpenCode: basePort + 3,
  homeCodex: basePort + 4,
  workers: Array.from({ length: workerCount }, (_, index) => ({
    kernel: basePort + 10 + index,
    mcp: basePort + 30 + index,
    opencode: basePort + 50 + index,
    codex: basePort + 70 + index,
  })),
}
const root = path.join(os.tmpdir(), `chariox-distributed-scale-${process.pid}-${Date.now()}`)
const relayToken = `distributed-scale-${process.pid}-${Date.now()}`
const relayBinary = path.join(cargoTargetDir, buildProfile, "chariox-relay")
const kernelBinary = path.join(cargoTargetDir, buildProfile, "chariox-kernel")
const children = []
let client
let report
let workerRelayStatuses = []

try {
  await mkdir(root, { recursive: true })
  children.push(start(relayBinary, relayEnv()))
  children.push(start(kernelBinary, kernelEnv("home", "home-machine", ports.homeKernel, ports.homeMcp, ports.homeOpenCode, ports.homeCodex, false)))
  for (let index = 0; index < workerCount; index += 1) {
    const worker = ports.workers[index]
    children.push(start(kernelBinary, kernelEnv(
      `worker-${index}`,
      `worker-machine-${index}`,
      worker.kernel,
      worker.mcp,
      worker.opencode,
      worker.codex,
      true,
      0,
    )))
  }

  await waitForKernel(ports.homeKernel)
  for (const worker of ports.workers) await waitForKernel(worker.kernel)
  const relayProbe = new LocalIpcClient(`ws://127.0.0.1:${ports.relay}`, {
    relayAuthToken: relayToken,
    targetDaemonAlias: "home",
  })
  try {
    await waitFor(async () => {
      await relayProbe.send(requests.listSessionsRequest())
      return true
    }, 30_000, "home kernel relay route")
  } finally {
    await relayProbe.close().catch(() => undefined)
  }
  client = new LocalIpcClient(`ws://127.0.0.1:${ports.homeKernel}`)
  for (const worker of ports.workers) {
    const workerClient = new LocalIpcClient(`ws://127.0.0.1:${worker.kernel}`)
    try {
      workerRelayStatuses.push(unwrap(await workerClient.send(requests.relayStatusRequest()), "RelayStatus"))
    } finally {
      await workerClient.close().catch(() => undefined)
    }
  }
  const workerKernels = []
  for (let index = 0; index < workerCount; index += 1) {
    workerKernels.push(await waitForWorkerKernel(client, `worker-machine-${index}`, timeoutMs))
  }

  const startedAt = Date.now()
  const created = unwrap(await client.send(requests.createSessionRequest(root, root)), "SessionCreated")
  const sessionId = created.session.id
  const attached = unwrap(await client.send(requests.attachToSessionRequest(sessionId, `distributed-scale-${process.pid}`)), "SessionAttached")
  const attachmentId = attached.attachment.id
  let completionCount = 0
  client.onKernelEvent((event) => {
    if (event?.event === "assistant_message_completed") completionCount += 1
  })
  await client.subscribeToKernelEvents(sessionId, attachmentId)

  const spawnStartedAt = Date.now()
  const spawnItems = workerKernels.flatMap((worker, workerIndex) => Array.from({ length: agentsPerWorker }, (_, agentIndex) => ({
    provider: "codex",
    alias: `scale-w${workerIndex}-a${agentIndex}`,
    model: "gpt-5.2-codex",
    worktreeId: root,
    effort: "low",
    executionMode: "build",
    permissionLevel: "yolo",
    kernelRef: worker.kernel_id,
  })))
  const spawned = unwrap(await client.send(requests.spawnAgentsRequest(sessionId, spawnItems)), "AgentsSpawned").agents
  assert.equal(spawned.length, totalAgents)
  const spawnMs = Date.now() - spawnStartedAt
  const placementCounts = new Map()
  for (const agent of spawned) {
    const workerKernelId = agent.remote_execution?.worker_kernel_id
    assert.ok(workerKernelId, `agent ${agent.id} did not receive a remote worker lease`)
    placementCounts.set(workerKernelId, (placementCounts.get(workerKernelId) ?? 0) + 1)
  }
  for (const worker of workerKernels) assert.equal(placementCounts.get(worker.kernel_id), agentsPerWorker)

  const launchStartedAt = Date.now()
  const launchRequest = requests.launchProviderRunsRequest(spawned.map((agent) => ({
    sessionId,
    provider: "codex",
    accountProfile: "default",
    model: "distributed-scale-shared-pty",
    effort: "low",
    agentId: agent.id,
    native: { nativeTui: true },
  })), 64)
  for (const launch of launchRequest.LaunchProviderRuns.launches) launch.adapter_key = "dev-stub"
  const launchResponse = unwrap(await client.send(launchRequest), "ProviderRunsLaunchAccepted")
  assert.equal(launchResponse.failures?.length ?? 0, 0, JSON.stringify(launchResponse.failures))
  assert.equal(launchResponse.provider_runs.length, totalAgents)
  const launchMs = Date.now() - launchStartedAt

  // The last run registered on each worker owns that worker's shared synthetic PTY output.
  // Sampling that run proves prompt/output/completion routing on every worker while all 500
  // logical runs remain active concurrently.
  const promptSampleByWorker = new Map()
  for (const agent of spawned) promptSampleByWorker.set(agent.remote_execution.worker_kernel_id, agent)
  const promptSample = [...promptSampleByWorker.values()]
  assert.equal(promptSample.length, workerCount)

  const promptStartedAt = Date.now()
  const promptResponse = unwrap(await client.send(requests.submitPromptsRequest(sessionId, attachmentId, promptSample.map((agent) => ({
    targetAgentId: agent.id,
    prompt: `distributed scale ${agent.alias}`,
    attachments: [],
  })), 64)), "PromptsSubmitted")
  assert.equal(promptResponse.failures?.length ?? 0, 0, JSON.stringify(promptResponse.failures))
  assert.equal(promptResponse.results.length, workerCount)
  const promptAcceptedMs = Date.now() - promptStartedAt
  await waitFor(() => completionCount >= workerCount, timeoutMs, `${workerCount} sampled remote completions`)
  const completionMs = Date.now() - promptStartedAt

  const state = unwrap(await client.send(requests.getSessionStateRequest(sessionId)), "SessionState")
  const finalAgents = state.agents ?? state.session?.agents ?? []
  const metrics = processMetrics(children)
  report = {
    ok: true,
    buildProfile,
    cargoTargetDir,
    workerCount,
    agentsPerWorker,
    totalAgents,
    sessionAgentCount: finalAgents.length,
    placementCounts: Object.fromEntries(placementCounts),
    spawnMs,
    launchMs,
    promptAcceptedMs,
    completionMs,
    totalMs: Date.now() - startedAt,
    completionCount,
    completedPromptAgents: workerCount,
    leasedRemoteAgents: totalAgents,
    runningProviderAgents: totalAgents,
    syntheticProviderProcesses: workerCount,
    providerCapacityScope: "Chariox orchestration gate; provider child-process quotas and memory are measured separately",
    homeRelayRouteProbed: true,
    metrics,
    workerRelayStatuses,
    ports,
  }
} catch (error) {
  report = { ok: false, error: String(error?.stack ?? error), workerCount, agentsPerWorker, totalAgents, buildProfile, cargoTargetDir, workerRelayStatuses, ports }
  process.exitCode = 1
} finally {
  await client?.close?.().catch(() => undefined)
  const cleanup = await terminateOwnedTree(children)
  await mkdir(path.dirname(output), { recursive: true })
  await writeFile(output, `${JSON.stringify({ ...report, cleanup }, null, 2)}\n`)
  await rm(root, { recursive: true, force: true })
  if (cleanup.remaining.length) process.exitCode = 1
}

console.log(JSON.stringify({ ...report, output }, null, 2))

function numberArg(flag, fallback) {
  const index = args.indexOf(flag)
  const value = Number(index >= 0 ? args[index + 1] : fallback)
  if (!Number.isInteger(value) || value <= 0) throw new Error(`${flag} must be a positive integer`)
  return value
}

function stringArg(flag) {
  const index = args.indexOf(flag)
  return index >= 0 ? path.resolve(args[index + 1]) : ""
}

function buildProfileArg() {
  const index = args.indexOf("--build-profile")
  const value = index >= 0 ? args[index + 1] : "release"
  if (value !== "debug" && value !== "release") throw new Error("--build-profile must be debug or release")
  return value
}

function cargoTargetPath() {
  const value = process.env.CARGO_TARGET_DIR?.trim() || path.join(repoRoot, "target")
  if (!path.isAbsolute(value)) throw new Error("CARGO_TARGET_DIR must be absolute")
  return value
}

async function reservePortBand(workers) {
  const width = 70 + workers
  for (let base = 53_000; base + width < 64_000; base += 100) {
    const candidates = [base, base + 1, base + 2, base + 3, base + 4]
    for (let index = 0; index < workers; index += 1) candidates.push(base + 10 + index, base + 30 + index, base + 50 + index, base + 70 + index)
    if (await portsAvailable(candidates)) return base
  }
  throw new Error("no empty distributed scale port band")
}

async function portsAvailable(candidates) {
  const servers = []
  try {
    for (const port of candidates) {
      const server = net.createServer()
      await new Promise((resolve, reject) => server.once("error", reject).listen(port, "127.0.0.1", resolve))
      servers.push(server)
    }
    return true
  } catch {
    return false
  } finally {
    await Promise.all(servers.map((server) => new Promise((resolve) => server.close(resolve))))
  }
}

function relayEnv() {
  return { ...process.env, CHARIOX_RELAY_HOST: "127.0.0.1", CHARIOX_RELAY_PORT: String(ports.relay), CHARIOX_RELAY_TOKEN: relayToken }
}

function kernelEnv(daemonId, machineId, kernelPort, mcpPort, opencodePort, codexPort, leases, runtimeInitDelayMs = 0) {
  const state = path.join(root, daemonId)
  return {
    ...process.env,
    HOME: state,
    XDG_CONFIG_HOME: path.join(state, "config"),
    XDG_STATE_HOME: path.join(state, "state"),
    XDG_CACHE_HOME: path.join(state, "cache"),
    CHARIOX_KERNEL_PORT: String(kernelPort),
    CHARIOX_MCP_PORT: String(mcpPort),
    CHARIOX_OPENCODE_PORT: String(opencodePort),
    CHARIOX_CODEX_PORT: String(codexPort),
    CHARIOX_RELAY_URL: `ws://127.0.0.1:${ports.relay}`,
    CHARIOX_RELAY_TOKEN: relayToken,
    CHARIOX_DAEMON_ID: daemonId,
    CHARIOX_DAEMON_ALIAS: daemonId,
    CHARIOX_MACHINE_ID: machineId,
    CHARIOX_MACHINE_ALIAS: machineId,
    CHARIOX_ACCEPT_REMOTE_LEASES: leases ? "1" : "0",
    CHARIOX_PROVIDER_RUNTIME_INIT_DELAY_MS: String(runtimeInitDelayMs),
    CHARIOX_DAEMON_SOCKET: path.join(state, "daemon.sock"),
    CHARIOX_SESSION_HISTORY_DIR: path.join(state, "history"),
  }
}

function start(command, env) {
  return spawn(command, [], { cwd: repoRoot, env, detached: true, stdio: "ignore" })
}

async function waitForKernel(port) {
  await waitFor(async () => {
    const probe = new LocalIpcClient(`ws://127.0.0.1:${port}`)
    try { await probe.send(requests.listSessionsRequest()); return true } catch { return false } finally { await probe.close().catch(() => undefined) }
  }, 30_000, "home kernel readiness")
}

async function waitForWorkerKernel(home, machineRef, timeout) {
  let found
  let machines = []
  await waitFor(async () => {
    const machineResponse = await home.send(requests.listRemoteMachinesRequest())
    machines = unwrap(machineResponse, "RemoteMachinesListed").machines ?? []
    const response = unwrap(await home.send({ ListRemoteMachineKernels: { machine_ref: machineRef } }), "RemoteMachineKernelsListed")
    found = (response.kernels ?? []).find((kernel) => kernel.accepting_remote_leases && (kernel.available_providers ?? []).includes("codex"))
    return Boolean(found)
  }, timeout, `worker ${machineRef}; machines=${JSON.stringify(machines)}`)
  return found
}

async function waitFor(predicate, timeout, label) {
  const deadline = Date.now() + timeout
  let lastError
  while (Date.now() < deadline) {
    try { if (await predicate()) return } catch (error) { lastError = error }
    await sleep(100)
  }
  throw new Error(`timed out waiting for ${label}: ${lastError ?? "no matching state"}`)
}

function processMetrics(processes) {
  const pids = processes.map((child) => child.pid).filter(Boolean)
  const text = execFileSync("ps", ["-o", "pid=,%cpu=,rss=", "-p", pids.join(",")], { encoding: "utf8" })
  return text.trim().split("\n").filter(Boolean).map((line) => {
    const [pid, cpuPercent, rssKb] = line.trim().split(/\s+/).map(Number)
    return { pid, cpuPercent, rssKb }
  })
}

async function terminateOwnedTree(roots) {
  const rootPids = roots.map((child) => child.pid).filter(Boolean)
  const descendants = descendantPids(rootPids)
  for (const pid of [...descendants, ...rootPids]) { try { process.kill(pid, "SIGTERM") } catch {} }
  await sleep(2_000)
  const remaining = [...descendants, ...rootPids].filter(running)
  for (const pid of remaining) { try { process.kill(pid, "SIGKILL") } catch {} }
  await sleep(250)
  return { rootPids, descendantPids: descendants, forced: remaining, remaining: [...descendants, ...rootPids].filter(running) }
}

function descendantPids(roots) {
  const result = spawnSync("ps", ["-axo", "pid=,ppid="], { encoding: "utf8" })
  const byParent = new Map()
  for (const line of result.stdout.split("\n")) {
    const [pid, parent] = line.trim().split(/\s+/).map(Number)
    if (!Number.isInteger(pid) || !Number.isInteger(parent)) continue
    byParent.set(parent, [...(byParent.get(parent) ?? []), pid])
  }
  const found = []
  const queue = [...roots]
  while (queue.length) for (const pid of byParent.get(queue.shift()) ?? []) { found.push(pid); queue.push(pid) }
  return found.reverse()
}

function running(pid) {
  try { process.kill(pid, 0); return true } catch { return false }
}
