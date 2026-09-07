#!/usr/bin/env node

import { spawn } from "node:child_process"
import { createHmac } from "node:crypto"
import { createWriteStream } from "node:fs"
import { access, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { promisify } from "node:util"
import { execFile } from "node:child_process"
import { fileURLToPath, pathToFileURL } from "node:url"

const execFileAsync = promisify(execFile)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.resolve(scriptDir, "..", "..", "..")
const kernelClientRoot = path.join(repoRoot, "packages", "kernel-client")
const RELAY_ISSUER = "chariox-room-environment-m1-drill"
const RELAY_SECRET = "chariox-room-environment-m1-drill-secret"
const RELAY_REALM = "room-environment-m1-drill"

function parseArgs(argv) {
  const options = {
    workspace: repoRoot,
    worktree: repoRoot,
    cargoTargetDir: process.env.CARGO_TARGET_DIR
      ? path.resolve(process.env.CARGO_TARGET_DIR)
      : path.join(repoRoot, "target"),
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--workspace") options.workspace = path.resolve(argv[++index])
    else if (arg === "--worktree") options.worktree = path.resolve(argv[++index])
    else if (arg === "--cargo-target-dir") options.cargoTargetDir = path.resolve(argv[++index])
    else if (arg === "--help") options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/live-room-environment-m1-drill.mjs [options]",
    "",
    "Options:",
    "  --workspace PATH",
    "  --worktree PATH",
    "  --cargo-target-dir PATH",
  ].join("\n"))
}

function base64url(input) {
  return Buffer.from(input).toString("base64url")
}

function signRelayToken(claims) {
  const payload = base64url(JSON.stringify(claims))
  const signature = createHmac("sha256", RELAY_SECRET).update(payload).digest("base64url")
  return `chariox-scoped-v1.${payload}.${signature}`
}

function relayClaims({ subject, subjectKind, actions, userId }) {
  return {
    issuer: RELAY_ISSUER,
    subject,
    subject_kind: subjectKind,
    realm_id: RELAY_REALM,
    allowed_actions: actions,
    allowed_targets: null,
    issued_at_ms: Date.now(),
    expires_at_ms: Date.now() + 10 * 60_000,
    token_id: `${subject}-${Date.now()}`,
    account_id: "room-environment-m1-drill-account",
    organization_id: null,
    user_id: userId,
    device_id: subject,
    machine_id: subjectKind === "kernel" ? subject : null,
    client_id: subjectKind === "client" ? subject : null,
    public_key_thumbprint: `${subject}-thumbprint`,
    entitlements_version: "drill",
  }
}

function makePorts() {
  const base = 48000 + Math.floor(Math.random() * 800)
  return {
    relay: base,
    homeKernel: base + 1000,
    workerKernel: base + 1001,
    homeMcp: base + 2000,
    workerMcp: base + 2001,
    homeOpenCode: base + 3000,
    workerOpenCode: base + 3001,
    homeCodex: base + 3002,
    workerCodex: base + 3003,
  }
}

function kernelEnv({ ports, stateRoot, identity, acceptRemoteLeases }) {
  const token = signRelayToken(relayClaims({
    subject: identity.daemonId,
    subjectKind: "kernel",
    actions: [
      "daemon_register",
      "daemon_heartbeat",
      "peer_request",
      "peer_event",
      "client_connect",
      "client_metadata_read",
      "packet_route",
    ],
    userId: "user-1",
  }))
  const kernelRoot = path.join(stateRoot, identity.alias)
  return {
    ...process.env,
    CHARIOX_HOME: kernelRoot,
    CHARIOX_KERNEL_PORT: String(identity.kernelPort),
    CHARIOX_MCP_PORT: String(identity.mcpPort),
    CHARIOX_OPENCODE_PORT: String(identity.openCodePort),
    CHARIOX_CODEX_PORT: String(identity.codexPort),
    CHARIOX_RELAY_URL: `ws://127.0.0.1:${ports.relay}`,
    CHARIOX_RELAY_TOKEN: token,
    CHARIOX_DAEMON_ID: identity.daemonId,
    CHARIOX_DAEMON_ALIAS: identity.alias,
    CHARIOX_MACHINE_ID: identity.machineId,
    CHARIOX_MACHINE_ALIAS: identity.machineAlias,
    CHARIOX_ACCEPT_REMOTE_LEASES: acceptRemoteLeases ? "1" : "0",
    CHARIOX_PROVIDER_DEV_STUB: "1",
    CHARIOX_DAEMON_SOCKET: path.join(kernelRoot, "daemon.sock"),
    CHARIOX_SESSION_HISTORY_DIR: path.join(kernelRoot, "history"),
    XDG_CONFIG_HOME: path.join(kernelRoot, "xdg-config"),
    XDG_STATE_HOME: path.join(kernelRoot, "xdg-state"),
    XDG_CACHE_HOME: path.join(kernelRoot, "xdg-cache"),
  }
}

function clientToken(userId) {
  return signRelayToken(relayClaims({
    subject: `client-${userId}-${process.pid}`,
    subjectKind: "client",
    actions: ["client_connect", "client_metadata_read", "packet_route"],
    userId,
  }))
}

function clientFor(LocalIpcClient, relayUrl, daemonAlias, userId) {
  return new LocalIpcClient(relayUrl, {
    relayAuthToken: clientToken(userId),
    targetDaemonAlias: daemonAlias,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
}

function spawnObserved(label, binary, env, evidenceRoot) {
  const log = createWriteStream(path.join(evidenceRoot, `${label}.log`), { flags: "a" })
  const child = spawn(binary, [], { cwd: repoRoot, env, stdio: ["ignore", "pipe", "pipe"] })
  child.stdout.pipe(log)
  child.stderr.pipe(log)
  child.once("exit", () => log.end())
  return child
}

async function terminateChild(child) {
  if (!child || child.exitCode != null) return
  child.kill("SIGTERM")
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([
      new Promise((resolve) => child.once("exit", resolve)),
      sleep(2_000),
    ])
  }
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function requireCondition(condition, message, detail = null) {
  if (condition) return
  const suffix = detail == null ? "" : `\n${JSON.stringify(detail, null, 2)}`
  throw new Error(`${message}${suffix}`)
}

function unwrap(response, key) {
  return response?.[key] ?? response
}

async function waitForTarget(LocalIpcClient, relayUrl, daemonAlias) {
  let lastError = "unknown error"
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = clientFor(LocalIpcClient, relayUrl, daemonAlias, "user-1")
    try {
      await Promise.race([
        client.send({ ListSessions: null }),
        sleep(2_000).then(() => { throw new Error("probe timeout") }),
      ])
      await client.close()
      return
    } catch (error) {
      lastError = error instanceof Error ? error.message : String(error)
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target ${daemonAlias} did not become reachable: ${lastError}`)
}

async function waitForWorkerKernel(client, listRemoteMachinesRequest, listRemoteMachineKernelsRequest, machineId) {
  let lastKernels = []
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const machines = unwrap(await client.send(listRemoteMachinesRequest()), "RemoteMachinesListed").machines ?? []
    if (machines.some((machine) => machine.machine_id === machineId)) {
      lastKernels = unwrap(
        await client.send(listRemoteMachineKernelsRequest(machineId)),
        "RemoteMachineKernelsListed",
      ).kernels ?? []
      const worker = lastKernels.find((kernel) =>
        kernel.accepting_remote_leases && (kernel.available_providers ?? []).includes("dev-stub"))
      if (worker) return worker
    }
    await sleep(250)
  }
  throw new Error(`worker did not advertise dev-stub: ${JSON.stringify(lastKernels)}`)
}

async function resourceSnapshot(label, pids = []) {
  const snapshot = {
    label,
    at: new Date().toISOString(),
    host: {
      totalMemoryBytes: os.totalmem(),
      freeMemoryBytes: os.freemem(),
      loadAverage: os.loadavg(),
      cpuCount: os.cpus().length,
    },
    processes: [],
    processCount: null,
    memoryPressure: null,
    swapUsage: null,
    disk: null,
  }
  if (pids.length > 0) {
    const { stdout } = await execFileAsync("ps", ["-o", "pid=,rss=,%cpu=,comm=", "-p", pids.join(",")])
    snapshot.processes = stdout.trim().split("\n").filter(Boolean)
  }
  const { stdout: disk } = await execFileAsync("df", ["-k", repoRoot])
  snapshot.disk = disk.trim().split("\n").at(-1)
  const { stdout: processIds } = await execFileAsync("ps", ["-A", "-o", "pid="])
  snapshot.processCount = processIds.trim().split("\n").filter(Boolean).length
  snapshot.memoryPressure = await execFileAsync("memory_pressure", ["-Q"])
    .then(({ stdout }) => stdout.trim())
    .catch(() => null)
  snapshot.swapUsage = await execFileAsync("sysctl", ["-n", "vm.swapusage"])
    .then(({ stdout }) => stdout.trim())
    .catch(() => null)
  return snapshot
}

async function cargoPackageVersion(manifestPath) {
  const manifest = await readFile(manifestPath, "utf8")
  const match = manifest.match(/^version\s*=\s*"([^"]+)"/m)
  if (!match) throw new Error(`missing package version in ${manifestPath}`)
  return match[1]
}

async function assertPortsReleased(ports) {
  const occupied = []
  for (const port of Object.values(ports)) {
    try {
      const { stdout } = await execFileAsync("lsof", ["-nP", `-iTCP:${port}`, "-sTCP:LISTEN"])
      if (stdout.trim()) occupied.push({ port, listeners: stdout.trim() })
    } catch {
      // lsof exits non-zero when no listener matches.
    }
  }
  requireCondition(occupied.length === 0, "drill-owned listeners remained after cleanup", occupied)
}

async function resolveBinary(cargoTargetDir, name) {
  const binary = path.join(cargoTargetDir, "debug", name)
  await access(binary).catch(() => {
    throw new Error(`missing ${binary}; build chariox-kernel and chariox-relay in the selected Cargo target first`)
  })
  return binary
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }

  const startedAt = new Date().toISOString()
  const runId = startedAt.replace(/[:.]/g, "-")
  const evidenceRoot = path.join(os.homedir(), ".codex", "evidence", "browser-computer-use", "m1", runId)
  const stateRoot = await mkdtemp(path.join(os.tmpdir(), "chariox-room-environment-m1-"))
  await mkdir(evidenceRoot, { recursive: true })

  const ports = makePorts()
  const home = {
    daemonId: `room-environment-home-${process.pid}`,
    alias: `room-environment-home-${process.pid}`,
    machineId: `room-environment-home-machine-${process.pid}`,
    machineAlias: `room-environment-home-machine-${process.pid}`,
    kernelPort: ports.homeKernel,
    mcpPort: ports.homeMcp,
    openCodePort: ports.homeOpenCode,
    codexPort: ports.homeCodex,
  }
  const worker = {
    daemonId: `room-environment-worker-${process.pid}`,
    alias: `room-environment-worker-${process.pid}`,
    machineId: `room-environment-worker-machine-${process.pid}`,
    machineAlias: `room-environment-worker-machine-${process.pid}`,
    kernelPort: ports.workerKernel,
    mcpPort: ports.workerMcp,
    openCodePort: ports.workerOpenCode,
    codexPort: ports.workerCodex,
  }
  const relayUrl = `ws://127.0.0.1:${ports.relay}`
  const children = []
  const clients = []
  const resources = [await resourceSnapshot("before")]
  const assertions = []
  const { stdout: commitOutput } = await execFileAsync("git", ["rev-parse", "HEAD"], { cwd: repoRoot })
  const ossCommit = commitOutput.trim()
  const versions = {
    node: process.version,
    kernel: await cargoPackageVersion(path.join(repoRoot, "apps", "kernel", "Cargo.toml")),
    relay: await cargoPackageVersion(path.join(repoRoot, "apps", "relay", "Cargo.toml")),
    os: `${os.platform()} ${os.release()} ${os.arch()}`,
  }
  let sessionId = null
  let endSessionRequest = null
  let failure = null
  let report = null

  try {
    const [{ LocalIpcClient }, requests, kernelTypes] = await Promise.all([
      import(pathToFileURL(path.join(kernelClientRoot, "dist", "ipc.js")).href),
      import(pathToFileURL(path.join(kernelClientRoot, "dist", "ipc-requests.js")).href),
      import(pathToFileURL(path.join(kernelClientRoot, "dist", "kernel-types.js")).href),
    ])
    ;({ endSessionRequest } = requests)
    const relayBinary = await resolveBinary(options.cargoTargetDir, "chariox-relay")
    const kernelBinary = await resolveBinary(options.cargoTargetDir, "chariox-kernel")
    const relay = spawnObserved("relay", relayBinary, {
      ...process.env,
      CHARIOX_RELAY_HOST: "127.0.0.1",
      CHARIOX_RELAY_PORT: String(ports.relay),
      CHARIOX_RELAY_SCOPED_ISSUER: RELAY_ISSUER,
      CHARIOX_RELAY_SCOPED_HMAC_SECRET: RELAY_SECRET,
    }, evidenceRoot)
    const homeKernel = spawnObserved(
      "home-kernel",
      kernelBinary,
      kernelEnv({ ports, stateRoot, identity: home, acceptRemoteLeases: false }),
      evidenceRoot,
    )
    const workerKernel = spawnObserved(
      "worker-kernel",
      kernelBinary,
      kernelEnv({ ports, stateRoot, identity: worker, acceptRemoteLeases: true }),
      evidenceRoot,
    )
    children.push(relay, homeKernel, workerKernel)

    await waitForTarget(LocalIpcClient, relayUrl, home.alias)
    await waitForTarget(LocalIpcClient, relayUrl, worker.alias)
    const user1 = clientFor(LocalIpcClient, relayUrl, home.alias, "user-1")
    let user2 = clientFor(LocalIpcClient, relayUrl, home.alias, "user-2")
    clients.push(user1, user2)
    resources.push(await resourceSnapshot("kernels-ready", children.map((child) => child.pid)))

    const session = unwrap(
      await user1.send(requests.createSessionRequest(options.workspace, options.worktree)),
      "SessionCreated",
    ).session
    sessionId = session.id
    const invite = unwrap(
      await user1.send(requests.createSessionInviteRequest(sessionId, null, 1)),
      "SessionInviteCreated",
    ).invite
    await user2.send(requests.joinSessionInviteRequest(invite.invite_token, "user-2"))

    const started = unwrap(
      await user1.send(requests.startRoomEnvironmentRequest(sessionId, {
        css_width: 1280,
        css_height: 800,
        device_scale_factor: 1,
        desktop_pixel_width: 1280,
        desktop_pixel_height: 800,
      })),
      "RoomEnvironmentUpdated",
    ).environment
    requireCondition(started.lifecycle === "starting", "M1 Environment should enter starting", started)

    const advertisedWorker = await waitForWorkerKernel(
      user1,
      requests.listRemoteMachinesRequest,
      requests.listRemoteMachineKernelsRequest,
      worker.machineId,
    )
    const localAgent = unwrap(await user1.send({
      SpawnAgent: {
        session_id: sessionId,
        provider: "dev-stub",
        alias: "local-observer",
        model: "default",
        effort: "low",
        worktree_id: options.worktree,
        kernel_ref: null,
      },
    }), "AgentSpawned").agent
    const remoteAgent = unwrap(await user2.send({
      SpawnAgent: {
        session_id: sessionId,
        provider: "dev-stub",
        alias: "worker-observer",
        model: "default",
        effort: "low",
        worktree_id: options.worktree,
        kernel_ref: advertisedWorker.kernel_id,
      },
    }), "AgentSpawned").agent
    requireCondition(
      remoteAgent.remote_execution?.worker_kernel_id === advertisedWorker.kernel_id,
      "second agent should be worker-backed",
      remoteAgent,
    )

    const user1Snapshot = unwrap(
      await user1.send(requests.getRoomEnvironmentStateRequest(sessionId)),
      "RoomEnvironmentState",
    ).environment
    const user2Snapshot = unwrap(
      await user2.send(requests.getRoomEnvironmentStateRequest(sessionId)),
      "RoomEnvironmentState",
    ).environment
    requireCondition(
      JSON.stringify(user1Snapshot) === JSON.stringify(user2Snapshot),
      "both clients should observe the same authoritative Environment snapshot",
      { user1Snapshot, user2Snapshot },
    )
    requireCondition(user1Snapshot.environment_id === started.environment_id, "Environment identity changed", user1Snapshot)
    requireCondition(
      JSON.stringify(user1Snapshot.viewport) === JSON.stringify(started.viewport),
      "kernel-owned canonical viewport diverged",
      user1Snapshot.viewport,
    )
    requireCondition(
      user1Snapshot.actors.some((actor) => actor.actor_id === "user:user-1"),
      "session owner should remain represented in Environment actor history",
      user1Snapshot.actors,
    )
    for (const actorId of [`agent:${localAgent.id}`, `agent:${remoteAgent.id}`]) {
      requireCondition(
        user1Snapshot.actors.some((actor) => actor.actor_id === actorId && actor.presence === "present"),
        `missing present Environment actor ${actorId}`,
        user1Snapshot.actors,
      )
    }
    assertions.push(
      "one Environment identity across home and worker-backed agents",
      "two authenticated clients observe the same snapshot",
      "kernel-owned canonical viewport is identical for both clients",
      "two agent actors are present in the authoritative projection",
    )

    const replay = unwrap(
      await user1.send(requests.getRoomEnvironmentEventsRequest(sessionId, 0)),
      "RoomEnvironmentEvents",
    ).replay
    requireCondition(replay.Events?.events?.length > 0, "event replay should contain ordered Environment changes", replay)
    const nextCursor = replay.Events.next_cursor
    const history = unwrap(
      await user1.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, null, 25)),
      "RoomEnvironmentActionHistoryListed",
    ).page
    requireCondition(history.actions.length === 0, "M1 drill should not fabricate controller Actions", history)
    assertions.push("event replay and authenticated v279 history reads cross the relay boundary")

    await user2.close()
    clients.pop()
    user2 = clientFor(LocalIpcClient, relayUrl, home.alias, "user-2")
    clients.push(user2)
    const reconnectedSnapshot = unwrap(
      await user2.send(requests.getRoomEnvironmentStateRequest(sessionId)),
      "RoomEnvironmentState",
    ).environment
    requireCondition(
      JSON.stringify(reconnectedSnapshot) === JSON.stringify(user1Snapshot),
      "reconnected client should recover the same authoritative snapshot",
      reconnectedSnapshot,
    )
    const replayAfterReconnect = unwrap(
      await user2.send(requests.getRoomEnvironmentEventsRequest(sessionId, nextCursor)),
      "RoomEnvironmentEvents",
    ).replay
    requireCondition(
      replayAfterReconnect.Events?.events?.length === 0,
      "reconnect at the current cursor should not duplicate Environment events",
      replayAfterReconnect,
    )
    assertions.push("client reconnect preserves snapshot identity and does not duplicate events")
    resources.push(await resourceSnapshot("active", children.map((child) => child.pid)))

    const finalHistory = unwrap(
      await user2.send(requests.listRoomEnvironmentActionHistoryRequest(sessionId, null, 25)),
      "RoomEnvironmentActionHistoryListed",
    ).page
    requireCondition(finalHistory.actions.length === 0, "history changed during reconnect", finalHistory)

    report = {
      schema: "chariox.room_environment.m1_drill.v1",
      status: "passed",
      startedAt,
      finishedAt: new Date().toISOString(),
      ossCommit,
      protocolVersion: kernelTypes.LOCAL_DAEMON_PROTOCOL_VERSION,
      versions,
      command: "pnpm --filter @chariox/cli run room-environment:m1-drill",
      topology: "same-host relay, home kernel, worker kernel, two authenticated clients",
      machine: "local development Mac",
      provider: "dev-stub",
      sessionId,
      environmentId: user1Snapshot.environment_id,
      localAgentId: localAgent.id,
      workerAgentId: remoteAgent.id,
      workerKernelId: advertisedWorker.kernel_id,
      eventCursor: nextCursor,
      assertions,
      artifacts: ["relay.log", "home-kernel.log", "worker-kernel.log", "cleanup.json"],
      resources,
      cleanup: { stateRootRemoved: false, listenersReleased: false },
    }
  } catch (error) {
    failure = error
  } finally {
    if (sessionId && clients[0] && endSessionRequest) {
      await clients[0].send(endSessionRequest(sessionId)).catch(() => {})
    }
    await Promise.all(clients.map((client) => client.close().catch(() => {})))
    for (const child of children.reverse()) await terminateChild(child)
    let listenersReleased = true
    await assertPortsReleased(ports).catch((error) => {
      listenersReleased = false
      failure ??= error
    })
    await rm(stateRoot, { recursive: true, force: true })
    const after = await resourceSnapshot("after")
    report ??= {
      schema: "chariox.room_environment.m1_drill.v1",
      status: "failed",
      startedAt,
      finishedAt: new Date().toISOString(),
      ossCommit,
      protocolVersion: null,
      versions,
      command: "pnpm --filter @chariox/cli run room-environment:m1-drill",
      topology: "same-host relay, home kernel, worker kernel, two authenticated clients",
      machine: "local development Mac",
      provider: "dev-stub",
      sessionId,
      assertions,
      artifacts: ["relay.log", "home-kernel.log", "worker-kernel.log", "cleanup.json"],
      resources,
      cleanup: { stateRootRemoved: false, listenersReleased: false },
    }
    report.status = failure == null ? "passed" : "failed"
    report.finishedAt = new Date().toISOString()
    report.failedAssertion = failure instanceof Error ? failure.message : failure
    report.resources.push(after)
    report.cleanup = { stateRootRemoved: true, listenersReleased }
    await writeFile(path.join(evidenceRoot, "report.json"), `${JSON.stringify(report, null, 2)}\n`, "utf8")
    await writeFile(path.join(evidenceRoot, "cleanup.json"), `${JSON.stringify({
      finishedAt: new Date().toISOString(),
      stateRootRemoved: true,
      listenersReleased,
      resource: after,
      failure: failure instanceof Error ? failure.message : failure,
    }, null, 2)}\n`, "utf8")
  }
  if (failure) throw failure
  console.log(JSON.stringify({ status: "passed", evidenceRoot, assertions }, null, 2))
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
