#!/usr/bin/env node
import assert from "node:assert/strict"
import { execFileSync, spawn, spawnSync } from "node:child_process"
import { access, mkdir, rm, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "..")
const args = process.argv.slice(2)
const clientCount = numberArg("--clients", 32)
const cycles = numberArg("--cycles", 5)
const slowEvents = numberArg("--slow-events", 4_096)
const timeoutMs = numberArg("--timeout-ms", 30_000)
const output = reconnectStormEvidencePath(stringArg("--output"))
const cargoTargetDir = reconnectStormCargoTargetDir()
const buildProfile = reconnectStormBuildProfile()
const kernelBinary = path.join(cargoTargetDir, buildProfile, "chariox-kernel")
const relayBinary = path.join(cargoTargetDir, buildProfile, "chariox-relay")
const dryRun = args.includes("--dry-run")
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const unwrap = (response, key) => response?.[key] ?? response

if (clientCount < 2) throw new Error("--clients must be at least 2 so one slow and one healthy viewer are exercised")

if (args.includes("--help")) {
  console.log("Usage: live-reconnect-storm-drill.mjs [--clients 32] [--cycles 5] [--slow-events 4096] [--timeout-ms 30000] [--output PATH] [--dry-run]")
  process.exit(0)
}
if (dryRun) {
  console.log(JSON.stringify({
    clientCount,
    cycles,
    slowEvents,
    timeoutMs,
    output,
    cargoTargetDir,
    buildProfile,
    kernelBinary,
    relayBinary,
    release: buildProfile === "release",
  }, null, 2))
  process.exit(0)
}

const { LocalIpcClient } = await import("../../../packages/kernel-client/dist/ipc.js")
const requests = await import("../../../packages/kernel-client/dist/ipc-requests.js")
await Promise.all([
  requireExecutable(kernelBinary, "kernel"),
  requireExecutable(relayBinary, "relay"),
])

const basePort = await availablePortBand()
const ports = { relay: basePort, kernel: basePort + 1, mcp: basePort + 2, opencode: basePort + 3, codex: basePort + 4 }
const root = path.join(os.tmpdir(), `chariox-reconnect-storm-${process.pid}-${Date.now()}`)
const relayToken = `reconnect-storm-${process.pid}-${Date.now()}`
const children = []
const clients = []
const resourceSamples = []
let control
let pressureControl
let report
let resourceTimer
let receivedSignal = null
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    receivedSignal ??= signal
    for (const client of clients) client.destroy()
    control?.destroy?.()
    pressureControl?.destroy?.()
  })
}

try {
  await mkdir(root, { recursive: true })
  children.push(spawnOwned(relayBinary, relayEnv()))
  children.push(spawnOwned(kernelBinary, kernelEnv()))
  resourceTimer = setInterval(() => {
    try { resourceSamples.push({ at: Date.now(), processes: processMetrics(children) }) } catch {}
  }, 1_000)
  await waitForKernel()
  control = new LocalIpcClient(`ws://127.0.0.1:${ports.kernel}`)
  pressureControl = new LocalIpcClient(`ws://127.0.0.1:${ports.kernel}`)
  await waitFor(async () => unwrap(await control.send(requests.relayStatusRequest()), "RelayStatus")?.status?.connected === true, timeoutMs, "kernel relay connection")

  const contexts = []
  for (let index = 0; index < clientCount; index += 1) {
    throwIfInterrupted()
    const created = unwrap(await control.send(requests.createSessionRequest(root, root)), "SessionCreated")
    const attached = unwrap(
      await control.send(requests.attachToSessionRequest(created.session.id, `reconnect-storm-${process.pid}-${index}`)),
      "SessionAttached",
    )
    contexts.push({
      sessionId: created.session.id,
      agentId: created.agent.id,
      attachmentId: attached.attachment.id,
    })
  }
  const launched = unwrap(await control.send(requests.launchProviderRunsRequest(contexts.map((context) => ({
    sessionId: context.sessionId,
    provider: "dev-stub",
    accountProfile: "default",
    model: "default",
    effort: "low",
    agentId: context.agentId,
  })), clientCount)), "ProviderRunsLaunchAccepted")
  assert.equal(launched.failures?.length ?? 0, 0)
  assert.equal(launched.provider_runs.length, clientCount)
  for (const context of contexts) {
    const launchedRun = launched.provider_runs.find((entry) => entry.provider_run.session_id === context.sessionId)
    assert.ok(launchedRun, `provider run missing for ${context.sessionId}`)
    context.providerRunId = launchedRun.provider_run.id
  }
  const appendMarker = (context, marker, client = control) => client.send(requests.appendNativeProviderOutputRequest(
    context.sessionId,
    context.attachmentId,
    context.providerRunId,
    "provider_output",
    `${marker}\n`,
    marker,
  ))

  const seen = Array.from({ length: clientCount }, () => new Set())
  const providerSeen = Array.from({ length: clientCount }, () => new Set())
  const resumeCounts = Array.from({ length: clientCount }, () => 0)
  for (let index = 0; index < clientCount; index += 1) {
    const client = relayClient()
    client.onKernelEvent((event) => {
      if (event?.event === "transport_resumed") resumeCounts[index] += 1
      if (event?.event !== "terminal_output") return
      for (const record of event.records ?? []) {
        const text = Buffer.from(record.bytes ?? []).toString("utf8")
        for (const marker of text.match(/RECONNECT_STORM_[A-Z0-9_]+/g) ?? []) {
          seen[index].add(marker)
          if (record.kind === "provider_output") providerSeen[index].add(marker)
        }
      }
    })
    clients.push(client)
  }
  await Promise.all(clients.map((client, index) => client.subscribeToKernelEvents(
    contexts[index].sessionId,
    contexts[index].attachmentId,
  )))

  const baselineMarker = `RECONNECT_STORM_BASELINE_${Date.now()}`
  await Promise.all(contexts.map((context) => appendMarker(context, baselineMarker)))
  await waitFor(
    () => seen.every((markers) => markers.has(baselineMarker)),
    timeoutMs,
    `baseline output on all ${clientCount} independent attachment cursors`,
  )

  const reconnectLatenciesMs = []
  for (let cycle = 0; cycle < cycles; cycle += 1) {
    throwIfInterrupted()
    const marker = `RECONNECT_STORM_CYCLE_${cycle}_${Date.now()}`
    const startedAt = Date.now()
    const resumeBaseline = [...resumeCounts]
    await Promise.all(clients.map((client) => client.restartKernelEventStream()))
    await waitFor(
      () => resumeCounts.every((count, index) => count > resumeBaseline[index]),
      timeoutMs,
      `all ${clientCount} event streams to resume cycle ${cycle}`,
    )
    await Promise.all(contexts.map((context) => appendMarker(context, marker)))
    await waitFor(
      () => seen.every((markers) => markers.has(marker)),
      timeoutMs,
      `all ${clientCount} clients to recover cycle ${cycle}`,
    )
    reconnectLatenciesMs.push(Date.now() - startedAt)
  }

  const slowClient = clients[0]
  const slowContext = contexts[0]
  assert.ok(slowClient.eventWebsocket?._socket, "slow client event socket was not connected")
  const pressureBaselineHealth = await relayHealthSnapshot()
  assert.equal(pressureBaselineHealth.subscription_count, clientCount)
  slowClient.eventWebsocket._socket.pause()
  const payload = "s".repeat(8 * 1024)
  const pressureQueueDepth = 4
  let slowEventsSubmitted = 0
  let signalPressureReady
  let pressureSignalled = false
  let healthyProbeSettled = false
  let stopSlowFlood = false
  let slowFloodSettled = false
  let releaseHealthyProbe
  let signalSlowFloodProgress
  const pressureReady = new Promise((resolve) => { signalPressureReady = resolve })
  const healthyProbeStarted = new Promise((resolve) => { releaseHealthyProbe = resolve })
  const slowFloodProgress = new Promise((resolve) => { signalSlowFloodProgress = resolve })
  let healthyProbeStartedObserved = false
  let slowFloodProgressSignalled = false
  const slowFlood = (async () => {
    for (let offset = 0; offset < slowEvents && !stopSlowFlood;) {
      throwIfInterrupted()
      if (pressureSignalled && !healthyProbeStartedObserved) {
        await healthyProbeStarted
        healthyProbeStartedObserved = true
      }
      const count = Math.min(healthyProbeSettled ? 64 : 4, slowEvents - offset)
      await withDeadline(control.send(requests.appendNativeProviderOutputBatchRequest(
        slowContext.sessionId,
        slowContext.attachmentId,
        Array.from({ length: count }, (_, index) => {
          const eventIndex = offset + index
          return {
            providerRunId: slowContext.providerRunId,
            kind: "provider_output",
            text: payload,
            mergeKey: `slow-subscriber-${eventIndex}`,
          }
        }),
      )), timeoutMs, `slow-subscriber batch at offset ${offset}`)
      slowEventsSubmitted += count
      offset += count
      if (healthyProbeStartedObserved && !slowFloodProgressSignalled) {
        slowFloodProgressSignalled = true
        signalSlowFloodProgress()
      }
      if (!pressureSignalled) {
        const health = await relayHealthSnapshot()
        if (health.backpressure.subscription_queue_max_depth >= pressureQueueDepth) {
          pressureSignalled = true
          signalPressureReady()
        }
      }
      if (!healthyProbeSettled && !pressureSignalled) await sleep(10)
    }
  })().finally(() => { slowFloodSettled = true })
  await withDeadline(Promise.race([
    pressureReady,
    slowFlood.then(() => { throw new Error("slow flood finished before its pressure checkpoint") }),
  ]), timeoutMs, "slow-subscriber pressure checkpoint")
  const pressureObservedAtMs = Date.now()
  const healthAtPressureStart = await relayHealthSnapshot()
  assert.equal(healthAtPressureStart.subscription_count, clientCount, "slow subscription closed before the healthy probe")
  assert.equal(
    healthAtPressureStart.backpressure.slow_subscription_close_count,
    pressureBaselineHealth.backpressure.slow_subscription_close_count,
    "slow subscription was already isolated before the healthy probe",
  )
  assert.ok(
    healthAtPressureStart.backpressure.pressured_subscription_count >= 1
      && healthAtPressureStart.backpressure.subscription_queue_max_depth >= pressureQueueDepth,
    `slow subscription queue was not under observed pressure: ${JSON.stringify(healthAtPressureStart.backpressure)}`,
  )

  const healthyMarker = `RECONNECT_STORM_HEALTHY_${Date.now()}`
  const providerPrompt = `RECONNECT_STORM_PROVIDER_${Date.now()}`
  const submittedAtHealthyProbe = slowEventsSubmitted
  const healthyStartedAt = Date.now()
  let providerSubmission
  let pressureRelayStatus
  let healthyTrafficLatencyMs
  let healthAtHealthyCompletion
  let submittedAtHealthyCompletion
  let healthyProbeError
  releaseHealthyProbe()
  const healthyWork = Promise.all([
    withDeadline(
      pressureControl.send(requests.submitPromptRequest(
        contexts[1].sessionId,
        contexts[1].attachmentId,
        contexts[1].agentId,
        providerPrompt,
        [],
      )),
      timeoutMs,
      "provider prompt during slow-subscriber pressure",
    ),
    withDeadline(pressureControl.send(requests.relayStatusRequest()), timeoutMs, "kernel control during slow-subscriber pressure"),
    ...contexts.slice(1).map((context) => withDeadline(
      appendMarker(context, healthyMarker, pressureControl),
      timeoutMs,
      `healthy provider output for ${context.sessionId}`,
    )),
  ])
  try {
    await withDeadline(slowFloodProgress, timeoutMs, "slow flood progress during the healthy probe")
    const healthyResults = await healthyWork
    providerSubmission = healthyResults[0]
    pressureRelayStatus = healthyResults[1]
    assert.ok(unwrap(providerSubmission, "PromptSubmitted")?.outcome, "provider prompt was not accepted during pressure")
    assert.equal(unwrap(pressureRelayStatus, "RelayStatus")?.status?.connected, true)
    await waitFor(
      async () => {
        await withDeadline(
          pressureControl.send(requests.pumpTerminalOutputRequest(contexts[1].sessionId, contexts[1].attachmentId)),
          Math.min(timeoutMs, 5_000),
          "provider output pump during slow-subscriber pressure",
        )
        return providerSeen[1].has(providerPrompt)
      },
      timeoutMs,
      "provider output from dev-stub during slow-client pressure",
    )
    const providerCompletion = unwrap(await withDeadline(
      pressureControl.send(requests.completePromptRequest(contexts[1].sessionId)),
      timeoutMs,
      "provider turn completion during slow-subscriber pressure",
    ), "PromptCompleted")
    assert.ok(providerCompletion?.completion, "dev-stub turn did not complete during slow-client pressure")
    await waitFor(
      () => seen.slice(1).every((markers) => markers.has(healthyMarker)),
      timeoutMs,
      "healthy subscribers during slow-client pressure",
    )
    healthyTrafficLatencyMs = Date.now() - healthyStartedAt
    assert.ok(healthyTrafficLatencyMs <= timeoutMs, `healthy traffic took ${healthyTrafficLatencyMs} ms`)
    assert.ok(slowEventsSubmitted > submittedAtHealthyProbe, "slow flood did not advance during the healthy probe")
    assert.equal(slowFloodSettled, false, "slow flood finished before healthy work completed")
    submittedAtHealthyCompletion = slowEventsSubmitted
    healthAtHealthyCompletion = await relayHealthSnapshot()
    assert.equal(healthAtHealthyCompletion.subscription_count, clientCount, "slow subscription closed before healthy work completed")
    assert.equal(
      healthAtHealthyCompletion.backpressure.slow_subscription_close_count,
      pressureBaselineHealth.backpressure.slow_subscription_close_count,
      "slow subscription isolation resolved pressure before healthy work completed",
    )
    assert.ok(
      healthAtHealthyCompletion.backpressure.pressured_subscription_count >= 1
        && healthAtHealthyCompletion.backpressure.subscription_queue_max_depth > 0,
      "slow subscription queue pressure cleared before healthy work completed",
    )
  } catch (error) {
    healthyProbeError = error
  } finally {
    healthyProbeSettled = true
    stopSlowFlood = Boolean(healthyProbeError)
  }
  if (healthyProbeError) {
    await slowFlood.catch(() => undefined)
    throw healthyProbeError
  }
  await slowFlood
  let relayHealth
  let slowSubscriptionClosedAtMs
  await waitFor(async () => {
    relayHealth = await relayHealthSnapshot()
    if (relayHealth.backpressure.slow_subscription_close_count <= pressureBaselineHealth.backpressure.slow_subscription_close_count) return false
    slowSubscriptionClosedAtMs = Date.now()
    return true
  }, timeoutMs, "relay to close only the slow subscription")
  const healthyCompletedAtMs = healthyStartedAt + healthyTrafficLatencyMs
  assert.ok(slowSubscriptionClosedAtMs > healthyCompletedAtMs, "slow subscription closed before healthy traffic completed")
  const metrics = processMetrics(children)
  const resources = resourceSummary(resourceSamples, children[1].pid)
  assert.ok(resources.peakKernelRssMb <= 1_024, JSON.stringify(resources))
  assert.ok(resources.peakKernelCpuPercent <= 150, JSON.stringify(resources))
  report = {
    ok: true,
    clientCount,
    cycles,
    slowEvents,
    reconnectLatenciesMs,
    reconnectP95Ms: percentile([...reconnectLatenciesMs].sort((left, right) => left - right), 0.95),
    healthySubscribers: clientCount - 1,
    healthyTrafficLatencyMs,
    pressureObservedAtMs,
    pressureQueueDepth,
    slowEventsSubmittedAtHealthyProbe: submittedAtHealthyProbe,
    slowEventsSubmittedAtHealthyCompletion: submittedAtHealthyCompletion,
    healthAtHealthyCompletion,
    healthyCompletedAtMs,
    slowSubscriptionClosedAtMs,
    slowSubscriptionActiveThroughoutHealthyProbe: true,
    providerAcceptedDuringPressure: true,
    providerOutputCompletedDuringPressure: true,
    kernelControlHealthyDuringPressure: true,
    slowSubscriptionCloseCount: relayHealth.backpressure.slow_subscription_close_count,
    targetQueueFullCount: relayHealth.backpressure.target_queue_full_count,
    relayHealth,
    metrics,
    resources,
    ports,
  }
} catch (error) {
  report = {
    ok: false,
    error: String(error?.stack ?? error),
    clientCount,
    cycles,
    slowEvents,
    resources: resourceSummary(resourceSamples, children[1]?.pid),
    ports,
  }
  process.exitCode = 1
} finally {
  clearInterval(resourceTimer)
  for (const client of clients) client.destroy()
  await pressureControl?.close?.().catch(() => undefined)
  await control?.close?.().catch(() => undefined)
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
function stringArg(flag) { const index = args.indexOf(flag); return index >= 0 ? args[index + 1] : "" }
async function requireExecutable(file, label) {
  try {
    await access(file)
  } catch {
    throw new Error(`${label} binary is missing at ${file}; build it or select an available profile`)
  }
}
function reconnectStormEvidencePath(requested, now = new Date()) {
  const configuredRoot = process.env.CHARIOX_RECONNECT_STORM_EVIDENCE_ROOT
  if (configuredRoot && !path.isAbsolute(configuredRoot)) {
    throw new Error("CHARIOX_RECONNECT_STORM_EVIDENCE_ROOT must be absolute")
  }
  const stamp = now.toISOString().replace(/[:.]/g, "-")
  const value = requested || path.join(
    configuredRoot ?? path.join(os.homedir(), ".codex", "evidence", "browser-computer-use", "reconnect-storm"),
    stamp,
    "report.json",
  )
  if (!path.isAbsolute(value)) throw new Error("evidence report must be absolute")
  const normalized = path.normalize(value)
  const relative = path.relative(repoRoot, normalized)
  const withinRepo = relative === "" || (
    relative !== ".." && !relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative)
  )
  if (withinRepo) throw new Error("evidence must stay outside repositories")
  return normalized
}
function reconnectStormCargoTargetDir() {
  const value = process.env.CARGO_TARGET_DIR || path.join(
    os.homedir(),
    ".chariox",
    "dev",
    "browser-computer-use",
    "cargo-target",
  )
  if (!path.isAbsolute(value)) throw new Error("CARGO_TARGET_DIR must be absolute")
  return path.normalize(value)
}
function reconnectStormBuildProfile() {
  const value = process.env.CHARIOX_RECONNECT_STORM_BUILD_PROFILE?.trim() || "release"
  if (value !== "debug" && value !== "release") {
    throw new Error("CHARIOX_RECONNECT_STORM_BUILD_PROFILE must be debug or release")
  }
  return value
}
function relayClient() {
  return new LocalIpcClient(`ws://127.0.0.1:${ports.relay}`, {
    relayAuthToken: relayToken,
    targetDaemonAlias: "home",
    reconnectJitterMs: 0,
  })
}
function relayEnv() {
  return {
    ...process.env,
    CHARIOX_RELAY_HOST: "127.0.0.1",
    CHARIOX_RELAY_PORT: String(ports.relay),
    CHARIOX_RELAY_TOKEN: relayToken,
    CHARIOX_RELAY_OUTGOING_QUEUE_CAPACITY: "64",
  }
}
function kernelEnv() {
  return {
    ...process.env,
    HOME: root,
    XDG_CONFIG_HOME: path.join(root, "config"),
    XDG_STATE_HOME: path.join(root, "state"),
    XDG_CACHE_HOME: path.join(root, "cache"),
    CHARIOX_KERNEL_PORT: String(ports.kernel),
    CHARIOX_MCP_PORT: String(ports.mcp),
    CHARIOX_OPENCODE_PORT: String(ports.opencode),
    CHARIOX_CODEX_PORT: String(ports.codex),
    CHARIOX_RELAY_URL: `ws://127.0.0.1:${ports.relay}`,
    CHARIOX_RELAY_TOKEN: relayToken,
    CHARIOX_DAEMON_ID: "reconnect-storm-home",
    CHARIOX_DAEMON_ALIAS: "home",
    CHARIOX_DAEMON_SOCKET: path.join(root, "daemon.sock"),
    CHARIOX_SESSION_HISTORY_DIR: path.join(root, "history"),
  }
}
function spawnOwned(command, env) { return spawn(command, [], { cwd: repoRoot, env, detached: true, stdio: "ignore" }) }
async function availablePortBand() {
  for (let base = 55_000; base < 60_000; base += 10) {
    const servers = []
    try {
      for (let offset = 0; offset < 5; offset += 1) {
        const server = net.createServer()
        await new Promise((resolve, reject) => server.once("error", reject).listen(base + offset, "127.0.0.1", resolve))
        servers.push(server)
      }
      return base
    } catch {} finally { await Promise.all(servers.map((server) => new Promise((resolve) => server.close(resolve)))) }
  }
  throw new Error("no empty reconnect-storm port band")
}
async function waitForKernel() {
  await waitFor(async () => {
    const probe = new LocalIpcClient(`ws://127.0.0.1:${ports.kernel}`)
    try { await probe.send(requests.listSessionsRequest()); return true } catch { return false } finally { await probe.close().catch(() => undefined) }
  }, 30_000, "kernel readiness")
}
async function relayHealthSnapshot() {
  return await fetch(`http://127.0.0.1:${ports.relay}/health`).then((response) => response.json())
}
async function waitFor(predicate, timeout, label) {
  const deadline = Date.now() + timeout
  let lastError
  while (Date.now() < deadline) {
    throwIfInterrupted()
    try { if (await predicate()) return } catch (error) { lastError = error }
    await sleep(25)
  }
  throw new Error(`timed out waiting for ${label}: ${lastError ?? "condition false"}`)
}
async function withDeadline(promise, timeout, label) {
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = setTimeout(() => reject(new Error(`timed out waiting for ${label}`)), timeout)
      }),
    ])
  } finally {
    clearTimeout(timer)
  }
}
function throwIfInterrupted() {
  if (receivedSignal) throw new Error(`received ${receivedSignal}`)
}
function percentile(sorted, ratio) { return sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * ratio))] ?? 0 }
function processMetrics(processes) {
  const pids = processes.map((child) => child.pid).filter(Boolean)
  const text = execFileSync("ps", ["-o", "pid=,%cpu=,rss=", "-p", pids.join(",")], { encoding: "utf8" })
  return text.trim().split("\n").filter(Boolean).map((line) => {
    const [pid, cpuPercent, rssKb] = line.trim().split(/\s+/).map(Number)
    return { pid, cpuPercent, rssKb }
  })
}
function resourceSummary(samples, kernelPid) {
  const kernelSamples = samples.flatMap((sample) => sample.processes.filter((process) => process.pid === kernelPid))
  return {
    sampleCount: kernelSamples.length,
    peakKernelRssMb: Math.round((Math.max(0, ...kernelSamples.map((sample) => sample.rssKb)) / 1024) * 10) / 10,
    peakKernelCpuPercent: Math.max(0, ...kernelSamples.map((sample) => sample.cpuPercent)),
  }
}
async function terminateOwnedTree(roots) {
  const rootPids = roots.map((child) => child.pid).filter(Boolean)
  const descendants = descendantPids(rootPids)
  for (const pid of [...descendants, ...rootPids]) { try { process.kill(pid, "SIGTERM") } catch {} }
  await sleep(2_000)
  const forced = [...descendants, ...rootPids].filter(running)
  for (const pid of forced) { try { process.kill(pid, "SIGKILL") } catch {} }
  await sleep(250)
  return { rootPids, descendantPids: descendants, forced, remaining: [...descendants, ...rootPids].filter(running) }
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
function running(pid) { try { process.kill(pid, 0); return true } catch { return false } }
