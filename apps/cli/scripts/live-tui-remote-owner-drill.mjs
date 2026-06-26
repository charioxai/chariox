#!/usr/bin/env node
import { spawn } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { access, mkdir, rm } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const DEFAULT_TIMEOUT_MS = 120_000

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, "-")
}

function parseArgs(argv) {
  const options = {
    workspace: repoRoot,
    worktree: repoRoot,
    timeoutMs: DEFAULT_TIMEOUT_MS,
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--workspace") options.workspace = argv[++i]
    else if (arg === "--worktree") options.worktree = argv[++i]
    else if (arg === "--timeout-ms") options.timeoutMs = Number(argv[++i])
    else if (arg === "--help") options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function makePorts() {
  const base = 49000 + Math.floor(Math.random() * 1000)
  return {
    relayPort: base,
    localKernelPort: base + 1000,
    peerKernelPort: base + 1001,
    localMcpPort: base + 2000,
    peerMcpPort: base + 2001,
    localOpenCodePort: base + 3000,
    peerOpenCodePort: base + 3001,
    localCodexPort: base + 3002,
    peerCodexPort: base + 3003,
  }
}

function makeKernelEnv({
  ports,
  rootDir,
  relayToken,
  daemonId,
  daemonAlias,
  machineId,
  machineAlias,
  acceptRemoteLeases,
  socketName,
  kernelPort,
  mcpPort,
  opencodePort,
  codexPort,
}) {
  return {
    ...process.env,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(opencodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
    ARROBA_RELAY_TOKEN: relayToken,
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_MACHINE_ID: machineId,
    ARROBA_MACHINE_ALIAS: machineAlias,
    ARROBA_ACCEPT_REMOTE_LEASES: acceptRemoteLeases ? "1" : "0",
    ARROBA_DAEMON_SOCKET: path.join(rootDir, socketName),
    ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, `${daemonId}-history`),
    XDG_CONFIG_HOME: path.join(rootDir, `${daemonId}-xdg-config`),
    XDG_STATE_HOME: path.join(rootDir, `${daemonId}-xdg-state`),
    XDG_CACHE_HOME: path.join(rootDir, `${daemonId}-xdg-cache`),
  }
}

async function resolveBinary(binaryPath, manifestPath, binName) {
  try {
    await access(binaryPath)
    return binaryPath
  } catch {
    throw new Error(`missing built binary ${binaryPath}; run cargo build --manifest-path ${manifestPath} --bin ${binName} first`)
  }
}

function spawnProcess(command, args, options) {
  return spawn(command, args, { ...options, stdio: ["ignore", "ignore", "pipe"] })
}

async function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode != null) return
  child.kill(signal)
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

async function waitForSocket(socketPath) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const client = net.createConnection(socketPath)
      await new Promise((resolve, reject) => {
        client.once("connect", resolve)
        client.once("error", reject)
      })
      client.destroy()
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error(`automation socket did not become ready: ${lastError?.message ?? lastError}`)
}

function createAutomationClient(socketPath) {
  const socket = net.createConnection(socketPath)
  socket.setEncoding("utf8")
  let nextId = 1
  let buffer = ""
  const pending = new Map()
  socket.on("data", (chunk) => {
    buffer += chunk
    while (buffer.includes("\n")) {
      const newline = buffer.indexOf("\n")
      const line = buffer.slice(0, newline).trim()
      buffer = buffer.slice(newline + 1)
      if (!line) continue
      const response = JSON.parse(line)
      const deferred = pending.get(response.id)
      if (!deferred) continue
      pending.delete(response.id)
      if (response.ok) deferred.resolve(response.data)
      else deferred.reject(new Error(response.error ?? "automation command failed"))
    }
  })
  socket.on("error", (error) => {
    for (const deferred of pending.values()) deferred.reject(error)
    pending.clear()
  })
  return {
    send(action, fields = {}) {
      const id = nextId++
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(`${JSON.stringify({ id, action, ...fields })}\n`)
      })
    },
    close() {
      socket.destroy()
    },
  }
}

function unwrapVariant(response, ...keys) {
  for (const key of keys) {
    if (response?.[key] != null) return response[key]
  }
  return response
}

async function waitForKernel(client, workspace, worktree) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      const created = unwrapVariant(
        await client.send({ CreateSession: { workspace_id: workspace, worktree_id: worktree } }),
        "SessionCreated",
      )
      await client.send({ EndSession: { session_id: created.session.id } }).catch(() => {})
      return
    } catch {
      await sleep(250)
    }
  }
  throw new Error("kernel did not become ready")
}

async function waitForRemoteKernel(client, machineRef) {
  let last = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      const listed = unwrapVariant(
        await client.send({ ListRemoteMachineKernels: { machine_ref: machineRef } }),
        "RemoteMachineKernelsListed",
      )
      if ((listed.kernels ?? []).length > 0) return listed.kernels
    } catch (error) {
      last = error
    }
    await sleep(500)
  }
  throw new Error(`remote kernel ${machineRef} did not become visible: ${last?.message ?? last}`)
}

async function waitForSessionCount(client, expectedCount, label, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const listed = unwrapVariant(await client.send({ ListSessions: null }), "SessionsListed")
    last = listed.sessions ?? []
    if (last.length === expectedCount) return last
    await sleep(250)
  }
  throw new Error(`${label} expected ${expectedCount} sessions, saw ${last?.length ?? "unknown"}: ${JSON.stringify(last)}`)
}

async function attachForDrill(client, sessionId, clientId) {
  return unwrapVariant(
    await client.send({
      AttachToSession: {
        session_id: sessionId,
        client_id: clientId,
        capability_level: "FullTerminal",
      },
    }),
    "SessionAttached",
  ).attachment
}

async function promptRemoteWorker({ client, sessionId, attachmentId, agentId, marker, events, timeoutMs }) {
  const baselineCompletions = events.filter((event) => event.event === "assistant_message_completed").length
  const submitted = unwrapVariant(
    await client.send({
      SubmitPrompt: {
        session_id: sessionId,
        attachment_id: attachmentId,
        target_agent_id: agentId,
        prompt: `Reply with exactly ${marker}.`,
        attachments: [],
      },
    }),
    "PromptSubmitted",
  )
  const startedPrompt = submitted.outcome?.Started?.prompt ?? submitted.outcome?.started?.prompt ?? null
  if (!startedPrompt?.id) {
    throw new Error(`worker prompt did not start: ${JSON.stringify(submitted, null, 2)}`)
  }
  const deadline = Date.now() + timeoutMs
  let completeAttempted = false
  let lastCompleteError = null
  while (Date.now() < deadline) {
    await client.send({
      PumpTerminalOutput: {
        session_id: sessionId,
        attachment_id: attachmentId,
      },
    }).catch(() => {})
    const completions = events.filter((event) => event.event === "assistant_message_completed")
    if (completions.length > baselineCompletions) {
      return completions.at(-1)
    }
    if (!completeAttempted) {
      completeAttempted = true
      await client.send({ CompletePrompt: { session_id: sessionId } }).catch((error) => {
        lastCompleteError = error
        completeAttempted = false
      })
    }
    await sleep(250)
  }
  throw new Error(`timed out waiting for worker prompt ${startedPrompt.id}; lastCompleteError=${lastCompleteError?.message ?? lastCompleteError}`)
}

async function waitForAgentPlacement(client, sessionId, agentId, predicate, label, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let lastAgent = null
  while (Date.now() < deadline) {
    const state = unwrapVariant(
      await client.send({ GetSessionState: { session_id: sessionId } }),
      "SessionStateLoaded",
      "SessionState",
    )
    lastAgent = state.session?.agents?.find((agent) => agent.id === agentId) ?? null
    if (lastAgent && predicate(lastAgent)) return lastAgent
    await sleep(250)
  }
  throw new Error(`${label} did not reach expected placement: ${JSON.stringify(lastAgent, null, 2)}`)
}

async function waitForAutomationSnapshot(automation, predicate, label, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await automation.send("snapshot")
    if (predicate(last)) return last
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${label}\nlast snapshot:\n${JSON.stringify(last, null, 2)}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    console.log([
      "Usage: node apps/cli/scripts/live-tui-remote-owner-drill.mjs [options]",
      `  --workspace ${options.workspace}`,
      `  --worktree ${options.worktree}`,
      `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    ].join("\n"))
    return
  }

  const ports = makePorts()
  const rootDir = path.join(repoRoot, ".artifacts", "live-tui-remote-owner-drill", nowStamp())
  const automationSocket = path.join("/tmp", `arroba-tui-owner-${process.pid}.sock`)
  const peerAutomationSocket = path.join("/tmp", `arroba-tui-owner-peer-${process.pid}.sock`)
  await prepareDrillArtifacts(rootDir)
  await mkdir(rootDir, { recursive: true })

  const relayToken = `tui-remote-owner-${process.pid}-${Date.now()}`
  const localDaemonId = `tui-owner-local-${process.pid}-${Date.now()}`
  const peerDaemonId = `tui-owner-peer-${process.pid}-${Date.now()}`
  const localMachineId = `tui-owner-local-machine-${process.pid}`
  const peerMachineId = `tui-owner-peer-machine-${process.pid}`
  const localEnv = makeKernelEnv({
    ports,
    rootDir,
    relayToken,
    daemonId: localDaemonId,
    daemonAlias: "local-tui",
    machineId: localMachineId,
    machineAlias: "local-tui-machine",
    acceptRemoteLeases: true,
    socketName: "local-daemon.sock",
    kernelPort: ports.localKernelPort,
    mcpPort: ports.localMcpPort,
    opencodePort: ports.localOpenCodePort,
    codexPort: ports.localCodexPort,
  })
  const peerEnv = makeKernelEnv({
    ports,
    rootDir,
    relayToken,
    daemonId: peerDaemonId,
    daemonAlias: "peer-tui",
    machineId: peerMachineId,
    machineAlias: "peer-tui-machine",
    acceptRemoteLeases: true,
    socketName: "peer-daemon.sock",
    kernelPort: ports.peerKernelPort,
    mcpPort: ports.peerMcpPort,
    opencodePort: ports.peerOpenCodePort,
    codexPort: ports.peerCodexPort,
  })
  const relayEnv = {
    ...process.env,
    ARROBA_RELAY_HOST: "127.0.0.1",
    ARROBA_RELAY_PORT: String(ports.relayPort),
    ARROBA_RELAY_TOKEN: relayToken,
  }

  const relayBinary = await resolveBinary(
    path.join(repoRoot, "apps/relay/target/debug/arroba-relay"),
    path.join(repoRoot, "apps/relay/Cargo.toml"),
    "arroba-relay",
  )
  const kernelBinary = await resolveBinary(
    path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel"),
    path.join(repoRoot, "apps/kernel/Cargo.toml"),
    "arroba-kernel",
  )

  const { LocalIpcClient } = await import("../dist/ipc.js")
  let relayChild = null
  let localKernelChild = null
  let peerKernelChild = null
  let tuiChild = null
  let peerTuiChild = null
  let automation = null
  let peerAutomation = null
  let localClient = null
  let peerClient = null
  let passed = false
  let failure = null
  let tuiStdout = ""
  let tuiStderr = ""
  let peerTuiStdout = ""
  let peerTuiStderr = ""
  let selectedKernel = null
  let sessionSnapshot = null
  let workerAgent = null
  let workerAgentFinal = null
  let workerCompletion = null

  try {
    relayChild = spawnProcess(relayBinary, [], { cwd: repoRoot, env: relayEnv })
    localKernelChild = spawnProcess(kernelBinary, [], { cwd: repoRoot, env: localEnv })
    peerKernelChild = spawnProcess(kernelBinary, [], { cwd: repoRoot, env: peerEnv })

    localClient = new LocalIpcClient(`ws://127.0.0.1:${ports.localKernelPort}`)
    peerClient = new LocalIpcClient(`ws://127.0.0.1:${ports.peerKernelPort}`)
    await waitForKernel(localClient, options.workspace, options.worktree)
    await waitForKernel(peerClient, options.workspace, options.worktree)
    const peerKernels = await waitForRemoteKernel(localClient, peerMachineId)
    selectedKernel = peerKernels.find((kernel) => kernel.kernel_id === peerDaemonId)
    if (!selectedKernel) {
      throw new Error(`peer kernel ${peerDaemonId} not found: ${JSON.stringify(peerKernels, null, 2)}`)
    }

    const cliArgs = [
      "-q",
      "/dev/null",
      "env",
      ...Object.entries(localEnv).map(([key, value]) => `${key}=${value}`),
      "bun",
      path.join(repoRoot, "apps/cli/dist/index.js"),
      "--kernel-url", `ws://127.0.0.1:${ports.localKernelPort}`,
      "--automation-socket", automationSocket,
      "--workspace", options.workspace,
      "--worktree", options.worktree,
      "--provider", "dev-stub",
      "--model", "tui-remote-owner-drill-model",
      "--client-id", `tui-remote-owner-${process.pid}`,
    ]
    tuiChild = spawn("script", cliArgs, { cwd: repoRoot, env: localEnv, stdio: ["ignore", "pipe", "pipe"] })
    tuiChild.stdout.on("data", (chunk) => {
      tuiStdout += chunk.toString()
      if (tuiStdout.length > 16_000) tuiStdout = tuiStdout.slice(-16_000)
    })
    tuiChild.stderr.on("data", (chunk) => {
      tuiStderr += chunk.toString()
      if (tuiStderr.length > 16_000) tuiStderr = tuiStderr.slice(-16_000)
    })
    const startupFailure = new Promise((resolve) => {
      tuiChild.once("error", (error) => resolve(error))
      tuiChild.once("exit", (code, signal) => {
        resolve(new Error(`TUI exited before automation socket was ready: code=${code ?? "none"} signal=${signal ?? "none"}`))
      })
    })
    try {
      const startup = await Promise.race([
        waitForSocket(automationSocket).then(() => null),
        startupFailure,
      ])
      if (startup) throw startup
    } catch (error) {
      throw new Error(`${error.message}\n--- tui stdout ---\n${tuiStdout.slice(-4000)}\n--- tui stderr ---\n${tuiStderr.slice(-4000)}`)
    }
    automation = createAutomationClient(automationSocket)
    await automation.send("ping")

    await waitForAutomationSnapshot(
      automation,
      (snapshot) => (snapshot.waitingRoom?.rows ?? []).some((row) => row.id === `remote-kernel:${peerDaemonId}`),
      "peer kernel row in TUI waiting room",
      30_000,
    )
    await automation.send("set_waiting_room_launch", {
      machineRef: peerMachineId,
      kernelRef: peerDaemonId,
      focus: "new",
    })
    const activated = await automation.send("activate_waiting_room")
    sessionSnapshot = await waitForAutomationSnapshot(
      automation,
      (snapshot) => typeof snapshot.session?.id === "string",
      "remote-owned session attach",
      options.timeoutMs,
    )
    const sessionId = sessionSnapshot.session.id
    if (activated.session?.id && activated.session.id !== sessionId) {
      throw new Error(`activation returned session ${activated.session.id} but snapshot attached ${sessionId}`)
    }

    const localSessions = await waitForSessionCount(localClient, 0, "local kernel")
    const peerSessions = await waitForSessionCount(peerClient, 1, "peer kernel")
    if (peerSessions[0]?.id !== sessionId) {
      throw new Error(`TUI attached to ${sessionId}, but peer owns ${peerSessions[0]?.id}`)
    }

    const workerKernels = await waitForRemoteKernel(peerClient, localMachineId)
    const localWorker = workerKernels.find((kernel) => kernel.kernel_id === localDaemonId)
    if (!localWorker) {
      throw new Error(`peer-owned session kernel did not see original local kernel as worker peer: ${JSON.stringify(workerKernels, null, 2)}`)
    }
    if (localWorker.accepting_remote_leases !== true) {
      throw new Error(`original local kernel is visible but not accepting remote leases: ${JSON.stringify(localWorker, null, 2)}`)
    }

    const peerTuiArgs = [
      "-q",
      "/dev/null",
      "env",
      ...Object.entries(peerEnv).map(([key, value]) => `${key}=${value}`),
      "bun",
      path.join(repoRoot, "apps/cli/dist/index.js"),
      "--relay-url", `ws://127.0.0.1:${ports.relayPort}`,
      "--relay-token", relayToken,
      "--target-daemon-id", peerDaemonId,
      "--automation-socket", peerAutomationSocket,
      "--session", sessionId,
      "--workspace", options.workspace,
      "--worktree", options.worktree,
      "--provider", "dev-stub",
      "--model", "tui-remote-owner-drill-model",
      "--client-id", `tui-remote-owner-peer-${process.pid}`,
    ]
    peerTuiChild = spawn("script", peerTuiArgs, { cwd: repoRoot, env: peerEnv, stdio: ["ignore", "pipe", "pipe"] })
    peerTuiChild.stdout.on("data", (chunk) => {
      peerTuiStdout += chunk.toString()
      if (peerTuiStdout.length > 16_000) peerTuiStdout = peerTuiStdout.slice(-16_000)
    })
    peerTuiChild.stderr.on("data", (chunk) => {
      peerTuiStderr += chunk.toString()
      if (peerTuiStderr.length > 16_000) peerTuiStderr = peerTuiStderr.slice(-16_000)
    })
    const peerStartupFailure = new Promise((resolve) => {
      peerTuiChild.once("error", (error) => resolve(error))
      peerTuiChild.once("exit", (code, signal) => {
        resolve(new Error(`peer TUI exited before automation socket was ready: code=${code ?? "none"} signal=${signal ?? "none"}`))
      })
    })
    try {
      const startup = await Promise.race([
        waitForSocket(peerAutomationSocket).then(() => null),
        peerStartupFailure,
      ])
      if (startup) throw startup
    } catch (error) {
      throw new Error(`${error.message}\n--- peer tui stdout ---\n${peerTuiStdout.slice(-4000)}\n--- peer tui stderr ---\n${peerTuiStderr.slice(-4000)}`)
    }
    peerAutomation = createAutomationClient(peerAutomationSocket)
    await peerAutomation.send("ping")
    await waitForAutomationSnapshot(
      peerAutomation,
      (snapshot) => snapshot.session?.id === sessionId,
      "relay peer TUI attached to peer-owned session",
      30_000,
    )

    const peerAttachment = await attachForDrill(peerClient, sessionId, `tui-remote-owner-worker-${process.pid}`)
    const peerEvents = []
    peerClient.onKernelEvent((event) => {
      peerEvents.push({ ...event, observed_at_ms: Date.now() })
    })
    await peerClient.subscribeToKernelEvents(sessionId, peerAttachment.id)
    const spawnedWorker = unwrapVariant(
      await peerClient.send({
        SpawnAgent: {
          session_id: sessionId,
          provider: "dev-stub",
          alias: "original-local-worker",
          model: "tui-remote-owner-worker-model",
          effort: "low",
          worktree_id: options.worktree,
          kernel_ref: localDaemonId,
        },
      }),
      "AgentSpawned",
    )
    workerAgent = spawnedWorker.agent
    workerAgentFinal = await waitForAgentPlacement(
      peerClient,
      sessionId,
      workerAgent.id,
      (agent) => agent.remote_execution?.worker_kernel_id === localDaemonId
        && agent.remote_execution?.worker_machine_id === localMachineId
        && Boolean(agent.remote_execution?.leased_agent_id),
      "original local kernel worker agent",
      30_000,
    )
    const marker = `TUI_REMOTE_OWNER_WORKER_${process.pid}_${Date.now()}`
    workerCompletion = await promptRemoteWorker({
      client: peerClient,
      sessionId,
      attachmentId: peerAttachment.id,
      agentId: workerAgent.id,
      marker,
      events: peerEvents,
      timeoutMs: options.timeoutMs,
    })

    await peerAutomation.send("exit").catch(() => {})
    await automation.send("exit").catch(() => {})
    passed = true
    console.log(JSON.stringify({
      status: "passed",
      sessionId,
      localSessionCount: localSessions.length,
      peerSessionCount: peerSessions.length,
      peerKernelId: peerDaemonId,
      localWorkerKernelId: localDaemonId,
      workerAgentId: workerAgent.id,
      workerLeasedAgentId: workerAgentFinal.remote_execution?.leased_agent_id,
      workerCompletionEvent: workerCompletion?.event ?? null,
    }, null, 2))
  } catch (error) {
    failure = error
    throw error
  } finally {
    peerAutomation?.close()
    automation?.close()
    await localClient?.close().catch(() => {})
    await peerClient?.close().catch(() => {})
    await terminateChild(peerTuiChild)
    await terminateChild(tuiChild)
    await terminateChild(localKernelChild)
    await terminateChild(peerKernelChild)
    await terminateChild(relayChild)
    await rm(peerAutomationSocket, { force: true }).catch(() => {})
    await rm(automationSocket, { force: true }).catch(() => {})
    await finalizeDrillArtifacts({
      rootDir,
      passed,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: "live-tui-remote-owner",
        selectedKernel,
        sessionSnapshot,
        workerAgent,
        workerAgentFinal,
        workerCompletion,
        tuiStdoutTail: tuiStdout.slice(-4000),
        tuiStderrTail: tuiStderr.slice(-4000),
        peerTuiStdoutTail: peerTuiStdout.slice(-4000),
        peerTuiStderrTail: peerTuiStderr.slice(-4000),
      },
    })
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
