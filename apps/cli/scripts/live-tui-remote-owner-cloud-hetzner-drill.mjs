#!/usr/bin/env node
import { spawn } from "node:child_process"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const DEFAULT_TIMEOUT_MS = 180_000

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, "-")
}

function shellQuote(value) {
  return `'${String(value).replace(/'/g, "'\\''")}'`
}

function parseArgs(argv) {
  const options = {
    workspace: repoRoot,
    worktree: repoRoot,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    hetznerHost: process.env.ARROBA_TUI_REMOTE_OWNER_HETZNER_HOST
      ?? process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_HOST
      ?? "root@195.201.123.115",
    hetznerKey: process.env.ARROBA_TUI_REMOTE_OWNER_HETZNER_KEY
      ?? process.env.ARROBA_CLOUD_HOSTED_REMOTE_CLI_KEY
      ?? path.join(os.homedir(), ".ssh", "arroba_hetzner_staging"),
    hetznerRepo: process.env.ARROBA_TUI_REMOTE_OWNER_HETZNER_REPO
      ?? "/tmp/arroba-tui-owner-remote-build/repo",
    profilePath: process.env.ARROBA_TUI_REMOTE_OWNER_PROFILE
      ?? path.join(os.homedir(), ".arroba", "daemon", "config.json"),
  }
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i]
    if (arg === "--") continue
    else if (arg === "--workspace") options.workspace = argv[++i]
    else if (arg === "--worktree") options.worktree = argv[++i]
    else if (arg === "--timeout-ms") options.timeoutMs = Number(argv[++i])
    else if (arg === "--hetzner-host") options.hetznerHost = argv[++i]
    else if (arg === "--hetzner-key") options.hetznerKey = argv[++i]
    else if (arg === "--hetzner-repo") options.hetznerRepo = argv[++i]
    else if (arg === "--profile") options.profilePath = argv[++i]
    else if (arg === "--help") options.help = true
    else throw new Error(`unknown argument: ${arg}`)
  }
  return options
}

function makePorts() {
  const base = 51000 + Math.floor(Math.random() * 1000)
  return {
    localKernelPort: base,
    localMcpPort: base + 1000,
    localOpenCodePort: base + 2000,
    localCodexPort: base + 2001,
    remoteKernelPort: base + 3000,
    remoteMcpPort: base + 3001,
  }
}

function unwrapVariant(response, ...keys) {
  for (const key of keys) {
    if (response?.[key] != null) return response[key]
  }
  return response
}

function spawnProcess(command, args, options) {
  return spawn(command, args, { ...options, stdio: ["ignore", "pipe", "pipe"] })
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

async function run(command, args, options = {}) {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
    child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
    child.on("error", reject)
    child.on("close", (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function runSsh(options, command) {
  return await run("ssh", [
    "-i", options.hetznerKey,
    "-o", "IdentitiesOnly=yes",
    "-o", "BatchMode=yes",
    "-o", "ConnectTimeout=10",
    "-o", "StrictHostKeyChecking=accept-new",
    options.hetznerHost,
    command,
  ])
}

async function readCloudProfile(profilePath) {
  const config = JSON.parse(await readFile(profilePath, "utf8"))
  const profile = config.cloud_relay ?? config.relay?.cloud
  if (!profile?.cloud_session_token && !profile?.cloudSessionToken) {
    throw new Error(`cloud profile ${profilePath} does not include a cloud session token`)
  }
  return normalizeProfile(profile)
}

function normalizeProfile(profile) {
  return {
    api_url: profile.api_url ?? profile.apiUrl,
    email: profile.email,
    account_id: profile.account_id ?? profile.accountId,
    user_id: profile.user_id ?? profile.userId,
    account_slug: profile.account_slug ?? profile.accountSlug,
    realm_id: profile.realm_id ?? profile.realmId,
    relay_url: profile.relay_url ?? profile.relayUrl,
    issuer_id: profile.issuer_id ?? profile.issuerId,
    client_id: profile.client_id ?? profile.clientId ?? null,
    machine_id: profile.machine_id ?? profile.machineId ?? null,
    machine_credential: profile.machine_credential ?? profile.machineCredential ?? null,
    cloud_session_token: profile.cloud_session_token ?? profile.cloudSessionToken ?? null,
    token_expires_at_ms: profile.token_expires_at_ms ?? profile.tokenExpiresAtMs ?? null,
  }
}

async function postJson(url, body) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  })
  if (!response.ok) {
    throw new Error(`POST ${url} failed with ${response.status}: ${await response.text()}`)
  }
  return response.json().catch(() => null)
}

async function pairMachine(profile, machineId, alias) {
  const pairing = await postJson(`${profile.api_url}/pairing-tokens`, {
    accountId: profile.account_id,
    createdByUserId: profile.user_id,
    subjectKind: "machine",
  })
  if (!pairing?.token) throw new Error(`cloud did not issue a machine pairing token: ${JSON.stringify(pairing)}`)
  const paired = await postJson(`${profile.api_url}/machines/pair`, {
    accountId: profile.account_id,
    token: pairing.token,
    machineId,
    userId: profile.user_id,
    alias,
  })
  if (paired?.machineId !== machineId) {
    throw new Error(`cloud machine pair returned unexpected payload: ${JSON.stringify(paired)}`)
  }
}

async function issueMachineToken(profile, machineId) {
  let issued = null
  try {
    issued = await postJson(`${profile.api_url}/relay/token`, {
      ...(profile.machine_credential
        ? { machineCredential: profile.machine_credential }
        : { sessionToken: profile.cloud_session_token }),
      accountId: profile.account_id,
      subject: machineId,
      subjectKind: "machine",
      realmId: profile.realm_id,
      userId: profile.user_id,
      machineId,
    })
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    if (message.includes("session_invalid")) {
      throw new Error(`${message}\nCloud credentials in the selected profile are invalid or expired. Refresh the Aruba Cloud login, then rerun this drill.`)
    }
    throw error
  }
  if (!issued?.token) throw new Error(`cloud did not issue a machine relay token: ${JSON.stringify(issued)}`)
  return issued.token
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

async function waitForRemoteKernel(client, machineRef, kernelRef = null, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    try {
      const listed = unwrapVariant(
        await client.send({ ListRemoteMachineKernels: { machine_ref: machineRef } }),
        "RemoteMachineKernelsListed",
      )
      last = listed.kernels ?? []
      const match = kernelRef
        ? last.find((kernel) => kernel.kernel_id === kernelRef)
        : last.find((kernel) => kernel.accepting_remote_leases)
      if (match) return match
    } catch (error) {
      last = error instanceof Error ? error.message : String(error)
    }
    await sleep(500)
  }
  throw new Error(`remote kernel ${machineRef}/${kernelRef ?? "*"} did not become visible: ${JSON.stringify(last)}`)
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

async function waitForAutomationSnapshot(automation, predicate, label, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await automation.send("snapshot")
    if (predicate(last)) return last
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${label}\nlast snapshot:\n${JSON.stringify(last, null, 2)}`)
}

async function waitForSessionCount(client, expectedCount, label, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const listed = unwrapVariant(await client.send({ ListSessions: {} }), "SessionsListed")
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

async function waitForAgentPlacement(client, sessionId, agentId, predicate, label, timeoutMs = 30_000) {
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

async function promptRemoteWorker({ client, sessionId, attachmentId, agentId, marker, events, timeoutMs }) {
  const baseline = events.filter((event) => event.event === "assistant_message_completed").length
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
  if (!startedPrompt?.id) throw new Error(`worker prompt did not start: ${JSON.stringify(submitted, null, 2)}`)
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    await client.send({ PumpTerminalOutput: { session_id: sessionId, attachment_id: attachmentId } }).catch(() => {})
    const completions = events.filter((event) => event.event === "assistant_message_completed")
    if (completions.length > baseline) return completions.at(-1)
    await client.send({ CompletePrompt: { session_id: sessionId } }).catch(() => {})
    await sleep(500)
  }
  throw new Error(`timed out waiting for worker prompt ${startedPrompt.id}`)
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    console.log("Usage: node apps/cli/scripts/live-tui-remote-owner-cloud-hetzner-drill.mjs [--hetzner-repo PATH] [--profile PATH]")
    return
  }
  const ports = makePorts()
  const runId = `tui-cloud-hetzner-${process.pid}-${Date.now()}`
  const rootDir = path.join(repoRoot, ".artifacts", "live-tui-remote-owner-cloud-hetzner-drill", nowStamp())
  const localHome = path.join(rootDir, "local-home")
  const localConfig = path.join(localHome, ".config")
  const localState = path.join(localHome, ".local", "state")
  const remoteRoot = `/tmp/${runId}`
  const automationSocket = path.join("/tmp", `arroba-tui-cloud-owner-${process.pid}.sock`)
  await prepareDrillArtifacts(rootDir)
  await mkdir(path.join(localConfig, "arroba", "daemon"), { recursive: true })
  await mkdir(localState, { recursive: true })

  const profile = await readCloudProfile(options.profilePath)
  const localMachineId = profile.machine_id || `tui-cloud-local-machine-${process.pid}`
  const localDaemonId = `tui-cloud-local-${process.pid}-${Date.now()}`
  const remoteMachineId = `tui-cloud-hetzner-machine-${process.pid}-${Date.now()}`
  const remoteDaemonId = `tui-cloud-hetzner-${process.pid}-${Date.now()}`
  const remoteAlias = `hetzner-tui-owner-${process.pid}`

  const kernelBinary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
  const { LocalIpcClient } = await import("../../../packages/kernel-client/dist/ipc.js")
  let localKernel = null
  let remoteKernel = null
  let tui = null
  let automation = null
  let localClient = null
  let remoteClient = null
  let passed = false
  let failure = null
  let tuiStdout = ""
  let tuiStderr = ""
  let sessionSnapshot = null
  let selectedKernel = null
  let workerAgent = null
  let workerAgentFinal = null
  let workerCompletion = null

  try {
    await writeFile(path.join(localConfig, "arroba", "daemon", "config.json"), `${JSON.stringify({
      cloud_relay: profile,
    }, null, 2)}\n`, "utf8")

    const localEnv = {
      ...process.env,
      HOME: localHome,
      XDG_CONFIG_HOME: localConfig,
      XDG_STATE_HOME: localState,
      ARROBA_KERNEL_PORT: String(ports.localKernelPort),
      ARROBA_MCP_PORT: String(ports.localMcpPort),
      ARROBA_OPENCODE_PORT: String(ports.localOpenCodePort),
      ARROBA_CODEX_PORT: String(ports.localCodexPort),
      ARROBA_DAEMON_ID: localDaemonId,
      ARROBA_DAEMON_ALIAS: "local-tui-cloud",
      ARROBA_MACHINE_ID: localMachineId,
      ARROBA_MACHINE_ALIAS: "local-tui-cloud-machine",
      ARROBA_ACCEPT_REMOTE_LEASES: "1",
      ARROBA_DAEMON_SOCKET: path.join(rootDir, "local.sock"),
      ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, "local-history"),
    }
    localKernel = spawnProcess(kernelBinary, [], { cwd: repoRoot, env: localEnv })
    localClient = new LocalIpcClient(`ws://127.0.0.1:${ports.localKernelPort}`)
    await waitForKernel(localClient, options.workspace, options.worktree)
    await localClient.send({ ConnectCloudRelay: null })

    await pairMachine(profile, remoteMachineId, remoteAlias)
    const remoteToken = await issueMachineToken(profile, remoteMachineId)
    const remotePreflight = await runSsh(options, [
      "set -e",
      `test -x ${shellQuote(path.posix.join(options.hetznerRepo, "apps/kernel/target/debug/arroba-kernel"))}`,
      `mkdir -p ${shellQuote(remoteRoot)}`,
    ].join("; "))
    if (remotePreflight.code !== 0) {
      throw new Error(`Hetzner preflight failed\n${remotePreflight.stdout}\n${remotePreflight.stderr}`)
    }
    const remoteCommand = [
      "set -e",
      "export PATH=/root/.cargo/bin:/root/.bun/bin:/opt/node-v22/bin:$PATH",
      `mkdir -p ${shellQuote(remoteRoot)}`,
      `cd ${shellQuote(options.hetznerRepo)}`,
      `echo $$ > ${shellQuote(path.posix.join(remoteRoot, "kernel.pid"))}`,
      "exec env",
      `HOME=${shellQuote(path.posix.join(remoteRoot, "home"))}`,
      `XDG_CONFIG_HOME=${shellQuote(path.posix.join(remoteRoot, "config"))}`,
      `XDG_STATE_HOME=${shellQuote(path.posix.join(remoteRoot, "state"))}`,
      `ARROBA_KERNEL_PORT=${shellQuote(String(ports.remoteKernelPort))}`,
      `ARROBA_MCP_PORT=${shellQuote(String(ports.remoteMcpPort))}`,
      `ARROBA_OPENCODE_PORT=${shellQuote(String(ports.remoteKernelPort + 1000))}`,
      `ARROBA_CODEX_PORT=${shellQuote(String(ports.remoteKernelPort + 1001))}`,
      `ARROBA_RELAY_URL=${shellQuote(profile.relay_url)}`,
      `ARROBA_RELAY_TOKEN=${shellQuote(remoteToken)}`,
      `ARROBA_DAEMON_ID=${shellQuote(remoteDaemonId)}`,
      `ARROBA_DAEMON_ALIAS=${shellQuote(remoteAlias)}`,
      `ARROBA_MACHINE_ID=${shellQuote(remoteMachineId)}`,
      `ARROBA_MACHINE_ALIAS=${shellQuote("hetzner-tui-owner-machine")}`,
      "ARROBA_ACCEPT_REMOTE_LEASES=1",
      `ARROBA_DAEMON_SOCKET=${shellQuote(path.posix.join(remoteRoot, "kernel.sock"))}`,
      `ARROBA_SESSION_HISTORY_DIR=${shellQuote(path.posix.join(remoteRoot, "history"))}`,
      "./apps/kernel/target/debug/arroba-kernel",
    ].join(" ")
    remoteKernel = spawnProcess("ssh", [
      "-i", options.hetznerKey,
      "-o", "IdentitiesOnly=yes",
      "-o", "BatchMode=yes",
      "-o", "ConnectTimeout=10",
      "-o", "StrictHostKeyChecking=accept-new",
      options.hetznerHost,
      remoteCommand,
    ], { cwd: repoRoot, env: process.env })

    selectedKernel = await waitForRemoteKernel(localClient, remoteMachineId, remoteDaemonId, 90_000)

    const tuiArgs = [
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
      "--model", "tui-remote-owner-cloud-hetzner-model",
      "--client-id", `tui-cloud-owner-${process.pid}`,
    ]
    tui = spawn("script", tuiArgs, { cwd: repoRoot, env: localEnv, stdio: ["ignore", "pipe", "pipe"] })
    tui.stdout.on("data", (chunk) => {
      tuiStdout += chunk.toString()
      if (tuiStdout.length > 16_000) tuiStdout = tuiStdout.slice(-16_000)
    })
    tui.stderr.on("data", (chunk) => {
      tuiStderr += chunk.toString()
      if (tuiStderr.length > 16_000) tuiStderr = tuiStderr.slice(-16_000)
    })
    const startupFailure = new Promise((resolve) => {
      tui.once("error", (error) => resolve(error))
      tui.once("exit", (code, signal) => {
        resolve(new Error(`TUI exited before automation socket was ready: code=${code ?? "none"} signal=${signal ?? "none"}`))
      })
    })
    const startup = await Promise.race([waitForSocket(automationSocket).then(() => null), startupFailure])
    if (startup) throw startup
    automation = createAutomationClient(automationSocket)
    await automation.send("ping")
    await waitForAutomationSnapshot(
      automation,
      (snapshot) => (snapshot.waitingRoom?.rows ?? []).some((row) => row.id === `remote-kernel:${remoteDaemonId}`),
      "Hetzner kernel row in local TUI waiting room",
      45_000,
    )
    await automation.send("set_waiting_room_launch", {
      machineRef: remoteMachineId,
      kernelRef: remoteDaemonId,
      focus: "new",
    })
    await automation.send("activate_waiting_room")
    sessionSnapshot = await waitForAutomationSnapshot(
      automation,
      (snapshot) => typeof snapshot.session?.id === "string",
      "Hetzner-owned session attach",
      options.timeoutMs,
    )
    const sessionId = sessionSnapshot.session.id
    const resolved = unwrapVariant(
      await localClient.send({
        ResolveKernelClientConnection: {
          machine_ref: remoteMachineId,
          kernel_ref: remoteDaemonId,
          client_id: `tui-cloud-verifier-${process.pid}`,
          session_id: sessionId,
        },
      }),
      "KernelClientConnectionResolved",
    ).connection
    remoteClient = new LocalIpcClient(resolved.relay_url, {
      relayAuthToken: resolved.relay_token,
      targetDaemonId: resolved.target_daemon_id,
      targetDaemonAlias: resolved.target_daemon_alias,
    })
    const localSessions = await waitForSessionCount(localClient, 0, "local kernel")
    const remoteSessions = await waitForSessionCount(remoteClient, 1, "Hetzner kernel")
    if (remoteSessions[0]?.id !== sessionId) {
      throw new Error(`TUI attached to ${sessionId}, but Hetzner owns ${remoteSessions[0]?.id}`)
    }

    await waitForRemoteKernel(remoteClient, localMachineId, localDaemonId, 90_000)
    const remoteAttachment = await attachForDrill(remoteClient, sessionId, `tui-cloud-worker-${process.pid}`)
    const remoteEvents = []
    remoteClient.onKernelEvent((event) => {
      remoteEvents.push({ ...event, observed_at_ms: Date.now() })
    })
    await remoteClient.subscribeToKernelEvents(sessionId, remoteAttachment.id)
    workerAgent = unwrapVariant(
      await remoteClient.send({
        SpawnAgent: {
          session_id: sessionId,
          provider: "dev-stub",
          alias: "local-worker-from-hetzner-home",
          model: "tui-cloud-worker-model",
          effort: "low",
          worktree_id: options.worktree,
          kernel_ref: localDaemonId,
        },
      }),
      "AgentSpawned",
    ).agent
    workerAgentFinal = await waitForAgentPlacement(
      remoteClient,
      sessionId,
      workerAgent.id,
      (agent) => agent.remote_execution?.worker_kernel_id === localDaemonId
        && agent.remote_execution?.worker_machine_id === localMachineId
        && Boolean(agent.remote_execution?.leased_agent_id),
      "local worker from Hetzner-owned session",
      60_000,
    )
    workerCompletion = await promptRemoteWorker({
      client: remoteClient,
      sessionId,
      attachmentId: remoteAttachment.id,
      agentId: workerAgent.id,
      marker: `TUI_CLOUD_HETZNER_WORKER_${process.pid}_${Date.now()}`,
      events: remoteEvents,
      timeoutMs: options.timeoutMs,
    })

    await automation.send("exit").catch(() => {})
    passed = true
    console.log(JSON.stringify({
      status: "passed",
      relayUrl: profile.relay_url,
      localMachineId,
      localKernelId: localDaemonId,
      remoteMachineId,
      remoteKernelId: remoteDaemonId,
      sessionId,
      localSessionCount: localSessions.length,
      remoteSessionCount: remoteSessions.length,
      workerAgentId: workerAgent.id,
      workerLeasedAgentId: workerAgentFinal.remote_execution?.leased_agent_id,
      workerCompletionEvent: workerCompletion?.event ?? null,
    }, null, 2))
  } catch (error) {
    failure = error
    throw error
  } finally {
    automation?.close()
    await remoteClient?.close().catch(() => {})
    await localClient?.close().catch(() => {})
    await terminateChild(tui)
    await terminateChild(localKernel)
    await terminateChild(remoteKernel)
    await rm(automationSocket, { force: true }).catch(() => {})
    await runSsh(options, [
      `if test -f ${shellQuote(path.posix.join(remoteRoot, "kernel.pid"))}; then`,
      `  pid=$(cat ${shellQuote(path.posix.join(remoteRoot, "kernel.pid"))} 2>/dev/null || true);`,
      "  test -n \"$pid\" && kill \"$pid\" 2>/dev/null || true;",
      "fi;",
      `rm -rf ${shellQuote(remoteRoot)}`,
    ].join(" ")).catch(() => {})
    await finalizeDrillArtifacts({
      rootDir,
      passed,
      preserveOnFailure: true,
      failure,
      metadata: {
        drill: "live-tui-remote-owner-cloud-hetzner",
        relayUrl: profile.relay_url,
        localMachineId,
        localKernelId: localDaemonId,
        remoteMachineId,
        remoteKernelId: remoteDaemonId,
        selectedKernel,
        sessionSnapshot,
        workerAgent,
        workerAgentFinal,
        workerCompletion,
        tuiStdoutTail: tuiStdout.slice(-4000),
        tuiStderrTail: tuiStderr.slice(-4000),
      },
    })
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
