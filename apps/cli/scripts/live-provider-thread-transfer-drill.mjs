#!/usr/bin/env node
import { spawn } from "node:child_process"
import { createHmac } from "node:crypto"
import { createWriteStream } from "node:fs"
import { chmod, copyFile, mkdir, readdir, readFile, rm, stat, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSliceRequest,
  createSessionRequest,
  deleteSliceRequest,
  endSessionRequest,
  getProviderRunRequest,
  getSliceRequest,
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
  getSessionStateRequest,
  grantAgentExtensionRequest,
  importSliceProviderAuthRequest,
  installMcpServerRequest,
  launchProviderRunRequest,
  listSessionsRequest,
  listProviderProcessesRequest,
  moveAgentToLocalRequest,
  moveAgentToRemoteRequest,
  saveSliceStateRequest,
  spawnAgentRequest,
  startSliceRequest,
  submitPromptRequest,
  teardownProviderProcessesRequest,
} from "../dist/ipc-requests.js"
import {
  assertBinary,
  makeAvailablePorts,
  portIsAvailable,
  terminateChild,
} from "./lib/drill-runtime-helpers.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const kernelBinary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
const relayBinary = path.join(repoRoot, "apps/relay/target/debug/arroba-relay")
const artifactsRoot = path.join(repoRoot, ".artifacts", "provider-thread-transfer")
const defaultLocalDockerSliceImage = process.env.ARROBA_SLICE_DOCKER_IMAGE ?? "arroba-slice-linux:0.1.0"

const DEFAULT_PROVIDERS = ["opencode", "codex"]
const DEFAULT_MODEL = "gpt-5.2"
const DEFAULT_CODEX_MODEL = process.env.ARROBA_PROVIDER_THREAD_CODEX_MODEL ?? "gpt-5.5"
const DEFAULT_TIMEOUT_MS = 420_000
const DEFAULT_POLL_MS = 1_000
const DEFAULT_SLICE_BUILD_IMAGE_POLICY = process.env.ARROBA_PROVIDER_THREAD_SLICE_BUILD_IMAGE ?? "always"
const RELAY_ISSUER = "arroba-provider-thread-transfer-drill"
const RELAY_SECRET = "arroba-provider-thread-transfer-drill-secret"
const RELAY_REALM = "provider-thread-transfer-drill"

function parseArgs(argv) {
  const options = {
    providers: DEFAULT_PROVIDERS,
    model: DEFAULT_MODEL,
    providerModels: {},
    timeoutMs: DEFAULT_TIMEOUT_MS,
    pollMs: DEFAULT_POLL_MS,
    spawnDaemon: true,
    kernel: null,
    drill: "local-reload",
    keepArtifactsOnFailure: true,
    skipRecallPrompt: false,
    workerState: "shared",
    sliceBuildImage: DEFAULT_SLICE_BUILD_IMAGE_POLICY,
    keepSliceOnFailure: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    else if (arg === "--provider") options.providers = [argv[++index]]
    else if (arg === "--providers") options.providers = argv[++index].split(",").map((value) => value.trim()).filter(Boolean)
    else if (arg === "--model") options.model = argv[++index]
    else if (arg === "--provider-model") {
      const [provider, model] = argv[++index].split("=", 2)
      if (!provider || !model) throw new Error("--provider-model must use provider=model")
      options.providerModels[provider] = model
    } else if (arg === "--timeout-ms") options.timeoutMs = Number(argv[++index])
    else if (arg === "--poll-ms") options.pollMs = Number(argv[++index])
    else if (arg === "--kernel") {
      options.kernel = argv[++index]
      options.spawnDaemon = false
    } else if (arg === "--no-spawn-daemon") {
      options.spawnDaemon = false
    } else if (arg === "--drill") {
      options.drill = argv[++index]
    } else if (arg === "--skip-recall-prompt") {
      options.skipRecallPrompt = true
    } else if (arg === "--worker-state") {
      options.workerState = argv[++index]
    } else if (arg === "--slice-build-image") {
      options.sliceBuildImage = argv[++index]
    } else if (arg === "--keep-slice-on-failure") {
      options.keepSliceOnFailure = true
    } else if (arg === "--cleanup-on-success") {
      options.keepArtifactsOnFailure = true
      options.cleanupOnSuccess = true
    } else if (arg === "--help" || arg === "-h") {
      options.help = true
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  if (options.providers.length === 0) throw new Error("at least one provider is required")
  if (!["local-reload", "worker-resume", "slice-restart", "live-migrate-to-slice", "live-migrate-roundtrip-slice"].includes(options.drill)) {
    throw new Error(`unsupported --drill ${options.drill}; implemented drills: local-reload, worker-resume, slice-restart, live-migrate-to-slice, live-migrate-roundtrip-slice`)
  }
  if (!["shared", "isolated"].includes(options.workerState)) {
    throw new Error(`unsupported --worker-state ${options.workerState}; expected shared or isolated`)
  }
  if (!["always", "auto", "never"].includes(options.sliceBuildImage)) {
    throw new Error(`unsupported --slice-build-image ${options.sliceBuildImage}; expected always, auto, or never`)
  }
  return options
}

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/live-provider-thread-transfer-drill.mjs [options]",
    "",
    "Runs executable drills from docs/ARROBA_SERVER_PROVIDER_THREAD_TRANSFER_DRILLS_PLAN.md.",
    "",
    "Implemented drill:",
    "  local-reload  Drill 1: baseline local reload preserves provider thread",
    "  worker-resume  Drill 3 precursor: resume a captured provider thread on a same-host worker",
    "  slice-restart  Drill 4 precursor: save/restart a local Docker slice and relaunch the same agent",
    "  live-migrate-to-slice  Drill 4: start locally, move the same agent to a slice, and resume the same provider thread",
    "  live-migrate-roundtrip-slice  Drill 5: move local -> slice -> local and resume the same provider thread both ways",
    "",
    "Options:",
    `  --providers ${DEFAULT_PROVIDERS.join(",")}`,
    "  --provider PROVIDER",
    "  --provider-model PROVIDER=MODEL",
    "  --model MODEL",
    `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
    `  --poll-ms ${DEFAULT_POLL_MS}`,
    "  --kernel ws://127.0.0.1:PORT",
    "  --no-spawn-daemon",
    "  --skip-recall-prompt",
    "  --worker-state shared|isolated",
    `  --slice-build-image always|auto|never (default ${DEFAULT_SLICE_BUILD_IMAGE_POLICY})`,
    "  --keep-slice-on-failure",
    "  --cleanup-on-success",
  ].join("\n"))
}

function variant(response, name) {
  if (!response || !(name in response)) {
    throw new Error(`expected ${name}, got ${JSON.stringify(response)}`)
  }
  return response[name]
}

function variantAny(response, ...names) {
  for (const name of names) {
    if (response && name in response) return response[name]
  }
  throw new Error(`expected one of ${names.join(", ")}, got ${JSON.stringify(response)}`)
}

function providerModel(provider, options) {
  if (options.providerModels[provider]) return options.providerModels[provider]
  if (provider === "opencode") return options.model
  if (provider === "codex" && options.model === DEFAULT_MODEL) return DEFAULT_CODEX_MODEL
  if (provider === "codex" && !options.model.endsWith("-codex") && /^gpt-5\.[23]$/.test(options.model)) {
    return `${options.model}-codex`
  }
  if ((provider === "claude-p" || provider === "claude-headless") && !options.model.startsWith("claude-")) {
    return "claude-sonnet-4-6"
  }
  return options.model
}

function providerEffort(provider) {
  if (provider === "claude-p" || provider === "claude-headless") return "low"
  return "low"
}

function base64url(input) {
  return Buffer.from(input).toString("base64url")
}

function signRelayToken(claims) {
  const payload = base64url(JSON.stringify(claims))
  const signature = createHmac("sha256", RELAY_SECRET).update(payload).digest("base64url")
  return `arroba-scoped-v1.${payload}.${signature}`
}

function relayClaims({ subject, subjectKind, actions, userId = "local", targets = null }) {
  return {
    issuer: RELAY_ISSUER,
    subject,
    subject_kind: subjectKind,
    realm_id: RELAY_REALM,
    allowed_actions: actions,
    allowed_targets: targets,
    issued_at_ms: Date.now(),
    expires_at_ms: Date.now() + 10 * 60_000,
    token_id: `${subject}-${Date.now()}`,
    account_id: "provider-thread-transfer-drill-account",
    organization_id: null,
    user_id: userId,
    device_id: subject,
    machine_id: subjectKind === "kernel" || subjectKind === "machine" ? subject : null,
    client_id: subjectKind === "client" ? subject : null,
    public_key_thumbprint: `${subject}-thumbprint`,
    entitlements_version: "drill",
  }
}

async function makeWorkerResumePorts() {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const ports = await makeAvailablePorts()
    const expanded = {
      ...ports,
      homeKernelPort: ports.kernelPort,
      homeMcpPort: ports.mcpPort,
      homeOpenCodePort: ports.openCodePort,
      homeCodexPort: ports.codexPort,
      workerOpenCodePort: ports.openCodePort + 101,
      workerCodexPort: ports.codexPort + 101,
    }
    if (
      await portIsAvailable(expanded.workerOpenCodePort)
      && await portIsAvailable(expanded.workerCodexPort)
    ) {
      return expanded
    }
  }
  throw new Error("could not find available worker-resume drill ports")
}

function providerThreadId(run) {
  return run?.provider_session_id
    ?? run?.resume_state?.opencode_session_id
    ?? run?.resume_state?.codex_thread_id
    ?? run?.resume_state?.claude_session_id
    ?? null
}

function providerRunSnapshot(run) {
  return {
    id: run?.id ?? null,
    provider: run?.provider ?? null,
    adapter_key: run?.adapter_key ?? null,
    state: run?.state ?? null,
    provider_session_id: run?.provider_session_id ?? null,
    resume_state: run?.resume_state ?? null,
    mcp_servers: (run?.mcp_servers ?? []).map((server) => server.name ?? server),
    execution_mode: run?.execution_mode ?? null,
    permission_level: run?.permission_level ?? null,
    write_access_mode: run?.write_access_mode ?? null,
    started_at_ms: run?.started_at_ms ?? null,
    last_activity_at_ms: run?.last_activity_at_ms ?? null,
  }
}

function sliceRecordSnapshot(slice) {
  return {
    id: slice?.id ?? null,
    name: slice?.name ?? null,
    status: slice?.status ?? null,
    backend: slice?.backend ?? null,
    display_mode: slice?.display_mode ?? null,
    worker_kernel_ref: slice?.worker_kernel_ref ?? null,
    worker_kernel_id: slice?.worker_kernel_id ?? null,
    worker_machine_id: slice?.worker_machine_id ?? null,
    providers: slice?.providers ?? [],
    session_ids: slice?.session_ids ?? [],
    agent_ids: slice?.agent_ids ?? [],
    saved_state_id: slice?.active_saved_state_id ?? slice?.saved_state_id ?? null,
    operation: slice?.operation ?? null,
  }
}

function sliceSavedStateSnapshot(state) {
  return {
    id: state?.id ?? null,
    slice_id: state?.slice_id ?? null,
    backend: state?.backend ?? null,
    status: state?.status ?? null,
    image_ref: state?.image_ref ?? null,
    created_at_ms: state?.created_at_ms ?? null,
    updated_at_ms: state?.updated_at_ms ?? null,
  }
}

function logStep(result, provider, step, details = {}) {
  const entry = {
    at_ms: Date.now(),
    step,
    ...details,
  }
  result.evidence.steps ??= []
  result.evidence.steps.push(entry)
  console.log(`${provider}: ${step}${Object.keys(details).length ? ` ${JSON.stringify(details)}` : ""}`)
}

async function withTimeout(promise, label, timeoutMs) {
  let timer = null
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => {
        timer = globalThis.setTimeout(() => reject(new Error(`${label} timed out after ${timeoutMs}ms`)), timeoutMs)
      }),
    ])
  } finally {
    if (timer) globalThis.clearTimeout(timer)
  }
}

async function sendControlRequest(kernelUrl, request, label, timeoutMs) {
  const controlClient = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  try {
    return await withTimeout(
      controlClient.send(request),
      label,
      timeoutMs,
    )
  } finally {
    await controlClient.close().catch(() => {})
  }
}

async function waitForLocalDaemon(kernelUrl, workspace, worktree) {
  let lastError = null
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const client = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      const session = variant(await client.send(createSessionRequest(workspace, worktree)), "SessionCreated").session
      await client.send(endSessionRequest(session.id)).catch(() => {})
      await client.close()
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
}

async function waitForRemoteMachine(localClient, machineRef, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await localClient.send({ ListRemoteMachineKernels: { machine_ref: machineRef } })
      const payload = variant(response, "RemoteMachineKernelsListed")
      if ((payload.kernels ?? []).length > 0) return payload.kernels
    } catch (error) {
      lastError = error
    }
    await sleep(pollMs)
  }
  throw new Error(`remote machine ${machineRef} did not become reachable: ${lastError?.message ?? lastError ?? "unknown error"}`)
}

async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias, timeoutMs, pollMs) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await withTimeout(client.send(listSessionsRequest()), `probe relay target ${targetDaemonAlias}`, 2_000)
      await client.close()
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(pollMs)
    }
  }
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable: ${lastError?.message ?? lastError ?? "unknown error"}`)
}

function realProviderEnv() {
  const home = process.env.HOME ?? os.homedir()
  const xdgDataHome = process.env.XDG_DATA_HOME ?? path.join(home, ".local", "share")
  return {
    HOME: home,
    CODEX_HOME: process.env.CODEX_HOME ?? path.join(home, ".codex"),
    OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(home, ".config", "opencode"),
    OPENCODE_DATA_HOME: process.env.OPENCODE_DATA_HOME ?? path.join(xdgDataHome, "opencode"),
    XDG_CONFIG_HOME: process.env.XDG_CONFIG_HOME ?? path.join(home, ".config"),
    XDG_DATA_HOME: xdgDataHome,
    XDG_STATE_HOME: process.env.XDG_STATE_HOME ?? path.join(home, ".local", "state"),
    XDG_CACHE_HOME: process.env.XDG_CACHE_HOME ?? path.join(home, ".cache"),
  }
}

async function copySecretIfPresent(source, destination) {
  try {
    await mkdir(path.dirname(destination), { recursive: true })
    await copyFile(source, destination)
    await chmod(destination, 0o600).catch(() => {})
    return true
  } catch (error) {
    if (error?.code === "ENOENT") return false
    throw error
  }
}

async function prepareSliceModeProviderEnv(root) {
  const real = realProviderEnv()
  const codexHome = path.join(root, "codex-home")
  const xdgConfigHome = path.join(root, "xdg-config")
  const xdgDataHome = path.join(root, "xdg-data")
  const xdgStateHome = path.join(root, "xdg-state")
  const xdgCacheHome = path.join(root, "xdg-cache")
  const opencodeDataHome = path.join(xdgDataHome, "opencode")

  await mkdir(codexHome, { recursive: true })
  await mkdir(opencodeDataHome, { recursive: true })
  await mkdir(xdgStateHome, { recursive: true })
  await mkdir(xdgCacheHome, { recursive: true })

  const codexAuthCopied = await copySecretIfPresent(
    path.join(real.CODEX_HOME, "auth.json"),
    path.join(codexHome, "auth.json"),
  )
  const opencodeAuthCopied = await copySecretIfPresent(
    path.join(real.OPENCODE_DATA_HOME, "auth.json"),
    path.join(opencodeDataHome, "auth.json"),
  )

  return {
    HOME: real.HOME,
    CODEX_HOME: codexHome,
    OPENCODE_CONFIG_DIR: real.OPENCODE_CONFIG_DIR,
    OPENCODE_DATA_HOME: opencodeDataHome,
    XDG_CONFIG_HOME: xdgConfigHome,
    XDG_DATA_HOME: xdgDataHome,
    XDG_STATE_HOME: xdgStateHome,
    XDG_CACHE_HOME: xdgCacheHome,
    ARROBA_PROVIDER_THREAD_CODEX_AUTH_COPIED: codexAuthCopied ? "1" : "0",
    ARROBA_PROVIDER_THREAD_OPENCODE_AUTH_COPIED: opencodeAuthCopied ? "1" : "0",
  }
}

async function prepareIsolatedWorkerProviderEnv() {
  const real = realProviderEnv()
  const secretRoot = path.join(os.tmpdir(), `arroba-provider-transfer-secrets-${process.pid}-${Date.now()}`)
  const isolatedHome = path.join(secretRoot, "home")
  const codexHome = path.join(secretRoot, "codex")
  const xdgDataHome = path.join(secretRoot, "xdg-data")
  const xdgStateHome = path.join(secretRoot, "xdg-state")
  const xdgCacheHome = path.join(secretRoot, "xdg-cache")
  const opencodeDataHome = path.join(secretRoot, "opencode-data")

  await mkdir(isolatedHome, { recursive: true })
  await mkdir(codexHome, { recursive: true })
  await mkdir(path.join(xdgDataHome, "opencode"), { recursive: true })
  await mkdir(xdgStateHome, { recursive: true })
  await mkdir(xdgCacheHome, { recursive: true })
  await mkdir(opencodeDataHome, { recursive: true })

  const codexAuthCopied = await copySecretIfPresent(
    path.join(real.CODEX_HOME, "auth.json"),
    path.join(codexHome, "auth.json"),
  )
  const opencodeSourceDataHome = process.env.OPENCODE_DATA_HOME
    ?? path.join(real.XDG_DATA_HOME, "opencode")
  const opencodeAuthSource = path.join(opencodeSourceDataHome, "auth.json")
  const opencodeDataAuthCopied = await copySecretIfPresent(
    opencodeAuthSource,
    path.join(opencodeDataHome, "auth.json"),
  )
  const opencodeXdgAuthCopied = await copySecretIfPresent(
    opencodeAuthSource,
    path.join(xdgDataHome, "opencode", "auth.json"),
  )

  return {
    secretRoot,
    providerEnv: {
      HOME: isolatedHome,
      CODEX_HOME: codexHome,
      OPENCODE_CONFIG_DIR: real.OPENCODE_CONFIG_DIR,
      OPENCODE_DATA_HOME: opencodeDataHome,
      XDG_CONFIG_HOME: real.XDG_CONFIG_HOME,
      XDG_DATA_HOME: xdgDataHome,
      XDG_STATE_HOME: xdgStateHome,
      XDG_CACHE_HOME: xdgCacheHome,
    },
    evidence: {
      mode: "isolated",
      codex_auth_copied: codexAuthCopied,
      opencode_auth_copied: opencodeDataAuthCopied || opencodeXdgAuthCopied,
      opencode_config_shared: true,
      provider_data_shared: false,
      provider_cache_shared: false,
      provider_home_shared: false,
    },
  }
}

function workerResumeDaemonEnv({
  ports,
  root,
  relayToken,
  daemonId,
  daemonAlias,
  machineId,
  machineAlias,
  acceptRemoteLeases,
  socketName,
  kernelPort,
  mcpPort,
  openCodePort,
  codexPort,
  providerEnv = realProviderEnv(),
}) {
  return {
    ...process.env,
    ...providerEnv,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(openCodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
    ARROBA_RELAY_TOKEN: relayToken,
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_DAEMON_ALIAS: daemonAlias,
    ARROBA_MACHINE_ID: machineId,
    ARROBA_MACHINE_ALIAS: machineAlias,
    ARROBA_ACCEPT_REMOTE_LEASES: acceptRemoteLeases ? "1" : "0",
    ARROBA_DAEMON_SOCKET: path.join(root, socketName),
    ARROBA_SESSION_HISTORY_DIR: path.join(root, `${daemonId}-history`),
    ARROBA_CAPABILITY_ISOLATION_ROOT: path.join(root, `${daemonId}-capabilities`),
    ARROBA_PROVIDER_RUNTIME_INIT_DELAY_MS: "250",
  }
}

function spawnLogged(command, args, { cwd, env, stdoutPath, stderrPath }) {
  const stdout = createWriteStream(stdoutPath, { flags: "a" })
  const stderr = createWriteStream(stderrPath, { flags: "a" })
  const child = spawn(command, args, {
    cwd,
    env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  child.stdout?.pipe(stdout)
  child.stderr?.pipe(stderr)
  return child
}

async function runLoggedCommand(command, args, { cwd, env, stdoutPath, stderrPath, timeoutMs }) {
  const child = spawnLogged(command, args, { cwd, env, stdoutPath, stderrPath })
  try {
    const status = await withTimeout(
      new Promise((resolve, reject) => {
        child.on("error", reject)
        child.on("close", (code, signal) => resolve({ code, signal }))
      }),
      `${command} ${args.join(" ")}`,
      timeoutMs,
    )
    if (status.code !== 0) {
      throw new Error(`${command} ${args.join(" ")} exited with code ${status.code}${status.signal ? ` signal ${status.signal}` : ""}`)
    }
  } catch (error) {
    await terminateChild(child)
    throw error
  }
}

async function prebuildLocalDockerSliceImageIfNeeded(root, policy, timeoutMs) {
  if (policy !== "always") return null
  const stdoutPath = path.join(root, "slice-image-build.stdout.log")
  const stderrPath = path.join(root, "slice-image-build.stderr.log")
  await runLoggedCommand("docker", [
    "build",
    "-f",
    path.join(repoRoot, "apps/kernel/slice-linux-docker/docker/Dockerfile"),
    "-t",
    defaultLocalDockerSliceImage,
    repoRoot,
  ], {
    cwd: repoRoot,
    env: process.env,
    stdoutPath,
    stderrPath,
    timeoutMs,
  })
  return { image: defaultLocalDockerSliceImage, stdoutPath, stderrPath }
}

async function waitForProviderRun({ client, providerRunId, timeoutMs, pollMs, requireThreadId = true }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const response = await client.send(getProviderRunRequest(providerRunId)).catch((error) => {
      last = { error: error.message ?? String(error) }
      return null
    })
    const run = response ? variant(response, "ProviderRun").provider_run : null
    if (run) {
      last = providerRunSnapshot(run)
      const state = String(run.state ?? "").toLowerCase()
      const threadId = providerThreadId(run)
      if ((state === "running" || state === "parked" || state === "starting") && (!requireThreadId || threadId)) {
        return run
      }
      if (state === "ended" || state === "failed" || state === "error") {
        throw new Error(`provider run ${providerRunId} ended before becoming ready: ${JSON.stringify(last)}`)
      }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run ${providerRunId}; last=${JSON.stringify(last)}`)
}

async function waitForProviderRunEnded({ client, providerRunId, timeoutMs, pollMs }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const response = await client.send(getProviderRunRequest(providerRunId)).catch((error) => {
      last = { error: error.message ?? String(error) }
      return null
    })
    const run = response ? variant(response, "ProviderRun").provider_run : null
    if (run) {
      last = providerRunSnapshot(run)
      if (String(run.state ?? "").toLowerCase() === "ended") {
        return run
      }
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run ${providerRunId} to end; last=${JSON.stringify(last)}`)
}

async function waitForActiveProviderRunChange({ client, sessionId, previousRunId, timeoutMs, pollMs }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const state = variantAny(await client.send(getSessionStateRequest(sessionId)), "SessionState", "SessionStateLoaded")
    const session = state.session ?? state
    const activeRunId = session.active_provider_run_id ?? null
    last = { activeRunId }
    if (activeRunId && activeRunId !== previousRunId) {
      const run = await waitForProviderRun({
        client,
        providerRunId: activeRunId,
        timeoutMs: Math.min(timeoutMs, 180_000),
        pollMs,
        requireThreadId: true,
      })
      return run
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for provider run change from ${previousRunId}; last=${JSON.stringify(last)}`)
}

async function waitForSessionActiveProviderRun({ client, sessionId, timeoutMs, pollMs }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const state = variantAny(await client.send(getSessionStateRequest(sessionId)), "SessionState", "SessionStateLoaded")
    const session = state.session ?? state
    const activeRunId = session.active_provider_run_id ?? null
    last = { activeRunId }
    if (activeRunId) {
      const run = await waitForProviderRun({
        client,
        providerRunId: activeRunId,
        timeoutMs: Math.min(timeoutMs, 180_000),
        pollMs,
        requireThreadId: true,
      })
      return run
    }
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for active provider run in session ${sessionId}; last=${JSON.stringify(last)}`)
}

async function waitForPromptIdle({ client, sessionId, attachmentId, agentId, timeoutMs, pollMs }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const state = variantAny(await client.send(getSessionStateRequest(sessionId)), "SessionState", "SessionStateLoaded")
    const session = state.session ?? state
    const agent = (session.agents ?? []).find((entry) => entry.id === agentId)
    const promptState = session.prompt_states?.[agentId]
    const activePrompt = promptState?.active_prompt ?? (session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null)
    const queuedPrompts = promptState?.queued_prompts ?? (session.queued_prompts ?? []).filter((prompt) => prompt.target_agent_id === agentId)
    last = {
      agent_state: agent?.state ?? null,
      is_processing: agent?.is_processing ?? null,
      active_prompt: activePrompt?.id ?? null,
      queued_count: queuedPrompts?.length ?? 0,
    }
    if (!activePrompt && (queuedPrompts?.length ?? 0) === 0) return
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for agent ${agentId} to become idle; last=${JSON.stringify(last)}`)
}

function providerAuthName(provider) {
  if (provider === "claude-p" || provider === "claude-headless") return "claude"
  return provider
}

function safeLabel(value) {
  return value.replace(/[^a-zA-Z0-9_.-]+/g, "-")
}

async function pathStat(pathname) {
  try {
    return await stat(pathname)
  } catch (error) {
    if (error?.code === "ENOENT") return null
    throw error
  }
}

function providerStateCopySpecs(provider, providerEnv = realProviderEnv()) {
  if (provider === "codex") {
    return [
      {
        label: "codex-home",
        source: providerEnv.CODEX_HOME,
        target: "/home/slice/.codex",
        kind: "dir",
      },
    ]
  }
  if (provider === "opencode") {
    const opencodeDataHome = providerEnv.OPENCODE_DATA_HOME
      ?? path.join(providerEnv.XDG_DATA_HOME, "opencode")
    return [
      {
        label: "opencode-data",
        source: opencodeDataHome,
        target: "/home/slice/.local/share/opencode",
        kind: "dir",
      },
      {
        label: "opencode-config",
        source: providerEnv.OPENCODE_CONFIG_DIR,
        target: "/home/slice/.config/opencode",
        kind: "dir",
      },
    ]
  }
  if (provider === "claude-p" || provider === "claude-headless" || provider === "claude") {
    const home = process.env.HOME ?? os.homedir()
    return [
      {
        label: "claude-home",
        source: path.join(home, ".claude"),
        target: "/home/slice/.claude",
        kind: "dir",
      },
      {
        label: "claude-json",
        source: path.join(home, ".claude.json"),
        target: "/home/slice/.claude.json",
        kind: "file",
      },
    ]
  }
  return []
}

async function runDockerCommandForTransfer(root, label, args, timeoutMs) {
  await runLoggedCommand("docker", args, {
    cwd: repoRoot,
    env: process.env,
    stdoutPath: path.join(root, `${safeLabel(label)}.stdout.log`),
    stderrPath: path.join(root, `${safeLabel(label)}.stderr.log`),
    timeoutMs,
  })
}

async function transferProviderStateToSlice({ provider, root, sliceName, timeoutMs, providerEnv }) {
  const container = `arroba-slice-${sliceName}`
  const evidence = {
    provider,
    container,
    copied: [],
    missing: [],
  }
  for (const spec of providerStateCopySpecs(provider, providerEnv)) {
    const sourceStat = await pathStat(spec.source)
    if (!sourceStat) {
      evidence.missing.push({
        label: spec.label,
        kind: spec.kind,
        target: spec.target,
      })
      continue
    }
    if (spec.kind === "dir" && !sourceStat.isDirectory()) {
      evidence.missing.push({
        label: spec.label,
        kind: spec.kind,
        target: spec.target,
        reason: "source is not a directory",
      })
      continue
    }
    if (spec.kind === "file" && !sourceStat.isFile()) {
      evidence.missing.push({
        label: spec.label,
        kind: spec.kind,
        target: spec.target,
        reason: "source is not a file",
      })
      continue
    }

    const targetDir = spec.kind === "file" ? path.posix.dirname(spec.target) : spec.target
    await runDockerCommandForTransfer(
      root,
      `${provider}-${spec.label}-mkdir`,
      ["exec", "-u", "root", container, "bash", "-lc", `mkdir -p ${JSON.stringify(targetDir)}`],
      Math.min(timeoutMs, 60_000),
    )
    if (spec.kind === "dir") {
      await runDockerCommandForTransfer(
        root,
        `${provider}-${spec.label}-copy`,
        ["cp", `${spec.source}/.`, `${container}:${spec.target}/`],
        Math.min(timeoutMs, 180_000),
      )
    } else {
      await runDockerCommandForTransfer(
        root,
        `${provider}-${spec.label}-copy`,
        ["cp", spec.source, `${container}:${spec.target}`],
        Math.min(timeoutMs, 60_000),
      )
    }
    await runDockerCommandForTransfer(
      root,
      `${provider}-${spec.label}-chown`,
      ["exec", "-u", "root", container, "bash", "-lc", `chown -R slice:slice ${JSON.stringify(spec.target)}`],
      Math.min(timeoutMs, 60_000),
    )
    evidence.copied.push({
      label: spec.label,
      kind: spec.kind,
      target: spec.target,
    })
  }
  return evidence
}

async function transferProviderStateFromSlice({ provider, root, sliceName, timeoutMs, providerEnv }) {
  const container = `arroba-slice-${sliceName}`
  const copyEnv = provider === "claude-p" || provider === "claude-headless" || provider === "claude"
    ? { ...providerEnv, HOME: path.join(root, "returned-claude-home") }
    : providerEnv
  const evidence = {
    provider,
    container,
    ...(copyEnv !== providerEnv ? { destination: copyEnv.HOME, destination_mode: "artifact_only" } : {}),
    copied: [],
    missing: [],
  }
  for (const spec of providerStateCopySpecs(provider, copyEnv)) {
    const targetStat = await dockerPathStat(container, spec.target, root, `${provider}-${spec.label}-reverse-stat`, timeoutMs)
    if (!targetStat.exists) {
      evidence.missing.push({
        label: spec.label,
        kind: spec.kind,
        target: spec.target,
      })
      continue
    }

    if (spec.kind === "dir") {
      await mkdir(spec.source, { recursive: true })
      await runDockerCommandForTransfer(
        root,
        `${provider}-${spec.label}-reverse-copy`,
        ["cp", `${container}:${spec.target}/.`, `${spec.source}/`],
        Math.min(timeoutMs, 180_000),
      )
    } else {
      await mkdir(path.dirname(spec.source), { recursive: true })
      await runDockerCommandForTransfer(
        root,
        `${provider}-${spec.label}-reverse-copy`,
        ["cp", `${container}:${spec.target}`, spec.source],
        Math.min(timeoutMs, 60_000),
      )
      await chmod(spec.source, 0o600).catch(() => {})
    }
    evidence.copied.push({
      label: spec.label,
      kind: spec.kind,
      source: spec.target,
    })
  }
  return evidence
}

async function dockerPathStat(container, containerPath, root, label, timeoutMs) {
  const stdoutPath = path.join(root, `${safeLabel(label)}.stdout.log`)
  const stderrPath = path.join(root, `${safeLabel(label)}.stderr.log`)
  try {
    await runLoggedCommand("docker", ["exec", container, "bash", "-lc", `test -e ${JSON.stringify(containerPath)}`], {
      cwd: repoRoot,
      env: process.env,
      stdoutPath,
      stderrPath,
      timeoutMs: Math.min(timeoutMs, 30_000),
    })
    return { exists: true }
  } catch (error) {
    return {
      exists: false,
      error: error.message ?? String(error),
      stdoutPath,
      stderrPath,
    }
  }
}

async function waitForSliceWorkerProvider({ client, sliceRef, provider, timeoutMs, pollMs }) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const payload = variant(await client.send(getSliceRequest(sliceRef)), "Slice")
    const slice = payload.slice
    last = {
      status: slice?.status ?? null,
      worker_kernel_id: slice?.worker_kernel_id ?? null,
      worker_kernel_ref: slice?.worker_kernel_ref ?? null,
      providers: slice?.providers ?? [],
    }
    const providers = slice?.providers ?? []
    if (slice?.worker_kernel_id && providers.includes(provider)) return slice
    await sleep(pollMs)
  }
  throw new Error(`timed out waiting for slice ${sliceRef} worker provider ${provider}; last=${JSON.stringify(last)}`)
}

async function loadAgentHistoryEntries(client, sessionId, agentId, latestPromptCount = 20) {
  const outline = variant(
    await client.send(getSessionHistoryOutlineRequest(sessionId, [agentId], latestPromptCount)),
    "SessionHistoryOutline",
  )
  const entries = []
  const agent = outline.agents?.find((entry) => entry.agent_id === agentId)
  for (const turn of agent?.turns ?? []) {
    for (const row of turn.entries ?? []) {
      if (row?.entry) entries.push(row.entry)
    }
    for (const blob of turn.blobs ?? []) {
      const content = variant(
        await client.send(getSessionHistoryBlobContentRequest(sessionId, agentId, blob.blob_id)),
        "SessionHistoryBlobContent",
      )
      for (const row of content.entries ?? []) {
        if (row?.entry) entries.push(row.entry)
      }
    }
  }
  return entries
}

async function waitForHistoryOutputMarker({ client, sessionId, attachmentId, agentId, marker, timeoutMs, pollMs, historyDir = null }) {
  const deadline = Date.now() + timeoutMs
  let lastText = ""
  let lastCompactText = ""
  let lastFallbackCompactText = ""
  let lastRawCompactText = ""
  let lastOrderedMatch = false
  while (Date.now() < deadline) {
    const entries = await loadAgentHistoryEntries(client, sessionId, agentId)
    const outputEntries = entries.filter((entry) => entry?.kind !== "user_prompt")
    const textFragments = outputEntries
      .filter((entry) => entry.agent_id == null || entry.agent_id === agentId)
      .map(historyEntryText)
      .filter(Boolean)
    const fallbackTextFragments = outputEntries.map(historyEntryText).filter(Boolean)
    lastText = textFragments.join("\n")
    lastCompactText = textFragments.join("")
    lastFallbackCompactText = fallbackTextFragments.join("")
    lastRawCompactText = historyDir
      ? await loadRawHistoryOutputText({ historyDir, sessionId, agentId }).catch(() => "")
      : ""
    lastOrderedMatch = containsOrderedMarker(lastCompactText, marker)
      || containsOrderedMarker(lastFallbackCompactText, marker)
      || containsOrderedMarker(lastRawCompactText, marker)
    if (
      lastText.includes(marker) ||
      lastCompactText.includes(marker) ||
      lastFallbackCompactText.includes(marker) ||
      lastRawCompactText.includes(marker) ||
      lastOrderedMatch
    ) {
      return {
        entries,
        text: lastText,
        compactText: lastCompactText,
        fallbackCompactText: lastFallbackCompactText,
        rawCompactText: lastRawCompactText,
        orderedMatch: lastOrderedMatch,
      }
    }
    await sleep(pollMs)
  }
  throw new Error(
    `timed out waiting for marker ${marker}; ordered_match=${lastOrderedMatch}\n${lastText.slice(-4000)}\ncompact:\n${lastCompactText.slice(-4000)}\nfallback_compact:\n${lastFallbackCompactText.slice(-4000)}\nraw_compact:\n${lastRawCompactText.slice(-4000)}`,
  )
}

function historyEntryText(entry) {
  if (!entry) return ""
  if (typeof entry.text === "string") return entry.text
  if (typeof entry.message === "string") return entry.message
  if (typeof entry.display_text === "string") return entry.display_text
  if (typeof entry.content === "string") return entry.content
  return ""
}

async function loadRawHistoryOutputText({ historyDir, sessionId, agentId }) {
  const names = await readdir(historyDir)
  const fragments = []
  for (const name of names) {
    if (!name.startsWith(`${sessionId}-`) || !name.endsWith(".jsonl")) continue
    const file = await readFile(path.join(historyDir, name), "utf8")
    for (const line of file.split("\n")) {
      if (!line.trim()) continue
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if (entry?.kind === "user_prompt") continue
      if (entry?.agent_id != null && entry.agent_id !== agentId) continue
      const text = historyEntryText(entry)
      if (text) fragments.push(text)
    }
  }
  return fragments.join("")
}

function containsOrderedMarker(text, marker) {
  let index = 0
  for (const char of text) {
    if (char === marker[index]) index += 1
    if (index === marker.length) return true
  }
  return false
}

async function createDeterministicMcp(root, name) {
  const mcpPath = path.join(root, `${name}.mjs`)
  await writeFile(mcpPath, [
    "let buffer = Buffer.alloc(0)",
    "function write(message) { process.stdout.write(`${JSON.stringify(message)}\\n`) }",
    "function handle(message) {",
    "  const { id, method, params } = message",
    "  if (method === 'notifications/initialized') return",
    "  if (method === 'initialize') {",
    "    write({ jsonrpc: '2.0', id, result: { protocolVersion: '2024-11-05', capabilities: { tools: {} }, serverInfo: { name: 'arroba-provider-thread-transfer', version: '1.0.0' } } })",
    "    return",
    "  }",
    "  if (method === 'tools/list') {",
    "    write({ jsonrpc: '2.0', id, result: { tools: [{ name: 'thread_transfer_probe', description: 'Returns a marker for Arroba provider-thread transfer drills.', inputSchema: { type: 'object', properties: { marker: { type: 'string' } }, required: ['marker'] } }] } })",
    "    return",
    "  }",
    "  if (method === 'tools/call' && params?.name === 'thread_transfer_probe') {",
    "    write({ jsonrpc: '2.0', id, result: { content: [{ type: 'text', text: `THREAD_TRANSFER_PROBE:${params?.arguments?.marker ?? ''}` }] } })",
    "    return",
    "  }",
    "  write({ jsonrpc: '2.0', id, error: { code: -32601, message: `unknown method ${method}` } })",
    "}",
    "process.stdin.on('data', (chunk) => {",
    "  buffer = Buffer.concat([buffer, chunk])",
    "  while (true) {",
    "    const newline = buffer.indexOf('\\n')",
    "    if (newline < 0) return",
    "    const line = buffer.subarray(0, newline).toString('utf8').trim()",
    "    buffer = buffer.subarray(newline + 1)",
    "    if (line) handle(JSON.parse(line))",
    "  }",
    "})",
  ].join("\n"), "utf8")
  return mcpPath
}

function mcpConfig(name, scriptPath) {
  return {
    name,
    transport: {
      type: "stdio",
      command: process.execPath,
      args: [scriptPath],
    },
    enabled: true,
    required: true,
    startup_timeout_sec: 45,
    tool_timeout_sec: 45,
  }
}

async function collectProviderProcesses(client, provider) {
  const response = await client.send(listProviderProcessesRequest(provider)).catch((error) => ({
    error: error.message ?? String(error),
  }))
  return response
}

async function runLocalReloadScenario({ provider, root, kernelUrl, options }) {
  const workspace = path.join(root, provider, "workspace")
  const outputsDir = path.join(workspace, "outputs")
  await mkdir(outputsDir, { recursive: true })
  await writeFile(path.join(workspace, "README.md"), `# Provider thread transfer drill for ${provider}\n`, "utf8")

  const client = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  const result = {
    drill: "local-reload",
    provider,
    status: "failed",
    started_at_ms: Date.now(),
    evidence: {},
    checks: {},
    errors: [],
  }

  let sessionId = null
  const kernelEvents = []
  try {
    client.onKernelEvent((event) => {
      kernelEvents.push({
        observed_at_ms: Date.now(),
        ...event,
      })
    })
    logStep(result, provider, "create-session", { workspace })
    const session = variant(
      await client.send(createSessionRequest(workspace, workspace, `provider-thread-${provider}`)),
      "SessionCreated",
    ).session
    sessionId = session.id
    logStep(result, provider, "attach-session", { sessionId })
    const attachment = variant(
      await client.send(attachToSessionRequest(session.id, `provider-thread-${provider}-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)

    logStep(result, provider, "install-mcp")
    const mcpName = `thread_transfer_probe_${provider.replaceAll("-", "_")}_${process.pid}`
    const mcpPath = await createDeterministicMcp(path.join(root, provider), mcpName)
    const installedMcp = variant(
      await client.send(installMcpServerRequest(workspace, mcpConfig(mcpName, mcpPath))),
      "McpServerInstalled",
    ).mcp
    result.evidence.installed_mcp = installedMcp

    const model = providerModel(provider, options)
    const effort = providerEffort(provider)
    logStep(result, provider, "spawn-agent", { model, effort })
    const agent = variant(
      await client.send(spawnAgentRequest(
        session.id,
        provider,
        `${provider}-thread-transfer`,
        model,
        workspace,
        effort,
      )),
      "AgentSpawned",
    ).agent
    result.evidence.agent = {
      id: agent.id,
      alias: agent.alias,
      provider: agent.provider,
      model: agent.model,
      effort: agent.effort,
    }

    logStep(result, provider, "launch-provider-run")
    const launched = variantAny(
      await client.send(launchProviderRunRequest(session.id, provider, "default", model, effort, agent.id)),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    logStep(result, provider, "wait-provider-run-ready", { providerRunId: launched.id })
    let beforeRun = await waitForProviderRun({
      client,
      providerRunId: launched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: false,
    })

    const rememberMarker = `THREAD_TRANSFER_${provider.replaceAll("-", "_").toUpperCase()}_${process.pid}_${Date.now()}`
    const readyMarker = `${rememberMarker}_READY`
    logStep(result, provider, "submit-initial-marker", { marker: rememberMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        agent.id,
        [
          `Remember this exact marker for a later recall check: ${rememberMarker}`,
          `Reply with exactly ${readyMarker} and nothing else.`,
        ].join("\n"),
        [],
      ),
      `submit initial marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
    logStep(result, provider, "initial-marker-submit-accepted")
    await waitForHistoryOutputMarker({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      marker: rememberMarker,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      historyDir: options.historyDir,
    })
    logStep(result, provider, "initial-marker-observed")
    await waitForPromptIdle({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    beforeRun = variant(await client.send(getProviderRunRequest(beforeRun.id)), "ProviderRun").provider_run
    const beforeThreadId = providerThreadId(beforeRun)
    if (!beforeThreadId) {
      throw new Error(`provider ${provider} did not expose a provider thread id before reload`)
    }
    result.evidence.before = providerRunSnapshot(beforeRun)
    result.evidence.remember_marker = rememberMarker

    logStep(result, provider, "grant-mcp", { mcpName })
    const grantResponse = await sendControlRequest(
      kernelUrl,
      grantAgentExtensionRequest(workspace, agent.id, "mcp", mcpName),
      `grant MCP ${mcpName}`,
      Math.min(options.timeoutMs, 60_000),
    )
    result.evidence.granted_agent = variant(grantResponse, "AgentExtensionGranted").agent
    logStep(result, provider, "wait-provider-reload", { previousRunId: beforeRun.id })
    const afterRun = await waitForActiveProviderRunChange({
      client,
      sessionId: session.id,
      previousRunId: beforeRun.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    const afterThreadId = providerThreadId(afterRun)
    result.evidence.after = providerRunSnapshot(afterRun)
    result.checks.provider_run_changed = beforeRun.id !== afterRun.id
    result.checks.provider_thread_id_before = beforeThreadId
    result.checks.provider_thread_id_after = afterThreadId
    result.checks.provider_thread_id_preserved = beforeThreadId === afterThreadId
    result.checks.mcp_loaded_after_reload = (afterRun.mcp_servers ?? []).some((server) => server.name === mcpName)

    if (!result.checks.provider_thread_id_preserved) {
      throw new Error(`provider thread id changed across reload: before=${beforeThreadId} after=${afterThreadId}`)
    }
    if (!result.checks.mcp_loaded_after_reload) {
      throw new Error(`reloaded run did not include MCP ${mcpName}`)
    }

    if (!options.skipRecallPrompt) {
      const recallMarker = `${rememberMarker}_SECOND_TURN_RECALLED`
      const recallObservationMarker = `${rememberMarker}_SECOND_TURN`
      logStep(result, provider, "submit-recall-marker", { marker: recallMarker })
      await sendControlRequest(
        kernelUrl,
        submitPromptRequest(
          session.id,
          attachment.id,
          agent.id,
          `Reply with exactly ${recallMarker} if you still remember the marker from the previous turn. Do not include any other text.`,
          [],
        ),
        `submit recall marker prompt for ${provider}`,
        Math.min(options.timeoutMs, 60_000),
      )
      logStep(result, provider, "recall-marker-submit-accepted")
      await waitForHistoryOutputMarker({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        marker: recallObservationMarker,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        historyDir: options.historyDir,
      })
      await waitForPromptIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
      result.checks.post_reload_recall_marker_observed = true
      result.evidence.recall_marker = recallMarker
      logStep(result, provider, "recall-marker-observed")
    }

    result.evidence.provider_processes = await collectProviderProcesses(client, provider)
    result.evidence.kernel_events = kernelEvents.slice(-50)
    result.status = "passed"
    return result
  } catch (error) {
    result.errors.push(error.stack ?? error.message ?? String(error))
    result.evidence.kernel_events = kernelEvents.slice(-50)
    result.evidence.provider_processes = await collectProviderProcesses(client, provider).catch((processError) => ({
      error: processError.message ?? String(processError),
    }))
    return result
  } finally {
    if (sessionId) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client.close().catch(() => {})
  }
}

async function selectWorkerKernel(client, workerMachineId, provider, timeoutMs, pollMs) {
  const kernels = await waitForRemoteMachine(client, workerMachineId, timeoutMs, pollMs)
  const selected = kernels.find((kernel) => {
    const providers = kernel.available_providers ?? []
    return kernel.accepting_remote_leases && providers.includes(provider)
  })
  if (!selected) {
    throw new Error(`no worker kernel on ${workerMachineId} advertises provider ${provider}: ${JSON.stringify(kernels, null, 2)}`)
  }
  return selected
}

async function runWorkerResumeScenario({ provider, root, kernelUrl, workerMachineId, workerKernelId, options }) {
  const workspace = path.join(root, provider, "workspace")
  await mkdir(workspace, { recursive: true })
  await writeFile(path.join(workspace, "README.md"), `# Worker resume provider thread transfer drill for ${provider}\n`, "utf8")

  const client = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  const result = {
    drill: "worker-resume",
    provider,
    status: "failed",
    started_at_ms: Date.now(),
    evidence: {
      worker_machine_id: workerMachineId,
      worker_kernel_id: workerKernelId,
      worker_state: options.workerState,
      scope: options.workerState === "isolated"
        ? "same-host worker with isolated provider home/data/cache and temporary copied auth; not a standard slice"
        : "same-host worker with shared provider credential/state directories; not a standard slice",
      same_arroba_agent_record: false,
    },
    checks: {},
    errors: [],
  }

  let sessionId = null
  const kernelEvents = []
  try {
    client.onKernelEvent((event) => {
      kernelEvents.push({
        observed_at_ms: Date.now(),
        ...event,
      })
    })

    logStep(result, provider, "create-session", { workspace })
    const session = variant(
      await client.send(createSessionRequest(workspace, workspace, `provider-thread-worker-${provider}`)),
      "SessionCreated",
    ).session
    sessionId = session.id
    const attachment = variant(
      await client.send(attachToSessionRequest(session.id, `provider-thread-worker-${provider}-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)

    const model = providerModel(provider, options)
    const effort = providerEffort(provider)
    logStep(result, provider, "spawn-local-agent", { model, effort })
    const localAgent = variant(
      await client.send(spawnAgentRequest(
        session.id,
        provider,
        `${provider}-local-source`,
        model,
        workspace,
        effort,
      )),
      "AgentSpawned",
    ).agent
    result.evidence.local_agent = { id: localAgent.id, alias: localAgent.alias, model: localAgent.model }

    logStep(result, provider, "launch-local-provider-run")
    const localLaunched = variantAny(
      await client.send(launchProviderRunRequest(session.id, provider, "default", model, effort, localAgent.id)),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    let localRun = await waitForProviderRun({
      client,
      providerRunId: localLaunched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: false,
    })

    const rememberMarker = `THREAD_TRANSFER_WORKER_${provider.replaceAll("-", "_").toUpperCase()}_${process.pid}_${Date.now()}`
    logStep(result, provider, "submit-local-marker", { marker: rememberMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        localAgent.id,
        [
          `Remember this exact marker for a worker resume check: ${rememberMarker}`,
          `Reply with exactly ${rememberMarker}_READY and nothing else.`,
        ].join("\n"),
        [],
      ),
      `submit local marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
    await waitForHistoryOutputMarker({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: localAgent.id,
      marker: rememberMarker,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      historyDir: options.historyDir,
    })
    await waitForPromptIdle({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: localAgent.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    localRun = variant(await client.send(getProviderRunRequest(localRun.id)), "ProviderRun").provider_run
    const beforeThreadId = providerThreadId(localRun)
    if (!beforeThreadId) throw new Error(`provider ${provider} did not expose a provider thread id before worker resume`)
    result.evidence.local_before = providerRunSnapshot(localRun)
    result.evidence.remember_marker = rememberMarker

    logStep(result, provider, "teardown-local-provider-process", { providerRunId: localRun.id })
    const teardown = variant(
      await sendControlRequest(
        kernelUrl,
        teardownProviderProcessesRequest(provider, true),
        `teardown local ${provider} provider process`,
        Math.min(options.timeoutMs, 60_000),
      ),
      "ProviderProcessesTornDown",
    )
    result.evidence.local_teardown = teardown
    const localAfterTeardown = await waitForProviderRunEnded({
      client,
      providerRunId: localRun.id,
      timeoutMs: Math.min(options.timeoutMs, 60_000),
      pollMs: Math.min(options.pollMs, 250),
    })
    result.evidence.local_after_teardown = providerRunSnapshot(localAfterTeardown)
    result.checks.local_run_ended_before_remote_launch = String(localAfterTeardown.state ?? "").toLowerCase() === "ended"
    if (!result.checks.local_run_ended_before_remote_launch) {
      throw new Error(`local provider run ${localRun.id} was not ended before remote launch: ${JSON.stringify(result.evidence.local_after_teardown)}`)
    }

    logStep(result, provider, "spawn-remote-agent", { workerKernelId })
    const remoteAgent = variant(
      await client.send(spawnAgentRequest(
        session.id,
        provider,
        `${provider}-worker-resume`,
        model,
        null,
        effort,
        undefined,
        undefined,
        workerKernelId,
      )),
      "AgentSpawned",
    ).agent
    result.evidence.remote_agent = {
      id: remoteAgent.id,
      alias: remoteAgent.alias,
      remote_execution: remoteAgent.remote_execution ?? null,
    }

    logStep(result, provider, "launch-remote-provider-run", { providerSessionId: beforeThreadId })
    const remoteLaunched = variantAny(
      await client.send(launchProviderRunRequest(
        session.id,
        provider,
        "default",
        model,
        effort,
        remoteAgent.id,
        { providerSessionId: beforeThreadId, nativeTui: true },
      )),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    const remoteRun = await waitForProviderRun({
      client,
      providerRunId: remoteLaunched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: true,
    })
    result.evidence.remote_after = providerRunSnapshot(remoteRun)
    const afterThreadId = providerThreadId(remoteRun)
    result.checks.provider_thread_id_before = beforeThreadId
    result.checks.provider_thread_id_after = afterThreadId
    result.checks.provider_thread_id_preserved = beforeThreadId === afterThreadId
    if (!result.checks.provider_thread_id_preserved) {
      throw new Error(`provider thread id changed across worker resume: before=${beforeThreadId} after=${afterThreadId}`)
    }

    const recallMarker = `${rememberMarker}_WORKER_RECALLED`
    const recallObservationMarker = `${rememberMarker}_WORKER`
    logStep(result, provider, "submit-worker-recall-marker", { marker: recallMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        remoteAgent.id,
        `Reply with exactly ${recallMarker} if you still remember the marker from the previous local provider thread turn. Do not include any other text.`,
        [],
      ),
      `submit worker recall marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
    await waitForHistoryOutputMarker({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: remoteAgent.id,
      marker: recallObservationMarker,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      historyDir: options.historyDir,
    })
    result.checks.worker_recall_marker_observed = true
    result.evidence.recall_marker = recallMarker

    result.evidence.provider_processes = await collectProviderProcesses(client, provider)
    result.evidence.kernel_events = kernelEvents.slice(-50)
    result.status = "passed"
    return result
  } catch (error) {
    result.errors.push(error.stack ?? error.message ?? String(error))
    result.evidence.kernel_events = kernelEvents.slice(-50)
    result.evidence.provider_processes = await collectProviderProcesses(client, provider).catch((processError) => ({
      error: processError.message ?? String(processError),
    }))
    return result
  } finally {
    if (sessionId) {
      await client.send(endSessionRequest(sessionId)).catch(() => {})
    }
    await client.close().catch(() => {})
  }
}

async function runSliceRestartScenario({ provider, root, kernelUrl, options }) {
  const workspace = path.join(root, provider, "workspace")
  await mkdir(workspace, { recursive: true })
  await writeFile(path.join(workspace, "README.md"), `# Slice restart provider thread transfer drill for ${provider}\n`, "utf8")

  const client = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  const result = {
    drill: "slice-restart",
    provider,
    status: "failed",
    started_at_ms: Date.now(),
    evidence: {
      scope: "home-managed local Docker slice save/restart with the same Arroba agent record",
      same_arroba_agent_record: true,
    },
    checks: {},
    errors: [],
  }

  let sessionId = null
  let sliceId = null
  const kernelEvents = []
  try {
    client.onKernelEvent((event) => {
      kernelEvents.push({
        observed_at_ms: Date.now(),
        ...event,
      })
    })

    const sliceName = `provider-thread-slice-${provider.replaceAll("-", "_")}-${process.pid}`
    logStep(result, provider, "create-slice", { sliceName, workspace })
    const createdSlice = variant(
      await withTimeout(
        client.send(createSliceRequest({
          name: sliceName,
          backend: "local_docker",
          os: "linux",
          displayMode: "headless",
          workspaceId: workspace,
          worktreeId: workspace,
          workspaceMount: workspace,
        })),
        `create slice for ${provider}`,
        Math.min(options.timeoutMs, 120_000),
      ),
      "SliceCreated",
    ).slice
    sliceId = createdSlice.id
    result.evidence.slice_created = sliceRecordSnapshot(createdSlice)

    logStep(result, provider, "start-slice", { sliceId })
    const startedSlice = variant(
      await withTimeout(
        client.send(startSliceRequest(sliceId)),
        `start slice for ${provider}`,
        options.timeoutMs,
      ),
      "SliceStarted",
    ).slice
    result.evidence.slice_started = sliceRecordSnapshot(startedSlice)
    const readySlice = await waitForSliceWorkerProvider({
      client,
      sliceRef: sliceId,
      provider,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
    })
    result.evidence.slice_ready_before_restart = sliceRecordSnapshot(readySlice)

    logStep(result, provider, "import-slice-provider-auth", { authProvider: providerAuthName(provider) })
    const authImported = variant(
      await withTimeout(
        client.send(importSliceProviderAuthRequest(sliceId, providerAuthName(provider))),
        `import slice provider auth for ${provider}`,
        Math.min(options.timeoutMs, 120_000),
      ),
      "SliceProviderAuthImported",
    ).slice
    result.evidence.slice_auth_imported = sliceRecordSnapshot(authImported)

    logStep(result, provider, "create-session", { workspace })
    const session = variant(
      await client.send(createSessionRequest(workspace, workspace, `provider-thread-slice-${provider}`)),
      "SessionCreated",
    ).session
    sessionId = session.id
    const attachment = variant(
      await client.send(attachToSessionRequest(session.id, `provider-thread-slice-${provider}-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)

    const model = providerModel(provider, options)
    const effort = providerEffort(provider)
    logStep(result, provider, "spawn-slice-agent", { model, effort, sliceId })
    const agent = variant(
      await client.send(spawnAgentRequest(
        session.id,
        provider,
        `${provider}-slice-transfer`,
        model,
        workspace,
        effort,
        undefined,
        undefined,
        undefined,
        undefined,
        sliceId,
      )),
      "AgentSpawned",
    ).agent
    result.evidence.agent = {
      id: agent.id,
      alias: agent.alias,
      provider: agent.provider,
      model: agent.model,
      effort: agent.effort,
      remote_execution: agent.remote_execution ?? null,
    }

    logStep(result, provider, "launch-slice-provider-run")
    const launched = variantAny(
      await client.send(launchProviderRunRequest(
        session.id,
        provider,
        "default",
        model,
        effort,
        agent.id,
        { nativeTui: true },
      )),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    let beforeRun = await waitForProviderRun({
      client,
      providerRunId: launched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: false,
    })

    const rememberMarker = `THREAD_TRANSFER_SLICE_${provider.replaceAll("-", "_").toUpperCase()}_${process.pid}_${Date.now()}`
    logStep(result, provider, "submit-slice-marker", { marker: rememberMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        agent.id,
        [
          `Remember this exact marker for a slice restart check: ${rememberMarker}`,
          `Reply with exactly ${rememberMarker}_READY and nothing else.`,
        ].join("\n"),
        [],
      ),
      `submit slice marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
    await waitForHistoryOutputMarker({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      marker: rememberMarker,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      historyDir: options.historyDir,
    })
    await waitForPromptIdle({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    beforeRun = variant(await client.send(getProviderRunRequest(beforeRun.id)), "ProviderRun").provider_run
    const beforeThreadId = providerThreadId(beforeRun)
    if (!beforeThreadId) throw new Error(`provider ${provider} did not expose a provider thread id before slice restart`)
    const sliceBeforeRestart = variant(await client.send(getSliceRequest(sliceId)), "Slice").slice
    result.evidence.before = providerRunSnapshot(beforeRun)
    result.evidence.slice_before_restart = sliceRecordSnapshot(sliceBeforeRestart)
    result.evidence.remember_marker = rememberMarker

    logStep(result, provider, "save-slice-state-restart-agents", { sliceId, providerSessionId: beforeThreadId })
    const savedState = variant(
      await withTimeout(
        client.send(saveSliceStateRequest(sliceId, "restart_agents", "this_slice")),
        `save and restart slice for ${provider}`,
        options.timeoutMs,
      ),
      "SliceStateSaved",
    )
    result.evidence.slice_state_saved = {
      slice: sliceRecordSnapshot(savedState.slice),
      state: sliceSavedStateSnapshot(savedState.state),
    }

    const afterRun = await waitForSessionActiveProviderRun({
      client,
      sessionId: session.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
    })
    const afterThreadId = providerThreadId(afterRun)
    const afterState = variantAny(await client.send(getSessionStateRequest(session.id)), "SessionState", "SessionStateLoaded")
    const afterSession = afterState.session ?? afterState
    const afterAgent = (afterSession.agents ?? []).find((entry) => entry.id === agent.id)
    result.evidence.after = providerRunSnapshot(afterRun)
    result.evidence.agent_after_restart = {
      id: afterAgent?.id ?? null,
      alias: afterAgent?.alias ?? null,
      remote_execution: afterAgent?.remote_execution ?? null,
    }
    result.checks.same_arroba_agent_record = afterAgent?.id === agent.id
    result.checks.provider_thread_id_before = beforeThreadId
    result.checks.provider_thread_id_after = afterThreadId
    result.checks.provider_thread_id_preserved = beforeThreadId === afterThreadId
    result.checks.slice_worker_restarted = (
      result.evidence.slice_before_restart.worker_kernel_id
      && result.evidence.slice_state_saved.slice.worker_kernel_id
      && result.evidence.slice_before_restart.worker_kernel_id !== result.evidence.slice_state_saved.slice.worker_kernel_id
    )
    if (!result.checks.same_arroba_agent_record) {
      throw new Error(`slice restart did not preserve same Arroba agent record ${agent.id}`)
    }
    if (!result.checks.provider_thread_id_preserved) {
      throw new Error(`provider thread id changed across slice restart: before=${beforeThreadId} after=${afterThreadId}`)
    }

    const recallMarker = `${rememberMarker}_SLICE_RECALLED`
    const recallObservationMarker = `${rememberMarker}_SLICE`
    logStep(result, provider, "submit-slice-recall-marker", { marker: recallMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        agent.id,
        `Reply with exactly ${recallMarker} if you still remember the marker from before the slice restart. Do not include any other text.`,
        [],
      ),
      `submit slice recall marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
    await waitForHistoryOutputMarker({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      marker: recallObservationMarker,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      historyDir: options.historyDir,
    })
    result.checks.slice_recall_marker_observed = true
    result.evidence.recall_marker = recallMarker

    result.evidence.provider_processes = await collectProviderProcesses(client, provider)
    result.evidence.kernel_events = kernelEvents.slice(-50)
    result.status = "passed"
    return result
  } catch (error) {
    result.errors.push(error.stack ?? error.message ?? String(error))
    result.evidence.kernel_events = kernelEvents.slice(-50)
    result.evidence.provider_processes = await collectProviderProcesses(client, provider).catch((processError) => ({
      error: processError.message ?? String(processError),
    }))
    return result
  } finally {
    if (sessionId) {
      await client.send(endSessionRequest(sessionId)).catch((error) => {
        result.evidence.session_cleanup_error = error.message ?? String(error)
      })
    }
    if (sliceId && !(options.keepSliceOnFailure && result.status !== "passed")) {
      await client.send(deleteSliceRequest(sliceId)).catch((error) => {
        result.evidence.slice_cleanup_error = error.message ?? String(error)
      })
    } else if (sliceId) {
      result.evidence.slice_left_running_for_debug = sliceId
    }
    await client.close().catch(() => {})
  }
}

async function runLiveMigrateToSliceScenario({ provider, root, kernelUrl, options }) {
  const roundTrip = options.drill === "live-migrate-roundtrip-slice"
  const workspace = path.join(root, provider, "workspace")
  await mkdir(workspace, { recursive: true })
  await writeFile(path.join(workspace, "README.md"), `# Live local-to-slice provider thread transfer drill for ${provider}\n`, "utf8")

  const client = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  const result = {
    drill: options.drill,
    provider,
    status: "failed",
    started_at_ms: Date.now(),
    evidence: {
      scope: roundTrip
        ? "provider thread starts on the main machine, the same Arroba agent record is moved to a local Docker slice and then back to local execution, and both provider runs resume the captured provider thread"
        : "provider thread starts on the main machine, the same Arroba agent record is moved to a local Docker slice, and the slice provider run resumes the captured provider thread",
      same_arroba_agent_record_required: true,
      same_provider_thread_required: true,
    },
    checks: {},
    errors: [],
  }

  let sessionId = null
  let sliceId = null
  const kernelEvents = []
  try {
    client.onKernelEvent((event) => {
      kernelEvents.push({
        observed_at_ms: Date.now(),
        ...event,
      })
    })

    logStep(result, provider, "create-local-session", { workspace })
    const session = variant(
      await client.send(createSessionRequest(workspace, workspace, `provider-thread-live-migrate-${provider}`)),
      "SessionCreated",
    ).session
    sessionId = session.id
    const attachment = variant(
      await client.send(attachToSessionRequest(session.id, `provider-thread-live-migrate-${provider}-${Date.now()}`)),
      "SessionAttached",
    ).attachment
    await client.subscribeToKernelEvents(session.id, attachment.id)

    const model = providerModel(provider, options)
    const effort = providerEffort(provider)
    logStep(result, provider, "spawn-local-agent", { model, effort })
    const agent = variant(
      await client.send(spawnAgentRequest(
        session.id,
        provider,
        `${provider}-live-migrate`,
        model,
        workspace,
        effort,
      )),
      "AgentSpawned",
    ).agent
    result.evidence.agent_before_migration = {
      id: agent.id,
      alias: agent.alias,
      provider: agent.provider,
      model: agent.model,
      effort: agent.effort,
      remote_execution: agent.remote_execution ?? null,
    }

    logStep(result, provider, "launch-local-provider-run")
    const launched = variantAny(
      await client.send(launchProviderRunRequest(session.id, provider, "default", model, effort, agent.id)),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    let localRun = await waitForProviderRun({
      client,
      providerRunId: launched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: false,
    })

    const rememberMarker = `THREAD_TRANSFER_LIVE_SLICE_${provider.replaceAll("-", "_").toUpperCase()}_${process.pid}_${Date.now()}`
    logStep(result, provider, "submit-local-marker", { marker: rememberMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        agent.id,
        [
          `Remember this exact marker for a live local-to-slice migration check: ${rememberMarker}`,
          `Reply with exactly ${rememberMarker}_READY and nothing else.`,
        ].join("\n"),
        [],
      ),
      `submit local marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
    await waitForHistoryOutputMarker({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      marker: rememberMarker,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      historyDir: options.historyDir,
    })
    await waitForPromptIdle({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })
    localRun = variant(await client.send(getProviderRunRequest(localRun.id)), "ProviderRun").provider_run
    const beforeThreadId = providerThreadId(localRun)
    if (!beforeThreadId) throw new Error(`provider ${provider} did not expose a provider thread id before live slice migration`)
    result.evidence.local_before = providerRunSnapshot(localRun)
    result.evidence.remember_marker = rememberMarker

    const sliceName = `provider-thread-live-${provider.replaceAll("-", "_")}-${process.pid}`
    logStep(result, provider, "create-slice", { sliceName, workspace })
    const createdSlice = variant(
      await withTimeout(
        client.send(createSliceRequest({
          name: sliceName,
          backend: "local_docker",
          os: "linux",
          displayMode: "headless",
          workspaceId: workspace,
          worktreeId: workspace,
          workspaceMount: workspace,
        })),
        `create live migration slice for ${provider}`,
        Math.min(options.timeoutMs, 120_000),
      ),
      "SliceCreated",
    ).slice
    sliceId = createdSlice.id
    result.evidence.slice_created = sliceRecordSnapshot(createdSlice)

    logStep(result, provider, "start-slice", { sliceId })
    const startedSlice = variant(
      await withTimeout(
        client.send(startSliceRequest(sliceId)),
        `start live migration slice for ${provider}`,
        options.timeoutMs,
      ),
      "SliceStarted",
    ).slice
    result.evidence.slice_started = sliceRecordSnapshot(startedSlice)
    const readySlice = await waitForSliceWorkerProvider({
      client,
      sliceRef: sliceId,
      provider,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
    })
    result.evidence.slice_ready = sliceRecordSnapshot(readySlice)

    logStep(result, provider, "transfer-provider-state-to-slice", { sliceName })
    result.evidence.provider_state_transfer = await transferProviderStateToSlice({
      provider,
      root,
      sliceName,
      timeoutMs: options.timeoutMs,
      providerEnv: options.providerStateSourceEnv ?? realProviderEnv(),
    })

    logStep(result, provider, "import-slice-provider-auth", { authProvider: providerAuthName(provider) })
    const authImported = variant(
      await withTimeout(
        client.send(importSliceProviderAuthRequest(sliceId, providerAuthName(provider))),
        `import slice provider auth for ${provider}`,
        Math.min(options.timeoutMs, 120_000),
      ),
      "SliceProviderAuthImported",
    ).slice
    result.evidence.slice_auth_imported = sliceRecordSnapshot(authImported)

    const machineRef = readySlice.worker_machine_id ?? `slice:${sliceId}`
    logStep(result, provider, "move-same-agent-to-slice", { agentId: agent.id, machineRef })
    const movedAgent = variant(
      await withTimeout(
        client.send(moveAgentToRemoteRequest(session.id, agent.id, machineRef)),
        `move same agent to slice for ${provider}`,
        Math.min(options.timeoutMs, 120_000),
      ),
      "AgentMovedToRemote",
    ).agent
    result.evidence.agent_after_move = {
      id: movedAgent.id,
      alias: movedAgent.alias,
      provider: movedAgent.provider,
      model: movedAgent.model,
      effort: movedAgent.effort,
      remote_execution: movedAgent.remote_execution ?? null,
    }
    result.checks.same_arroba_agent_record_after_move = movedAgent.id === agent.id
    if (!result.checks.same_arroba_agent_record_after_move) {
      throw new Error(`move returned a different Arroba agent record: before=${agent.id} after=${movedAgent.id}`)
    }
    if (!movedAgent.remote_execution) {
      throw new Error(`agent ${agent.id} was not remote-backed after move to slice`)
    }
    const localAfterMove = await waitForProviderRunEnded({
      client,
      providerRunId: localRun.id,
      timeoutMs: Math.min(options.timeoutMs, 60_000),
      pollMs: Math.min(options.pollMs, 250),
    })
    result.evidence.local_after_move = providerRunSnapshot(localAfterMove)
    result.checks.local_run_ended_by_move = String(localAfterMove.state ?? "").toLowerCase() === "ended"
    if (!result.checks.local_run_ended_by_move) {
      throw new Error(`local provider run ${localRun.id} was not ended by move: ${JSON.stringify(result.evidence.local_after_move)}`)
    }

    logStep(result, provider, "launch-slice-provider-run", { providerSessionId: beforeThreadId })
    const sliceLaunched = variantAny(
      await client.send(launchProviderRunRequest(
        session.id,
        provider,
        "default",
        model,
        effort,
        agent.id,
        { providerSessionId: beforeThreadId, nativeTui: true },
      )),
      "ProviderRunLaunchAccepted",
      "ProviderRunLaunched",
    ).provider_run
    const sliceRun = await waitForProviderRun({
      client,
      providerRunId: sliceLaunched.id,
      timeoutMs: Math.min(options.timeoutMs, 180_000),
      pollMs: options.pollMs,
      requireThreadId: true,
    })
    const afterThreadId = providerThreadId(sliceRun)
    result.evidence.slice_after = providerRunSnapshot(sliceRun)
    result.checks.provider_thread_id_before = beforeThreadId
    result.checks.provider_thread_id_after = afterThreadId
    result.checks.provider_thread_id_preserved = beforeThreadId === afterThreadId
    if (!result.checks.provider_thread_id_preserved) {
      throw new Error(`provider thread id changed across live slice migration: before=${beforeThreadId} after=${afterThreadId}`)
    }

    const recallMarker = `${rememberMarker}_SLICE_MIGRATED`
    const recallObservationMarker = `${rememberMarker}_SLICE`
    logStep(result, provider, "submit-slice-recall-marker", { marker: recallMarker })
    await sendControlRequest(
      kernelUrl,
      submitPromptRequest(
        session.id,
        attachment.id,
        agent.id,
        `Reply with exactly ${recallMarker} if you still remember the marker from before the local-to-slice migration. Do not include any other text.`,
        [],
      ),
      `submit slice migration recall marker prompt for ${provider}`,
      Math.min(options.timeoutMs, 60_000),
    )
    await waitForHistoryOutputMarker({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      marker: recallObservationMarker,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
      historyDir: options.historyDir,
    })
    await waitForPromptIdle({
      client,
      sessionId: session.id,
      attachmentId: attachment.id,
      agentId: agent.id,
      timeoutMs: options.timeoutMs,
      pollMs: options.pollMs,
    })

    if (roundTrip) {
      const reverseMarker = `${rememberMarker}_RETURN_CONTEXT`
      logStep(result, provider, "submit-slice-return-marker", { marker: reverseMarker })
      await sendControlRequest(
        kernelUrl,
        submitPromptRequest(
          session.id,
          attachment.id,
          agent.id,
          [
            `Remember this exact marker for the slice-to-local return check: ${reverseMarker}`,
            `Reply with exactly ${reverseMarker}_READY and nothing else.`,
          ].join("\n"),
          [],
        ),
        `submit slice return marker prompt for ${provider}`,
        Math.min(options.timeoutMs, 60_000),
      )
      await waitForHistoryOutputMarker({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        marker: reverseMarker,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        historyDir: options.historyDir,
      })
      await waitForPromptIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })

      logStep(result, provider, "transfer-provider-state-from-slice", { sliceName })
      result.evidence.provider_state_reverse_transfer = await transferProviderStateFromSlice({
        provider,
        root,
        sliceName,
        timeoutMs: options.timeoutMs,
        providerEnv: options.providerStateSourceEnv ?? realProviderEnv(),
      })

      const sliceRunBeforeReturn = variant(await client.send(getProviderRunRequest(sliceRun.id)), "ProviderRun").provider_run
      result.evidence.slice_before_return = providerRunSnapshot(sliceRunBeforeReturn)
      logStep(result, provider, "move-same-agent-to-local", { agentId: agent.id })
      const returnedAgent = variant(
        await withTimeout(
          client.send(moveAgentToLocalRequest(session.id, agent.id)),
          `move same agent back local for ${provider}`,
          Math.min(options.timeoutMs, 120_000),
        ),
        "AgentMovedToLocal",
      ).agent
      result.evidence.agent_after_return = {
        id: returnedAgent.id,
        alias: returnedAgent.alias,
        provider: returnedAgent.provider,
        model: returnedAgent.model,
        effort: returnedAgent.effort,
        remote_execution: returnedAgent.remote_execution ?? null,
      }
      result.checks.same_arroba_agent_record_after_return = returnedAgent.id === agent.id
      result.checks.agent_local_after_return = returnedAgent.remote_execution == null
      if (!result.checks.same_arroba_agent_record_after_return) {
        throw new Error(`return move returned a different Arroba agent record: before=${agent.id} after=${returnedAgent.id}`)
      }
      if (!result.checks.agent_local_after_return) {
        throw new Error(`agent ${agent.id} was still remote-backed after move back to local`)
      }

      const sliceRunAfterReturn = await waitForProviderRunEnded({
        client,
        providerRunId: sliceRun.id,
        timeoutMs: Math.min(options.timeoutMs, 60_000),
        pollMs: Math.min(options.pollMs, 250),
      })
      result.evidence.slice_after_return = providerRunSnapshot(sliceRunAfterReturn)
      result.checks.slice_run_ended_by_return_move = String(sliceRunAfterReturn.state ?? "").toLowerCase() === "ended"
      if (!result.checks.slice_run_ended_by_return_move) {
        throw new Error(`slice provider run ${sliceRun.id} was not ended by return move: ${JSON.stringify(result.evidence.slice_after_return)}`)
      }

      logStep(result, provider, "launch-returned-local-provider-run", { providerSessionId: afterThreadId })
      const returnedLaunched = variantAny(
        await client.send(launchProviderRunRequest(
          session.id,
          provider,
          "default",
          model,
          effort,
          agent.id,
          { providerSessionId: afterThreadId, nativeTui: true },
        )),
        "ProviderRunLaunchAccepted",
        "ProviderRunLaunched",
      ).provider_run
      const returnedLocalRun = await waitForProviderRun({
        client,
        providerRunId: returnedLaunched.id,
        timeoutMs: Math.min(options.timeoutMs, 180_000),
        pollMs: options.pollMs,
        requireThreadId: true,
      })
      const returnedThreadId = providerThreadId(returnedLocalRun)
      result.evidence.local_after_return = providerRunSnapshot(returnedLocalRun)
      result.checks.provider_thread_id_returned = returnedThreadId
      result.checks.provider_thread_id_preserved_after_return = beforeThreadId === returnedThreadId
      if (!result.checks.provider_thread_id_preserved_after_return) {
        throw new Error(`provider thread id changed across slice-to-local return: before=${beforeThreadId} returned=${returnedThreadId}`)
      }

      const returnRecallMarker = `${rememberMarker}_LOCAL_RETURNED`
      logStep(result, provider, "submit-returned-local-recall-marker", { marker: returnRecallMarker })
      await sendControlRequest(
        kernelUrl,
        submitPromptRequest(
          session.id,
          attachment.id,
          agent.id,
          `Reply with exactly ${returnRecallMarker} if you remember both ${rememberMarker} and ${reverseMarker} after returning from the slice to local execution. Do not include any other text.`,
          [],
        ),
        `submit returned local recall marker prompt for ${provider}`,
        Math.min(options.timeoutMs, 60_000),
      )
      await waitForHistoryOutputMarker({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        marker: returnRecallMarker,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
        historyDir: options.historyDir,
      })
      await waitForPromptIdle({
        client,
        sessionId: session.id,
        attachmentId: attachment.id,
        agentId: agent.id,
        timeoutMs: options.timeoutMs,
        pollMs: options.pollMs,
      })
      result.evidence.return_marker = reverseMarker
      result.evidence.return_recall_marker = returnRecallMarker
      result.checks.return_recall_marker_observed = true
    }

    const finalState = variantAny(await client.send(getSessionStateRequest(session.id)), "SessionState", "SessionStateLoaded")
    const finalSession = finalState.session ?? finalState
    const finalAgent = (finalSession.agents ?? []).find((entry) => entry.id === agent.id)
    const localRunFinal = variant(await client.send(getProviderRunRequest(localRun.id)), "ProviderRun").provider_run
    result.evidence.agent_final = {
      id: finalAgent?.id ?? null,
      alias: finalAgent?.alias ?? null,
      provider: finalAgent?.provider ?? null,
      model: finalAgent?.model ?? null,
      effort: finalAgent?.effort ?? null,
      remote_execution: finalAgent?.remote_execution ?? null,
      provider_resume_state: finalAgent?.provider_resume_state ?? null,
    }
    result.evidence.local_run_final = providerRunSnapshot(localRunFinal)
    result.checks.same_arroba_agent_record_final = finalAgent?.id === agent.id
    result.checks.local_run_still_ended_after_slice_launch = String(localRunFinal.state ?? "").toLowerCase() === "ended"
    result.checks.agent_execution_location_final = finalAgent?.remote_execution == null ? "local" : "remote"
    const expectedModel = provider === "codex" && agent.model && !agent.model.startsWith("codex/")
      ? `codex/${agent.model}`
      : agent.model
    result.checks.agent_original_provider_config_restored = finalAgent?.provider === agent.provider
      && (finalAgent?.model === agent.model || finalAgent?.model === expectedModel)
      && finalAgent?.effort === agent.effort
    result.checks.slice_recall_marker_observed = true
    result.evidence.recall_marker = recallMarker
    if (!result.checks.same_arroba_agent_record_final) {
      throw new Error(`same Arroba agent record was not present after migration: ${agent.id}`)
    }
    if (!result.checks.local_run_still_ended_after_slice_launch) {
      throw new Error(`old local provider run was not still ended after slice launch: ${JSON.stringify(result.evidence.local_run_final)}`)
    }
    if (roundTrip && result.checks.agent_execution_location_final !== "local") {
      throw new Error(`agent ${agent.id} did not finish local after round trip: ${JSON.stringify(result.evidence.agent_final)}`)
    }
    if (!result.checks.agent_original_provider_config_restored) {
      throw new Error(`agent ${agent.id} provider config changed across migration: ${JSON.stringify(result.evidence.agent_final)}`)
    }

    result.evidence.provider_processes = await collectProviderProcesses(client, provider)
    result.evidence.kernel_events = kernelEvents.slice(-80)
    result.status = "passed"
    return result
  } catch (error) {
    result.errors.push(error.stack ?? error.message ?? String(error))
    result.evidence.kernel_events = kernelEvents.slice(-80)
    result.evidence.provider_processes = await collectProviderProcesses(client, provider).catch((processError) => ({
      error: processError.message ?? String(processError),
    }))
    return result
  } finally {
    if (sessionId) {
      await client.send(endSessionRequest(sessionId)).catch((error) => {
        result.evidence.session_cleanup_error = error.message ?? String(error)
      })
    }
    if (sliceId && !(options.keepSliceOnFailure && result.status !== "passed")) {
      await client.send(deleteSliceRequest(sliceId)).catch((error) => {
        result.evidence.slice_cleanup_error = error.message ?? String(error)
      })
    } else if (sliceId) {
      result.evidence.slice_left_running_for_debug = sliceId
    }
    await client.close().catch(() => {})
  }
}

async function runWorkerResumeMatrix({ options, root, ports }) {
  await assertBinary(relayBinary, path.join(repoRoot, "apps/relay/Cargo.toml"), "arroba-relay")
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const homeKernelUrl = `ws://127.0.0.1:${ports.homeKernelPort}`
  const workerKernelUrl = `ws://127.0.0.1:${ports.workerKernelPort}`
  const realProvider = realProviderEnv()
  const isolatedWorker = options.workerState === "isolated"
    ? await prepareIsolatedWorkerProviderEnv()
    : null
  const workerProvider = isolatedWorker?.providerEnv ?? realProvider
  const relayEnv = {
    ...process.env,
    ARROBA_RELAY_HOST: "127.0.0.1",
    ARROBA_RELAY_PORT: String(ports.relayPort),
    ARROBA_RELAY_SCOPED_ISSUER: RELAY_ISSUER,
    ARROBA_RELAY_SCOPED_HMAC_SECRET: RELAY_SECRET,
  }
  const homeDaemonId = `provider-thread-home-${process.pid}-${Date.now()}`
  const workerDaemonId = `provider-thread-worker-${process.pid}-${Date.now()}`
  const workerMachineId = `provider-thread-worker-machine-${process.pid}`
  const clientRelayToken = signRelayToken(relayClaims({
    subject: `provider-thread-client-${process.pid}-${Date.now()}`,
    subjectKind: "client",
    actions: ["client_connect", "client_metadata_read", "packet_route"],
  }))
  const homeRelayToken = signRelayToken(relayClaims({
    subject: homeDaemonId,
    subjectKind: "kernel",
    actions: ["daemon_register", "daemon_heartbeat", "peer_request", "peer_event", "client_metadata_read"],
  }))
  const workerRelayToken = signRelayToken(relayClaims({
    subject: workerDaemonId,
    subjectKind: "kernel",
    actions: ["daemon_register", "daemon_heartbeat", "peer_request", "peer_event", "client_metadata_read"],
  }))

  const homeEnv = workerResumeDaemonEnv({
    ports,
    root,
    relayToken: homeRelayToken,
    daemonId: homeDaemonId,
    daemonAlias: "home",
    machineId: `provider-thread-home-machine-${process.pid}`,
    machineAlias: "provider-thread-home",
    acceptRemoteLeases: false,
    socketName: "home-daemon.sock",
    kernelPort: ports.homeKernelPort,
    mcpPort: ports.homeMcpPort,
    openCodePort: ports.homeOpenCodePort,
    codexPort: ports.homeCodexPort,
    providerEnv: realProvider,
  })
  const workerEnv = workerResumeDaemonEnv({
    ports,
    root,
    relayToken: workerRelayToken,
    daemonId: workerDaemonId,
    daemonAlias: "worker",
    machineId: workerMachineId,
    machineAlias: "provider-thread-worker",
    acceptRemoteLeases: true,
    socketName: "worker-daemon.sock",
    kernelPort: ports.workerKernelPort,
    mcpPort: ports.workerMcpPort,
    openCodePort: ports.workerOpenCodePort,
    codexPort: ports.workerCodexPort,
    providerEnv: workerProvider,
  })

  let relayChild = null
  let homeChild = null
  let workerChild = null
  const matrix = {
    goal: "provider-thread-transfer",
    drill: options.drill,
    run_id: path.basename(root),
    relay_url: relayUrl,
    home_kernel_url: homeKernelUrl,
    worker_kernel_url: workerKernelUrl,
    worker_machine_id: workerMachineId,
    worker_state: options.workerState,
    worker_provider_environment: isolatedWorker?.evidence ?? {
      mode: "shared",
      provider_data_shared: true,
      provider_cache_shared: true,
      provider_home_shared: true,
    },
    providers: options.providers,
    started_at_ms: Date.now(),
    results: [],
  }
  try {
    relayChild = spawnLogged(relayBinary, [], {
      cwd: repoRoot,
      env: relayEnv,
      stdoutPath: path.join(root, "relay.stdout.log"),
      stderrPath: path.join(root, "relay.stderr.log"),
    })
    homeChild = spawnLogged(kernelBinary, [], {
      cwd: repoRoot,
      env: homeEnv,
      stdoutPath: path.join(root, "home-kernel.stdout.log"),
      stderrPath: path.join(root, "home-kernel.stderr.log"),
    })
    workerChild = spawnLogged(kernelBinary, [], {
      cwd: repoRoot,
      env: workerEnv,
      stdoutPath: path.join(root, "worker-kernel.stdout.log"),
      stderrPath: path.join(root, "worker-kernel.stderr.log"),
    })

    await waitForLocalDaemon(homeKernelUrl, root, root)
    await waitForLocalDaemon(workerKernelUrl, root, root)
    await waitForRelayTarget(relayUrl, clientRelayToken, "home", Math.min(options.timeoutMs, 120_000), options.pollMs)
    await waitForRelayTarget(relayUrl, clientRelayToken, "worker", Math.min(options.timeoutMs, 120_000), options.pollMs)

    const client = new LocalIpcClient(homeKernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await client.send({ ConfigureRelay: { relay_url: relayUrl, relay_token: homeRelayToken } })
      for (const provider of options.providers) {
        const workerKernel = await selectWorkerKernel(
          client,
          workerMachineId,
          provider,
          Math.min(options.timeoutMs, 120_000),
          options.pollMs,
        )
        const result = await runWorkerResumeScenario({
          provider,
          root,
          kernelUrl: homeKernelUrl,
          workerMachineId,
          workerKernelId: workerKernel.kernel_id,
          options,
        })
        matrix.results.push(result)
        await writeFile(path.join(root, `${provider}-worker-resume-result.json`), `${JSON.stringify(result, null, 2)}\n`, "utf8")
        console.log(`${provider}: ${result.status}`)
        if (result.status !== "passed") {
          console.log(result.errors.join("\n"))
        }
      }
    } finally {
      await client.close().catch(() => {})
    }
  } finally {
    matrix.finished_at_ms = Date.now()
    matrix.passed = matrix.results.length > 0 && matrix.results.every((result) => result.status === "passed")
    await writeFile(path.join(root, "matrix.json"), `${JSON.stringify(matrix, null, 2)}\n`, "utf8")
    await terminateChild(workerChild)
    await terminateChild(homeChild)
    await terminateChild(relayChild)
    if (isolatedWorker?.secretRoot) {
      await rm(isolatedWorker.secretRoot, { recursive: true, force: true })
    }
  }
  return matrix
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  await assertBinary(kernelBinary, path.join(repoRoot, "apps/kernel/Cargo.toml"), "arroba-kernel")

  const runId = `${Date.now()}-${process.pid}`
  const root = path.join(artifactsRoot, runId)
  await mkdir(root, { recursive: true })
  const ports = options.drill === "worker-resume"
    ? await makeWorkerResumePorts()
    : await makeAvailablePorts()
  const kernelUrl = options.kernel ?? `ws://127.0.0.1:${ports.kernelPort}`
  const historyDir = path.join(root, "history")
  const capabilityRoot = path.join(root, "capabilities")
  const sliceMode = options.drill === "slice-restart"
    || options.drill === "live-migrate-to-slice"
    || options.drill === "live-migrate-roundtrip-slice"
  const sliceXdgConfigHome = path.join(root, "xdg-config")
  const sliceXdgStateHome = path.join(root, "xdg-state")
  const sliceXdgDataHome = path.join(root, "xdg-data")
  const sliceXdgCacheHome = path.join(root, "xdg-cache")
  const sliceRoot = path.join(root, "slices")
  await mkdir(historyDir, { recursive: true })
  await mkdir(capabilityRoot, { recursive: true })
  options.historyDir = historyDir
  let sliceImageBuild = null
  if (sliceMode) {
    await mkdir(path.join(sliceXdgConfigHome, "arroba"), { recursive: true })
    await mkdir(sliceXdgStateHome, { recursive: true })
    await mkdir(sliceXdgDataHome, { recursive: true })
    await mkdir(sliceXdgCacheHome, { recursive: true })
    await mkdir(sliceRoot, { recursive: true })
    await writeFile(path.join(sliceXdgConfigHome, "arroba", "config.toml"), [
      "version = 1",
      "",
      "[slices]",
      `root = ${JSON.stringify(sliceRoot)}`,
      "",
      "[slices.linux]",
      `docker_image = ${JSON.stringify(defaultLocalDockerSliceImage)}`,
      `build_image = ${JSON.stringify(options.sliceBuildImage === "always" ? "auto" : options.sliceBuildImage)}`,
      "",
    ].join("\n"), "utf8")
    console.log(`slice-restart: prebuild image policy ${options.sliceBuildImage}`)
    sliceImageBuild = await prebuildLocalDockerSliceImageIfNeeded(root, options.sliceBuildImage, options.timeoutMs)
  }
  const sliceModeProviderEnv = sliceMode
    ? await prepareSliceModeProviderEnv(root)
    : null
  if (sliceModeProviderEnv) {
    options.providerStateSourceEnv = sliceModeProviderEnv
  }

  if (options.drill === "worker-resume") {
    const matrix = await runWorkerResumeMatrix({ options, root, ports })
    console.log(`provider thread transfer drill artifacts: ${root}`)
    if (!matrix.passed) {
      throw new Error(`provider thread transfer drill failed; see ${path.join(root, "matrix.json")}`)
    }
    if (options.cleanupOnSuccess) {
      await rm(root, { recursive: true, force: true })
    }
    return
  }

  let daemonChild = null
  const matrix = {
    goal: "provider-thread-transfer",
    drill: options.drill,
    run_id: runId,
    kernel_url: kernelUrl,
    providers: options.providers,
    ...(sliceMode ? {
      slice_image: defaultLocalDockerSliceImage,
      slice_build_image: options.sliceBuildImage,
      slice_root: sliceRoot,
      slice_image_build: sliceImageBuild,
    } : {}),
    started_at_ms: Date.now(),
    results: [],
  }

  try {
    if (options.spawnDaemon) {
      const stdout = createWriteStream(path.join(root, "kernel.stdout.log"), { flags: "a" })
      const stderr = createWriteStream(path.join(root, "kernel.stderr.log"), { flags: "a" })
      const daemonEnv = {
        ...process.env,
        ...(sliceModeProviderEnv ? {
          ...sliceModeProviderEnv,
          ARROBA_LOG_DIR: path.join(root, "logs"),
        } : {}),
        ARROBA_KERNEL_PORT: String(ports.kernelPort),
        ARROBA_MCP_PORT: String(ports.mcpPort),
        ARROBA_OPENCODE_PORT: String(ports.openCodePort),
        ARROBA_CODEX_PORT: String(ports.codexPort),
        ARROBA_DAEMON_ID: `provider-thread-transfer-${runId}`,
        ARROBA_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        ARROBA_SESSION_HISTORY_DIR: historyDir,
        ARROBA_CAPABILITY_ISOLATION_ROOT: capabilityRoot,
        ARROBA_PROVIDER_RUNTIME_INIT_DELAY_MS: "250",
      }
      if (sliceMode) {
        delete daemonEnv.ARROBA_RELAY_URL
        delete daemonEnv.ARROBA_RELAY_TOKEN
        delete daemonEnv.ARROBA_CLOUD_RELAY_URL
        delete daemonEnv.ARROBA_CLOUD_RELAY_TOKEN
      }
      daemonChild = spawn(kernelBinary, [], {
        cwd: repoRoot,
        env: daemonEnv,
        stdio: ["ignore", "pipe", "pipe"],
      })
      daemonChild.stdout?.pipe(stdout)
      daemonChild.stderr?.pipe(stderr)
      await waitForLocalDaemon(kernelUrl, root, root)
    }

    const runScenario = options.drill === "slice-restart"
      ? runSliceRestartScenario
      : options.drill === "live-migrate-to-slice" || options.drill === "live-migrate-roundtrip-slice"
        ? runLiveMigrateToSliceScenario
        : runLocalReloadScenario
    for (const provider of options.providers) {
      const result = await runScenario({ provider, root, kernelUrl, options })
      matrix.results.push(result)
      await writeFile(path.join(root, `${provider}-${options.drill}-result.json`), `${JSON.stringify(result, null, 2)}\n`, "utf8")
      console.log(`${provider}: ${result.status}`)
      if (result.status !== "passed") {
        console.log(result.errors.join("\n"))
      }
    }
  } finally {
    matrix.finished_at_ms = Date.now()
    matrix.passed = matrix.results.length > 0 && matrix.results.every((result) => result.status === "passed")
    await writeFile(path.join(root, "matrix.json"), `${JSON.stringify(matrix, null, 2)}\n`, "utf8")
    await terminateChild(daemonChild)
  }

  console.log(`provider thread transfer drill artifacts: ${root}`)
  if (!matrix.passed) {
    throw new Error(`provider thread transfer drill failed; see ${path.join(root, "matrix.json")}`)
  }
  if (options.cleanupOnSuccess) {
    await rm(root, { recursive: true, force: true })
  }
}

main().catch((error) => {
  console.error(error.stack ?? error.message ?? String(error))
  process.exit(1)
})
