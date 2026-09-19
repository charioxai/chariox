import { execFile, spawn } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { access, mkdir, readFile, rm, symlink, writeFile } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { setTimeout as sleep } from "node:timers/promises"
import os from "node:os"
import { promisify } from "node:util"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import {
  isolatedKernelConfigToml,
  writeIsolatedKernelConfig,
} from "./lib/drill-kernel-storage.mjs"
import {
  assertHetznerTcpPortAvailable,
  copyHetznerDirectoryToLocal,
  ensureExecutionDirectory,
  hetznerNativeRuntimeTempDir,
  prepareHetznerClaudeWorkspaceTrust,
  prepareHetznerWorktree,
  remoteEnvCommand,
  removeExecutionFile,
  removeHetznerNativeRuntimePaths,
  removeHetznerWorktree,
  restoreHetznerClaudeWorkspaceTrust,
  seedHetznerOpenCodeRuntimeProfile,
  seedLocalOpenCodeRuntimeProfile,
  shellQuote,
  sshArgs,
  stopHetznerProcessByEnv,
  stopHetznerRuntimeBeforeClaudeTrustRestore,
  waitForExecutionFileContent,
} from "./lib/native-tui-remote-execution.mjs"
import {
  assertBinary,
  makeAvailablePorts,
  makeNonEphemeralDrillPorts,
  resolveBuiltBinarySync,
  resolveCommandPath,
  runLogged,
  screenQuit,
  screenStuff,
  startScreen,
  terminateChild,
  waitForFileMatch,
  waitForLogOccurrences,
  waitForTcpPort,
} from "./lib/drill-runtime-helpers.mjs"
import { applyProviderModelOverride } from "./lib/drill-provider-profiles.mjs"
import {
  PATH1_RUNNER_CONTEXT_SCHEMA,
  buildExistingRoomCellInvocation,
  buildStrictChildEnvironment,
  parseOptionalRunnerContext,
  runExistingRoomCell,
  validateExistingRoomContext,
} from "../../../scripts/path1-oss-runner-context-adapter.mjs"

let remoteScenarioApi = null
let ipcApi = null
async function loadIpcApi() {
  if (!ipcApi) {
    const [ipc, requests] = await Promise.all([
      import("../dist/ipc.js"),
      import("../dist/ipc-requests.js"),
    ])
    ipcApi = { ...ipc, ...requests }
  }
  return ipcApi
}

async function loadRemoteScenarioApi() {
  if (!remoteScenarioApi) {
    const [scenario, capabilities] = await Promise.all([
      import("./lib/live-remote-native-tui-drill-scenario.mjs"),
      import("./lib/native-tui-capabilities.mjs"),
    ])
    remoteScenarioApi = { ...scenario, ...capabilities }
  }
  return remoteScenarioApi
}

const CLAUDE_UNATTENDED_CREDENTIALS_GUIDANCE = "Claude unattended credentials are deliberately unsupported; use the official managed provider account path."

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cliPath = path.join(cliRoot, "dist/index.js")
let kernelBinary = null
let relayBinary = null
function getKernelBinary() {
  kernelBinary ??= resolveBuiltBinarySync(
    path.join(repoRoot, "apps/kernel/target/debug/chariox-kernel"),
    path.join(repoRoot, "apps/kernel/Cargo.toml"),
    "chariox-kernel",
  )
  return kernelBinary
}
function getRelayBinary() {
  relayBinary ??= resolveBuiltBinarySync(
    path.join(repoRoot, "apps/relay/target/debug/chariox-relay"),
    path.join(repoRoot, "apps/relay/Cargo.toml"),
    "chariox-relay",
  )
  return relayBinary
}
const defaultLocalDockerSliceImage = process.env.CHARIOX_SLICE_DOCKER_IMAGE ?? "chariox-slice-linux:0.1.0"
const realHomeDir = os.homedir()
const execFileAsync = promisify(execFile)

function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

function unwrapVariant(response, variant) {
  return unwrap(response, variant)
}

async function disableWorkspaceLiveSync(kernelUrl) {
  const { LocalIpcClient, setUserConfigValueRequest } = await loadIpcApi()
  if (!kernelUrl) return
  const client = new LocalIpcClient(kernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  try {
    await client.send(setUserConfigValueRequest("providers.workspace_live_sync", "off"))
  } finally {
    await client.close().catch(() => {})
  }
}

async function hetznerNativePortsAreAvailable(options, ports) {
  for (const [label, port] of [
    ["relay", ports.relayPort],
    ["worker kernel", ports.workerKernelPort],
    ["worker MCP", ports.workerMcpPort],
  ]) {
    try {
      await assertHetznerTcpPortAvailable(options, port, `Hetzner ${label} port`)
    } catch (error) {
      if (error instanceof Error && /is already in use by pid\(s\)/.test(error.message)) {
        return false
      }
      throw error
    }
  }
  return true
}

export function readPath1RunnerContext(argv = process.argv.slice(2), environment = process.env) {
  if (!argv.includes("--path1-runner-context")) return null
  const raw = environment.CHARIOX_PATH1_RUNNER_CONTEXT_JSON
  if (!raw) throw new Error("--path1-runner-context requires CHARIOX_PATH1_RUNNER_CONTEXT_JSON")
  const context = validateExistingRoomContext(parseOptionalRunnerContext(raw))
  if (context.schema !== PATH1_RUNNER_CONTEXT_SCHEMA) throw new Error("Path 1 runner context schema is unsupported")
  return context
}

export function parseArgs(argv) {
  const options = {
    providers: ["opencode", "codex", "claude"],
    keepArtifactsOnFailure: false,
    homeManagedSliceLocalDocker: false,
    standardHomeWorker: false,
    hetznerWorker: false,
    hetznerHost: process.env.CHARIOX_NATIVE_TUI_HETZNER_HOST ?? "root@195.201.123.115",
    hetznerRelayHost: process.env.CHARIOX_NATIVE_TUI_HETZNER_RELAY_HOST ?? "195.201.123.115",
    hetznerKey: process.env.CHARIOX_NATIVE_TUI_HETZNER_KEY ?? path.join(os.homedir(), ".ssh/chariox_hetzner_staging"),
    hetznerRepo: process.env.CHARIOX_NATIVE_TUI_HETZNER_REPO ?? "/tmp/chariox-native-remote-validate",
    includePermissions: false,
    includeAttachments: false,
    includeMcpSkills: false,
    providerModels: {},
    codexEffort: "high",
    runnerContext: null,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") {
      continue
    } else if (arg === "--providers") {
      options.providers = argv[++index].split(",").map((provider) => provider.trim()).filter(Boolean)
    } else if (arg === "--keep-artifacts-on-failure") {
      options.keepArtifactsOnFailure = true
    } else if (arg === "--home-managed-slice-local-docker") {
      options.homeManagedSliceLocalDocker = true
    } else if (arg === "--standard-home-worker") {
      options.standardHomeWorker = true
    } else if (arg === "--hetzner-worker") {
      options.hetznerWorker = true
      options.standardHomeWorker = true
    } else if (arg === "--hetzner-host") {
      options.hetznerHost = argv[++index]
    } else if (arg === "--hetzner-relay-host") {
      options.hetznerRelayHost = argv[++index]
    } else if (arg === "--hetzner-key") {
      options.hetznerKey = argv[++index]
    } else if (arg === "--hetzner-repo") {
      options.hetznerRepo = argv[++index]
    } else if (arg === "--include-permissions") {
      options.includePermissions = true
    } else if (arg === "--include-attachments") {
      options.includeAttachments = true
    } else if (arg === "--include-mcp-skills") {
      options.includeMcpSkills = true
    } else if (arg === "--provider-model") {
      applyProviderModelOverride(options.providerModels, argv[++index])
    } else if (arg === "--codex-effort") {
      options.codexEffort = argv[++index]
    } else if (arg === "--path1-runner-context") {
      options.runnerContext = readPath1RunnerContext(argv)
    } else if (arg === "--help" || arg === "-h") {
      options.help = true
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  const placementModes = [options.homeManagedSliceLocalDocker, options.standardHomeWorker]
    .filter(Boolean)
    .length
  if (placementModes > 1) {
    throw new Error("--home-managed-slice-local-docker and --standard-home-worker are mutually exclusive")
  }
  if (options.hetznerWorker && !options.standardHomeWorker) {
    throw new Error("--hetzner-worker requires --standard-home-worker")
  }
  for (const provider of options.providers) {
    if (provider !== "opencode" && provider !== "codex" && provider !== "claude") {
      throw new Error(`unsupported provider ${provider}; expected opencode, codex, or claude`)
    }
  }
  for (const provider of Object.keys(options.providerModels)) {
    if (provider !== "opencode" && provider !== "codex" && provider !== "claude") {
      throw new Error(`unsupported provider model override ${provider}; expected opencode, codex, or claude`)
    }
  }
  if (!options.codexEffort?.trim()) {
    throw new Error("--codex-effort requires a value")
  }
  return options
}

export function buildPath1RemoteCellInvocation(context, provider) {
  const normalized = validateExistingRoomContext(context, { expected: { provider } })
  return buildExistingRoomCellInvocation({
    context: normalized,
    surface: "remoteNativeTui",
    provider,
    role: "remote-native-tui",
  })
}

export async function runPath1RemoteCell({ context, provider, processRunner, clientFactory } = {}) {
  return runExistingRoomCell({
    context: validateExistingRoomContext(context, { expected: { provider } }),
    surface: "remoteNativeTui",
    provider,
    role: "remote-native-tui",
    processRunner,
    clientFactory,
  })
}

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/live-remote-native-tui-drill.mjs [options]",
    "",
    "Runs relay-attached native TUI drills for provider-native CLI mode:",
    "- starts an isolated relay and home kernel",
    "- launches two native TUIs through --relay-url into one Chariox session",
    "- opens a Chariox CLI observer through the same relay",
    "- verifies native-origin and Chariox-origin prompts, no cross-contamination, and badge transitions",
    "",
    "  --providers opencode,codex,claude",
    "  --standard-home-worker     Run home and worker kernels through the relay",
    "  --hetzner-worker           Run relay and worker kernel on the configured Hetzner host",
    "  --hetzner-host HOST        SSH host for --hetzner-worker (default root@195.201.123.115)",
    "  --hetzner-relay-host HOST  Relay host clients connect to for --hetzner-worker",
    "  --hetzner-key PATH         SSH key for --hetzner-worker",
    "  --hetzner-repo PATH        Remote Chariox checkout for --hetzner-worker",
    "  --home-managed-slice-local-docker  Run native TUIs through the home kernel into a managed local Docker slice",
    "  --include-permissions         Validate provider-native permissions through the Chariox observer",
    "  --include-attachments         Validate prompt attachment transfer through native TUI providers",
    "  --include-mcp-skills          Validate pre-granted MCP/skill propagation for native TUI providers",
    "  --provider-model P=M          Override the exact model for a provider",
    "  --codex-effort E              Override Codex reasoning effort (default high)",
    "  --keep-artifacts-on-failure",
  ].join("\n"))
}


function localCodexAuthPath() {
  const codexHome = process.env.CODEX_HOME?.trim() || path.join(realHomeDir, ".codex")
  return path.join(codexHome, "auth.json")
}

async function syncHetznerCodexAuth(options) {
  const authPath = localCodexAuthPath()
  await access(authPath)
  await execFileAsync("ssh", sshArgs(options, "mkdir -p /root/.codex && chmod 700 /root/.codex"))
  await execFileAsync("scp", [
    "-i",
    options.hetznerKey,
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=accept-new",
    authPath,
    `${options.hetznerHost}:/root/.codex/auth.json.tmp`,
  ])
  await execFileAsync("ssh", sshArgs(options, "mv /root/.codex/auth.json.tmp /root/.codex/auth.json && chmod 600 /root/.codex/auth.json"))
}

async function syncHetznerClaudeAuth() {
  throw new Error(CLAUDE_UNATTENDED_CREDENTIALS_GUIDANCE)
}

async function syncHetznerWorkerKernelConfig(options, root, remoteRuntimeRoot) {
  const localConfigPath = path.join(root, "hetzner-worker-config.toml")
  const remoteConfigDir = path.posix.join(remoteRuntimeRoot, "xdg-config", "chariox")
  await writeFile(
    localConfigPath,
    isolatedKernelConfigToml(path.posix.join(remoteRuntimeRoot, "worker-kernel-storage")),
    { mode: 0o600 },
  )
  await execFileAsync("ssh", sshArgs(options, `mkdir -p ${shellQuote(remoteConfigDir)}`))
  await execFileAsync("scp", [
    "-i",
    options.hetznerKey,
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=accept-new",
    localConfigPath,
    `${options.hetznerHost}:${path.posix.join(remoteConfigDir, "config.toml")}`,
  ])
}

async function createHomeManagedLocalDockerSlice({ homeKernelUrl, workspace, providers }) {
  const { LocalIpcClient, createSliceRequest, startSliceRequest, importSliceProviderAuthRequest } = await loadIpcApi()
  const client = new LocalIpcClient(homeKernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  try {
    const name = `native-tui-${process.pid}`
    const created = unwrap(await client.send(createSliceRequest({
      name,
      backend: "local_docker",
      os: "linux",
      workspaceMount: workspace,
    })), "SliceCreated").slice
    const started = unwrap(await client.send(startSliceRequest(created.id)), "SliceStarted").slice
    for (const provider of providers) {
      await client.send(importSliceProviderAuthRequest(started.id, provider))
    }
    if (!started.worker_kernel_id) {
      throw new Error(`started managed slice ${started.id} did not discover its worker kernel`)
    }
    if (!started.worker_kernel_ref) {
      throw new Error(`started managed slice ${started.id} did not expose its worker kernel reference`)
    }
    return started
  } finally {
    await client.close().catch(() => {})
  }
}

async function deleteHomeManagedSlice(homeKernelUrl, sliceRef) {
  const { LocalIpcClient, deleteSliceRequest } = await loadIpcApi()
  if (!sliceRef) return
  const client = new LocalIpcClient(homeKernelUrl, {
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  try {
    await client.send(deleteSliceRequest(sliceRef))
  } finally {
    await client.close().catch(() => {})
  }
}

async function prebuildLocalDockerSliceImageIfNeeded(policy) {
  if (policy !== "always") return
  await runLogged("docker", [
    "build",
    "-f",
    path.join(repoRoot, "apps/kernel/slice-linux-docker/docker/Dockerfile"),
    "-t",
    defaultLocalDockerSliceImage,
    repoRoot,
  ])
}

async function dismissCodexUpdatePromptIfPresent(screenName, logFile) {
  const deadline = Date.now() + 5_000
  while (Date.now() < deadline) {
    const text = await readFile(logFile, "utf8").catch(() => "")
    if (/Update available!/.test(text) && /Skip/.test(text)) {
      await screenStuff(screenName, "2\r")
      await sleep(500)
      return true
    }
    await sleep(250)
  }
  return false
}

async function runPath1RemoteProvider(provider, context, options) {
  const normalized = validateExistingRoomContext(context, { expected: { provider: null } })
  const root = path.join(normalized.evidenceRoot ?? path.join(os.tmpdir(), `chariox-path1-remote-${normalized.runId}`), `remote-${provider}`)
  const logs = {
    firstDir: path.join(root, "native-a-screen"),
    secondDir: path.join(root, "native-b-screen"),
    first: path.join(root, "native-a-screen", "screenlog.0"),
    second: path.join(root, "native-b-screen", "screenlog.0"),
  }
  await mkdir(logs.firstDir, { recursive: true })
  await mkdir(logs.secondDir, { recursive: true })
  const firstScreen = `chariox-path1-remote-${provider}-a-${process.pid}`
  const secondScreen = `chariox-path1-remote-${provider}-b-${process.pid}`
  const firstAlias = `${provider}-path1-a`
  const secondAlias = `${provider}-path1-b`
  const environment = buildStrictChildEnvironment(normalized, process.env)
  const providerArgs = provider === "codex"
    ? ["--model", "gpt-5.4-mini", "--effort", options.codexEffort ?? "high"]
    : provider === "claude"
      ? ["--model", "sonnet", "--effort", "low", "--remote-rendered"]
      : []
  const commonArgs = [
    cliPath,
    provider,
    normalized.sessionId,
    "--relay-url",
    normalized.relayEndpoint,
    "--target-daemon-id",
    normalized.homeKernelId,
    "--workspace",
    normalized.repoRoot,
    "--worktree",
    normalized.repoRoot,
    ...providerArgs,
  ]
  let succeeded = false
  let observerClient = null
  try {
    await startScreen(firstScreen, logs.firstDir, "bun", [
      ...commonArgs,
      "--alias",
      `${provider}-path1-session`,
      "--agent-alias",
      firstAlias,
    ], environment)
    await startScreen(secondScreen, logs.secondDir, "bun", [
      ...commonArgs,
      "--agent-alias",
      secondAlias,
    ], environment)
    const firstSession = (await waitForFileMatch(logs.first, /chariox session:\s+([^\s(]+)/)).match[1]
    const secondSession = (await waitForFileMatch(logs.second, /chariox session:\s+([^\s(]+)/)).match[1]
    if (firstSession !== normalized.sessionId || secondSession !== normalized.sessionId) {
      throw new Error("Path 1 remote native TUI changed the supplied session")
    }
    const { LocalIpcClient, attachToSessionRequest } = await loadIpcApi()
    observerClient = new LocalIpcClient(normalized.relayEndpoint, {
      targetDaemonId: normalized.homeKernelId,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    const attached = unwrap(
      await observerClient.send(attachToSessionRequest(normalized.sessionId, `${normalized.runId}-path1-remote-observer`)),
      "SessionAttached",
    )
    const attachedSessionId = attached.session?.id ?? attached.session_id ?? normalized.sessionId
    if (attachedSessionId !== normalized.sessionId) throw new Error("Path 1 remote observer changed the supplied session")
    succeeded = true
    return {
      schema: "chariox.path1.runner-context-result.v1",
      source: "deployed-oss-live-drill",
      liveObserved: true,
      dryRun: false,
      sourceTestOnly: false,
      action: "capability-drills",
      runId: normalized.runId,
      sourceHead: normalized.sourceHead,
      sessionId: normalized.sessionId,
      roomId: normalized.roomId,
      provider,
      official: true,
      kernelAuthoritative: true,
      relayTransportOnly: true,
      surfaces: ["remoteTui", "computer"],
      capabilities: {
        remoteTui: { attached: true, clients: 2, sameRoom: true, sameSession: true },
      },
      receiptId: `${normalized.runId}-${provider}-remote-native`,
    }
  } finally {
    await observerClient?.close?.().catch(() => {})
    await screenQuit(firstScreen)
    await screenQuit(secondScreen)
    if (!succeeded) {
      // Logs stay in the runner-owned evidence root for failure diagnosis.
    }
  }
}

async function runPath1RemoteCapability(context, options) {
  const providers = options.providers.length > 0 ? options.providers : ["opencode", "codex", "claude"]
  const cells = []
  for (const provider of providers) cells.push(await runPath1RemoteProvider(provider, context, options))
  return {
    schema: "chariox.path1.runner-context-result.v1",
    source: "deployed-oss-live-drill",
    liveObserved: true,
    dryRun: false,
    sourceTestOnly: false,
    action: "capability-drills",
    runId: context.runId,
    sourceHead: context.sourceHead,
    sessionId: context.sessionId,
    roomId: context.roomId,
    official: true,
    kernelAuthoritative: true,
    relayTransportOnly: true,
    capabilities: {
      remoteTui: { attached: true, providers: cells.map((cell) => cell.provider), sameRoom: true, sameSession: true },
    },
    receiptId: `${context.runId}-remote-native-capability`,
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  if (options.runnerContext) {
    const result = await runPath1RemoteCapability(options.runnerContext, options)
    console.log(JSON.stringify(result, null, 2))
    console.log(`CHARIOX_PATH1_RESULT:${JSON.stringify(result)}`)
    return
  }
  const {
    runProviderScenario,
    waitForLocalDaemon,
    waitForRelayTarget,
    waitForRemoteMachine,
  } = await loadRemoteScenarioApi()
  const runId = `${process.pid}-${Date.now()}`
  const root = path.join("/tmp", `arb-remote-native-tui-${runId}`)
  const ports = await makeAvailablePorts({
    candidateFactory: options.hetznerWorker ? makeNonEphemeralDrillPorts : undefined,
    additionalAvailability: options.hetznerWorker
      ? (candidate) => hetznerNativePortsAreAvailable(options, candidate)
      : undefined,
  })
  const relayToken = `remote-native-token-${process.pid}`
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const homeKernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const targetDaemonAlias = `remote-native-home-${process.pid}`
  const workerDaemonAlias = `remote-native-worker-${process.pid}`
  const workerMachineAlias = `remote-native-worker-machine-${process.pid}`
  const homeDaemonId = `remote-native-home-${runId}`
  const workerDaemonId = `remote-native-worker-${runId}`
  const remoteRuntimeParent = `/tmp/arb-remote-native-tui-${process.pid}`
  const remoteRuntimeRoot = options.hetznerWorker
    ? `/tmp/arb-remote-native-tui-${runId}`
    : null
  const remoteClaudeTrustStatePath = remoteRuntimeRoot
    ? path.posix.join(remoteRuntimeRoot, "claude-workspace-trust.json")
    : null
  const remoteTempDir = remoteRuntimeRoot
    ? hetznerNativeRuntimeTempDir(remoteRuntimeRoot)
    : null
  const workerKernelUrl = options.hetznerWorker ? null : `ws://127.0.0.1:${ports.workerKernelPort}`
  const workspace = repoRoot
  const worktree = repoRoot
  const homeDir = path.join(root, "home")
  const xdgConfigHome = path.join(root, "xdg-config")
  const xdgStateHome = path.join(root, "xdg-state")
  const xdgDataHome = path.join(root, "xdg-data")
  const xdgCacheHome = path.join(root, "xdg-cache")
  const homeCapabilityRoot = path.join(root, "home-capabilities")
  const workerCapabilityRoot = options.hetznerWorker
    ? path.posix.join(remoteRuntimeRoot, "worker-capabilities")
    : path.join(root, "worker-capabilities")
  const sliceBuildImagePolicy = process.env.CHARIOX_NATIVE_TUI_SLICE_BUILD_IMAGE ?? "always"
  const rustMinStack = process.env.RUST_MIN_STACK ?? "16777216"
  let relay = null
  let relayTunnel = null
  let kernel = null
  let workerKernel = null
  let hetznerWorktreePrepared = false
  let hetznerClaudeTrustPrepared = false
  let hetznerClaudeTrustRestoreFailure = null
  const localOpenCodeCredentialPaths = []
  const managedSlices = []
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(root)
    await assertBinary(getKernelBinary(), path.join(repoRoot, "apps/kernel/Cargo.toml"), "chariox-kernel")
    await assertBinary(getRelayBinary(), path.join(repoRoot, "apps/relay/Cargo.toml"), "chariox-relay")
    await mkdir(homeDir, { recursive: true })
    await mkdir(xdgConfigHome, { recursive: true })
    await mkdir(xdgStateHome, { recursive: true })
    await mkdir(xdgDataHome, { recursive: true })
    await mkdir(xdgCacheHome, { recursive: true })
    if (options.providers.includes("opencode")) {
      const sourceXdgDataHome = process.env.XDG_DATA_HOME?.trim()
        || path.join(realHomeDir, ".local", "share")
      const sourceOpenCodeDataHome = process.env.OPENCODE_DATA_HOME?.trim()
        || path.join(sourceXdgDataHome, "opencode")
      const sourceXdgCacheHome = process.env.XDG_CACHE_HOME?.trim()
        || path.join(realHomeDir, ".cache")
      const homeCredentialPath = await seedLocalOpenCodeRuntimeProfile({
        sourceDataHome: sourceOpenCodeDataHome,
        sourceCacheHome: path.join(sourceXdgCacheHome, "opencode"),
        destinationXdgDataHome: xdgDataHome,
        destinationXdgCacheHome: xdgCacheHome,
      })
      if (homeCredentialPath) localOpenCodeCredentialPaths.push(homeCredentialPath)
      if (options.standardHomeWorker && !options.hetznerWorker) {
        const workerCredentialPath = await seedLocalOpenCodeRuntimeProfile({
          sourceDataHome: sourceOpenCodeDataHome,
          sourceCacheHome: path.join(sourceXdgCacheHome, "opencode"),
          destinationXdgDataHome: path.join(root, "worker-xdg-data"),
          destinationXdgCacheHome: path.join(root, "worker-xdg-cache"),
        })
        if (workerCredentialPath) localOpenCodeCredentialPaths.push(workerCredentialPath)
      }
    }
    await writeIsolatedKernelConfig({
      xdgConfigHome,
      storageRoot: path.join(root, "home-kernel-storage"),
      extraToml: options.homeManagedSliceLocalDocker ? [
        "[slices]",
        `root = ${JSON.stringify(path.join(root, "slices"))}`,
        "",
        "[slices.linux]",
        `docker_image = ${JSON.stringify(defaultLocalDockerSliceImage)}`,
        `build_image = ${JSON.stringify(sliceBuildImagePolicy === "always" ? "auto" : sliceBuildImagePolicy)}`,
      ] : [],
    })
    if (options.standardHomeWorker && !options.hetznerWorker) {
      await writeIsolatedKernelConfig({
        xdgConfigHome: path.join(root, "worker-xdg-config"),
        storageRoot: path.join(root, "worker-kernel-storage"),
      })
    }
    if (options.homeManagedSliceLocalDocker) {
      await prebuildLocalDockerSliceImageIfNeeded(sliceBuildImagePolicy)
    }
    if (options.hetznerWorker) {
      await prepareHetznerWorktree(options, worktree)
      hetznerWorktreePrepared = true
      if (options.providers.includes("claude")) {
        await syncHetznerClaudeAuth(options, root)
        await prepareHetznerClaudeWorkspaceTrust(options, worktree, remoteClaudeTrustStatePath)
        hetznerClaudeTrustPrepared = true
      }
      await syncHetznerWorkerKernelConfig(options, root, remoteRuntimeRoot)
      await ensureExecutionDirectory(options, true, remoteTempDir)
      if (options.providers.includes("opencode")) {
        await seedHetznerOpenCodeRuntimeProfile(options, remoteRuntimeRoot)
      }
      if (options.providers.includes("codex")) {
        await syncHetznerCodexAuth(options)
      }
    }
    await access(path.join(realHomeDir, ".claude"))
      .then(() => symlink(path.join(realHomeDir, ".claude"), path.join(homeDir, ".claude"), "dir"))
      .catch(() => {})
    await access(path.join(realHomeDir, ".claude.json"))
      .then(() => symlink(path.join(realHomeDir, ".claude.json"), path.join(homeDir, ".claude.json")))
      .catch(() => {})
    await access(path.join(realHomeDir, ".codex"))
      .then(() => symlink(path.join(realHomeDir, ".codex"), path.join(homeDir, ".codex"), "dir"))
      .catch(() => {})
    if (options.hetznerWorker) {
      relay = spawn("ssh", sshArgs(options, remoteEnvCommand({
        CHARIOX_REMOTE_REPO: options.hetznerRepo,
        CHARIOX_RELAY_HOST: "127.0.0.1",
        CHARIOX_RELAY_PORT: String(ports.relayPort),
        CHARIOX_RELAY_TOKEN: relayToken,
        RUST_MIN_STACK: rustMinStack,
      }, "./apps/relay/target/debug/chariox-relay")), {
        stdio: ["ignore", "ignore", "inherit"],
      })
      relayTunnel = spawn("ssh", [
        "-i",
        options.hetznerKey,
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-N",
        "-L",
        `127.0.0.1:${ports.relayPort}:127.0.0.1:${ports.relayPort}`,
        options.hetznerHost,
      ], {
        stdio: ["ignore", "ignore", "inherit"],
      })
      await waitForTcpPort(ports.relayPort, "127.0.0.1", 30_000)
    } else {
      relay = spawn(getRelayBinary(), [], {
        cwd: repoRoot,
        env: {
          ...process.env,
          CHARIOX_RELAY_HOST: "127.0.0.1",
          CHARIOX_RELAY_PORT: String(ports.relayPort),
          CHARIOX_RELAY_TOKEN: relayToken,
          RUST_MIN_STACK: rustMinStack,
        },
        stdio: ["ignore", "ignore", "inherit"],
      })
      await waitForTcpPort(ports.relayPort)
    }
    kernel = spawn(getKernelBinary(), [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        HOME: realHomeDir,
        XDG_CONFIG_HOME: xdgConfigHome,
        XDG_STATE_HOME: xdgStateHome,
        XDG_DATA_HOME: xdgDataHome,
        XDG_CACHE_HOME: xdgCacheHome,
        CODEX_HOME: process.env.CODEX_HOME ?? path.join(realHomeDir, ".codex"),
        OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(realHomeDir, ".config", "opencode"),
        CHARIOX_LOG_DIR: path.join(root, "logs"),
        CHARIOX_KERNEL_PORT: String(ports.kernelPort),
        CHARIOX_MCP_PORT: String(ports.mcpPort),
        CHARIOX_OPENCODE_PORT: String(ports.openCodePort),
        CHARIOX_CODEX_PORT: String(ports.codexPort),
        CHARIOX_RELAY_URL: relayUrl,
        CHARIOX_RELAY_TOKEN: relayToken,
        CHARIOX_DAEMON_ID: homeDaemonId,
        CHARIOX_DAEMON_ALIAS: targetDaemonAlias,
        CHARIOX_MACHINE_ID: `remote-native-machine-${process.pid}`,
        CHARIOX_MACHINE_ALIAS: targetDaemonAlias,
        CHARIOX_ACCEPT_REMOTE_LEASES: "0",
        CHARIOX_DAEMON_SOCKET: path.join(root, "home.sock"),
        CHARIOX_SESSION_HISTORY_DIR: path.join(root, "history"),
        CHARIOX_CAPABILITY_ISOLATION_ROOT: homeCapabilityRoot,
        RUST_MIN_STACK: rustMinStack,
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForLocalDaemon(homeKernelUrl, workspace, worktree)
    await disableWorkspaceLiveSync(homeKernelUrl)
    await waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias)
    if (options.standardHomeWorker) {
      if (options.hetznerWorker) {
        workerKernel = spawn("ssh", sshArgs(options, remoteEnvCommand({
          CHARIOX_REMOTE_REPO: options.hetznerRepo,
          RUST_MIN_STACK: rustMinStack,
          PATH: `/root/.bun/bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`,
          HOME: "/root",
          XDG_CONFIG_HOME: path.posix.join(remoteRuntimeRoot, "xdg-config"),
          XDG_STATE_HOME: path.posix.join(remoteRuntimeRoot, "xdg-state"),
          XDG_DATA_HOME: path.posix.join(remoteRuntimeRoot, "xdg-data"),
          XDG_CACHE_HOME: path.posix.join(remoteRuntimeRoot, "xdg-cache"),
          CODEX_HOME: "/root/.codex",
          OPENCODE_CONFIG_DIR: "/root/.config/opencode",
          CHARIOX_LOG_DIR: path.posix.join(remoteRuntimeRoot, "worker-logs"),
          CHARIOX_KERNEL_PORT: String(ports.workerKernelPort),
          CHARIOX_MCP_PORT: String(ports.workerMcpPort),
          CHARIOX_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
          CHARIOX_RELAY_TOKEN: relayToken,
          CHARIOX_DAEMON_ID: workerDaemonId,
          CHARIOX_DAEMON_ALIAS: workerDaemonAlias,
          CHARIOX_MACHINE_ID: workerMachineAlias,
          CHARIOX_MACHINE_ALIAS: workerMachineAlias,
          CHARIOX_ACCEPT_REMOTE_LEASES: "1",
          CHARIOX_DAEMON_SOCKET: path.posix.join(remoteRuntimeRoot, "worker.sock"),
          CHARIOX_SESSION_HISTORY_DIR: path.posix.join(remoteRuntimeRoot, "worker-history"),
          CHARIOX_CAPABILITY_ISOLATION_ROOT: workerCapabilityRoot,
          TMPDIR: remoteTempDir,
        }, `mkdir -p ${shellQuote(remoteRuntimeParent)} && ./apps/kernel/target/debug/chariox-kernel`)), {
          stdio: ["ignore", "ignore", "inherit"],
        })
      } else {
        workerKernel = spawn(getKernelBinary(), [], {
          cwd: repoRoot,
          env: {
            ...process.env,
            HOME: realHomeDir,
            XDG_CONFIG_HOME: path.join(root, "worker-xdg-config"),
            XDG_STATE_HOME: path.join(root, "worker-xdg-state"),
            XDG_DATA_HOME: path.join(root, "worker-xdg-data"),
            XDG_CACHE_HOME: path.join(root, "worker-xdg-cache"),
            CODEX_HOME: process.env.CODEX_HOME ?? path.join(realHomeDir, ".codex"),
            OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(realHomeDir, ".config", "opencode"),
            CHARIOX_LOG_DIR: path.join(root, "worker-logs"),
            CHARIOX_KERNEL_PORT: String(ports.workerKernelPort),
            CHARIOX_MCP_PORT: String(ports.workerMcpPort),
            CHARIOX_RELAY_URL: relayUrl,
            CHARIOX_RELAY_TOKEN: relayToken,
            CHARIOX_DAEMON_ID: workerDaemonId,
            CHARIOX_DAEMON_ALIAS: workerDaemonAlias,
            CHARIOX_MACHINE_ID: workerMachineAlias,
            CHARIOX_MACHINE_ALIAS: workerMachineAlias,
            CHARIOX_ACCEPT_REMOTE_LEASES: "1",
            CHARIOX_DAEMON_SOCKET: path.join(root, "worker.sock"),
            CHARIOX_SESSION_HISTORY_DIR: path.join(root, "worker-history"),
            CHARIOX_CAPABILITY_ISOLATION_ROOT: workerCapabilityRoot,
            RUST_MIN_STACK: rustMinStack,
          },
          stdio: ["ignore", "ignore", "inherit"],
        })
        await waitForLocalDaemon(workerKernelUrl, workspace, worktree)
        await disableWorkspaceLiveSync(workerKernelUrl)
      }
      await waitForRelayTarget(relayUrl, relayToken, workerDaemonAlias)
      await waitForRemoteMachine(relayUrl, relayToken, targetDaemonAlias, workerMachineAlias)
    }

    const scenarios = []
    for (const provider of options.providers) {
      let providerSlice = null
      if (options.homeManagedSliceLocalDocker) {
        providerSlice = await createHomeManagedLocalDockerSlice({
          homeKernelUrl,
          workspace,
          providers: [provider],
        })
        managedSlices.push(providerSlice)
      }
      scenarios.push(await runProviderScenario({
        provider,
        root,
        relayUrl,
        relayToken,
        targetDaemonAlias,
        workerDaemonAlias: options.standardHomeWorker ? workerDaemonAlias : null,
        workerKernelUrl,
        machineRef: options.standardHomeWorker ? workerMachineAlias : null,
        sliceRef: providerSlice ? providerSlice.id : null,
        workspace,
        worktree,
        options,
        nativeEnv: options.hetznerWorker
          ? {
            HOME: realHomeDir,
            CODEX_HOME: process.env.CODEX_HOME ?? path.join(realHomeDir, ".codex"),
            CHARIOX_NATIVE_PROVIDER_ENDPOINT_SSH_HOST: options.hetznerHost,
            CHARIOX_NATIVE_PROVIDER_ENDPOINT_SSH_KEY: options.hetznerKey,
          }
          : {},
      }))
      if (providerSlice) {
        await deleteHomeManagedSlice(homeKernelUrl, providerSlice.id).catch((error) => {
          console.error(`home-managed slice cleanup failed: ${error.message}`)
        })
        const index = managedSlices.findIndex((slice) => slice.id === providerSlice.id)
        if (index >= 0) managedSlices.splice(index, 1)
      }
    }

    console.log(JSON.stringify({
      status: "ok",
      mode: "remote-native-tui-relay-drill",
      relayUrl,
      homeKernelUrl,
      workerKernelUrl: options.standardHomeWorker ? workerKernelUrl : null,
      targetDaemonAlias,
      workerMachineAlias: options.standardHomeWorker ? workerMachineAlias : null,
      sliceRefs: scenarios.map((scenario) => scenario.sliceRef).filter(Boolean),
      providers: options.providers,
      scenarios,
    }, null, 2))
    succeeded = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    const preserveFailedRun = !succeeded && options.keepArtifactsOnFailure
    for (const slice of managedSlices.splice(0)) {
      await deleteHomeManagedSlice(homeKernelUrl, slice.id).catch((error) => {
        console.error(`home-managed slice cleanup failed: ${error.message}`)
      })
    }
    await terminateChild(workerKernel)
    await terminateChild(kernel)
    await terminateChild(relayTunnel)
    await terminateChild(relay)
    for (const credentialPath of localOpenCodeCredentialPaths) {
      await rm(credentialPath, { force: true })
    }
    if (options.hetznerWorker) {
      await stopHetznerRuntimeBeforeClaudeTrustRestore({
        stopWorker: () => stopHetznerProcessByEnv(options, {
          CHARIOX_DAEMON_ID: workerDaemonId,
          CHARIOX_RELAY_TOKEN: relayToken,
        }),
        stopRelay: () => stopHetznerProcessByEnv(options, {
          CHARIOX_RELAY_PORT: String(ports.relayPort),
          CHARIOX_RELAY_TOKEN: relayToken,
        }),
        restoreTrust: hetznerClaudeTrustPrepared
          ? () => restoreHetznerClaudeWorkspaceTrust(options, worktree, remoteClaudeTrustStatePath)
          : null,
      }).then(() => {
        hetznerClaudeTrustPrepared = false
      }).catch((error) => {
        hetznerClaudeTrustRestoreFailure = error
        console.error(`Hetzner Claude workspace trust restoration failed: ${error.message}`)
      })
      if (remoteRuntimeRoot && options.providers.includes("opencode")) {
        await removeExecutionFile(
          options,
          true,
          path.posix.join(remoteRuntimeRoot, "xdg-data", "opencode", "auth.json"),
        ).catch(() => {})
      }
      if (preserveFailedRun && remoteRuntimeRoot) {
        await copyHetznerDirectoryToLocal(
          options,
          path.posix.join(remoteRuntimeRoot, "worker-logs"),
          path.join(root, "remote-worker-logs"),
        ).catch((error) => {
          console.error(`Hetzner worker log collection failed: ${error.message}`)
        })
      }
      const removableRuntimePaths = [remoteRuntimeParent]
      if (!hetznerClaudeTrustRestoreFailure) {
        removableRuntimePaths.push(remoteRuntimeRoot)
      } else {
        console.error(`Hetzner Claude workspace trust restoration state kept at ${remoteClaudeTrustStatePath}`)
      }
      await removeHetznerNativeRuntimePaths(options, removableRuntimePaths)
      if (hetznerWorktreePrepared) {
        await removeHetznerWorktree(options, worktree)
      }
    }
    const finalFailure = hetznerClaudeTrustRestoreFailure
      ? new AggregateError(
        failure && failure !== hetznerClaudeTrustRestoreFailure
          ? [failure, hetznerClaudeTrustRestoreFailure]
          : [hetznerClaudeTrustRestoreFailure],
        `Hetzner Claude workspace trust restoration failed; state remains at ${remoteClaudeTrustStatePath}`,
      )
      : failure
    await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded && !hetznerClaudeTrustRestoreFailure,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure: finalFailure,
      metadata: {
        drill: "remote-native-tui",
        providers: options.providers.join(","),
        standardHomeWorker: options.standardHomeWorker,
        hetznerWorker: options.hetznerWorker,
        homeManagedSliceLocalDocker: options.homeManagedSliceLocalDocker,
        includePermissions: options.includePermissions,
        includeAttachments: options.includeAttachments,
        includeMcpSkills: options.includeMcpSkills,
        relayUrl,
        homeKernelUrl,
        workerKernelUrl,
        targetDaemonAlias,
        workerMachineAlias,
      },
      log: (name, details) => console.log(`[remote-native-tui-drill] ${name}`, JSON.stringify(details)),
    })
    if (preserveFailedRun) {
      console.error(`remote native TUI drill artifacts kept at ${root}`)
    }
    if (hetznerClaudeTrustRestoreFailure) {
      throw finalFailure
    }
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) {
  main().catch((error) => {
    if (process.argv.includes("--path1-runner-context")) {
      console.log(`CHARIOX_PATH1_RESULT:${JSON.stringify({
        schema: "chariox.path1.runner-context-result.v1",
        status: "failed",
        failure: { code: "official-cell-failed", reason: "remote native TUI cell failed" },
      })}`)
    } else console.error(error)
    process.exit(1)
  })
}
