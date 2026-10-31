import {linkManagedSliceProviderProfile} from "./lib/managed-slice-provider-profile.mjs"
import {requireScopedProviderPath,withPrivateDrillRuntime} from "./lib/private-drill-runtime.mjs"
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
  runProviderScenario,
  waitForLocalDaemon,
  waitForRelayTarget,
  waitForRemoteMachine,
} from "./lib/live-remote-native-tui-drill-scenario.mjs"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSliceRequest,
  createSessionRequest,
  deleteSliceRequest,
  endSessionRequest,
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
  getSessionStateRequest,
  importSliceProviderAuthRequest,
  listAgentsRequest,
  listRemoteMachinesRequest,
  pumpTerminalOutputRequest,
  setUserConfigValueRequest,
  startSliceRequest,
} from "../dist/ipc-requests.js"
import {
  cleanupNativeDrillCapabilities,
  installNativeDrillCapabilities,
  waitForProviderRunMcpAdmission,
} from "./lib/native-tui-capabilities.mjs"
import {
  assertRemoteKernelTcpPortAvailable,
  copyRemoteKernelDirectoryToLocal,
  ensureExecutionDirectory,
  remoteKernelNativeRuntimeTempDir,
  prepareRemoteKernelClaudeWorkspaceTrust,
  prepareRemoteKernelWorktree,
  spawnRemoteEnv,
  removeExecutionFile,
  removeRemoteKernelNativeRuntimePaths,
  removeRemoteKernelWorktree,
  restoreRemoteKernelClaudeWorkspaceTrust,
  seedRemoteKernelOpenCodeRuntimeProfile,
  seedLocalOpenCodeRuntimeProfile,
  shellQuote,
  sshArgs,
  stopRemoteKernelProcessByEnv,
  stopRemoteKernelRuntimeBeforeClaudeTrustRestore,
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
import {
  runNativeCodexPrompt,
  runNativeOpenCodePrompt,
  runNativeOpenCodePromptDetached,
  sendClaudeRenderedPromptViaKernelInput,
} from "./lib/native-tui-provider-drivers.mjs"
import {
  CLAUDE_UNATTENDED_CREDENTIALS_GUIDANCE,
} from "./lib/live-provider-thread-transfer-runtime.mjs"
import { applyProviderModelOverride } from "./lib/drill-provider-profiles.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cliPath = path.join(cliRoot, "dist/index.js")
const kernelBinary = resolveBuiltBinarySync(
  path.join(repoRoot, "apps/kernel/target/debug/chariox-kernel"),
  path.join(repoRoot, "apps/kernel/Cargo.toml"),
  "chariox-kernel",
)
const relayBinary = resolveBuiltBinarySync(
  path.join(repoRoot, "apps/relay/target/debug/chariox-relay"),
  path.join(repoRoot, "apps/relay/Cargo.toml"),
  "chariox-relay",
)
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

async function disableWorkspaceLiveSync(kernelUrl, localAuthEnvironment) {
  if (!kernelUrl) return
  const client = new LocalIpcClient(kernelUrl, {
    localAuthEnvironment,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  try {
    await client.send(setUserConfigValueRequest("providers.workspace_live_sync", "off"))
  } finally {
    await client.close().catch(() => {})
  }
}

async function remoteKernelNativePortsAreAvailable(options, ports) {
  for (const [label, port] of [
    ["relay", ports.relayPort],
    ["worker kernel", ports.workerKernelPort],
    ["worker MCP", ports.workerMcpPort],
  ]) {
    try {
      await assertRemoteKernelTcpPortAvailable(options, port, `remote kernel ${label} port`)
    } catch (error) {
      if (error instanceof Error && /is already in use by pid\(s\)/.test(error.message)) {
        return false
      }
      throw error
    }
  }
  return true
}

function parseArgs(argv) {
  const options = {
    providers: ["opencode", "codex", "claude"],
    keepArtifactsOnFailure: false,
    homeManagedSliceLocalDocker: false,
    standardHomeWorker: false,
    remoteKernelWorker: false,
    remoteKernelHost: process.env.CHARIOX_NATIVE_TUI_REMOTE_KERNEL_HOST ?? "",
    remoteKernelRelayHost: process.env.CHARIOX_NATIVE_TUI_REMOTE_KERNEL_RELAY_HOST ?? "",
    remoteKernelKey: process.env.CHARIOX_NATIVE_TUI_REMOTE_KERNEL_KEY ?? "",
    remoteKernelRepo: process.env.CHARIOX_NATIVE_TUI_REMOTE_KERNEL_REPO ?? "/tmp/chariox-native-remote-validate",
    includePermissions: false,
    includeAttachments: false,
    includeMcpSkills: false,
    providerModels: {},
    codexEffort: "high",
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
    } else if (arg === "--remote-kernel-worker") {
      options.remoteKernelWorker = true
      options.standardHomeWorker = true
    } else if (arg === "--remote-kernel-host") {
      options.remoteKernelHost = argv[++index]
    } else if (arg === "--remote-kernel-relay-host") {
      options.remoteKernelRelayHost = argv[++index]
    } else if (arg === "--remote-kernel-key") {
      options.remoteKernelKey = argv[++index]
    } else if (arg === "--remote-kernel-repo") {
      options.remoteKernelRepo = argv[++index]
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
  if (options.remoteKernelWorker && !options.standardHomeWorker) {
    throw new Error("--remote-kernel-worker requires --standard-home-worker")
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
    "  --remote-kernel-worker           Run relay and worker kernel on the configured remote kernel host",
    "  --remote-kernel-host HOST        SSH host for --remote-kernel-worker",
    "  --remote-kernel-relay-host HOST  Relay host clients connect to for --remote-kernel-worker",
    "  --remote-kernel-key PATH         SSH key for --remote-kernel-worker",
    "  --remote-kernel-repo PATH        Remote Chariox checkout for --remote-kernel-worker",
    "  --home-managed-slice-local-docker  Run native TUIs through the home kernel into a managed local Docker slice",
    "  --include-permissions         Validate provider-native permissions through the Chariox observer",
    "  --include-attachments         Validate prompt attachment transfer through native TUI providers",
    "  --include-mcp-skills          Validate pre-granted MCP/skill propagation for native TUI providers",
    "  --provider-model P=M          Override the exact model for a provider",
    "  --codex-effort E              Override Codex reasoning effort (default high)",
    "  --keep-artifacts-on-failure",
  ].join("\n"))
}


async function syncRemoteKernelCodexAuth(options) {
  const workerProfile=requireScopedProviderPath(process.env,"CHARIOX_DRILL_WORKER_CODEX_HOME")
  await execFileAsync("ssh",sshArgs(options,`test -f ${shellQuote(path.posix.join(workerProfile,"auth.json"))}`))
}

async function syncRemoteKernelClaudeAuth() {
  throw new Error(CLAUDE_UNATTENDED_CREDENTIALS_GUIDANCE)
}

async function syncRemoteKernelWorkerKernelConfig(options, root, remoteRuntimeRoot) {
  const localConfigPath = path.join(root, "remote-kernel-worker-config.toml")
  const remoteConfigDir = path.posix.join(remoteRuntimeRoot, "xdg-config", "chariox")
  await writeFile(
    localConfigPath,
    isolatedKernelConfigToml(path.posix.join(remoteRuntimeRoot, "worker-kernel-storage")),
    { mode: 0o600 },
  )
  await execFileAsync("ssh", sshArgs(options, `mkdir -p ${shellQuote(remoteConfigDir)}`))
  await execFileAsync("scp", [
    "-i",
    options.remoteKernelKey,
    "-o",
    "BatchMode=yes",
    "-o",
    "StrictHostKeyChecking=accept-new",
    localConfigPath,
    `${options.remoteKernelHost}:${path.posix.join(remoteConfigDir, "config.toml")}`,
  ])
}

async function createHomeManagedLocalDockerSlice({ homeKernelUrl, workspace, providers, localAuthEnvironment }) {
  const client = new LocalIpcClient(homeKernelUrl, {
    localAuthEnvironment,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
  let ownedSliceId = null
  try {
    const providerProfiles = new Map()
    for (const provider of providers) providerProfiles.set(provider, await linkManagedSliceProviderProfile(client, provider, process.env))
    const name = `native-tui-${process.pid}`
    const created = unwrap(await client.send(createSliceRequest({
      name,
      backend: "local_docker",
      os: "linux",
      workspaceMount: workspace,
    })), "SliceCreated").slice
    ownedSliceId = created.id
    const started = unwrap(await client.send(startSliceRequest(created.id)), "SliceStarted").slice
    for (const provider of providers) {
      const profile = providerProfiles.get(provider)
      unwrap(await client.send(importSliceProviderAuthRequest(started.id, provider, profile.profile_id)), "SliceProviderAuthImported")
    }
    if (!started.worker_kernel_id) {
      throw new Error(`started managed slice ${started.id} did not discover its worker kernel`)
    }
    if (!started.worker_kernel_ref) {
      throw new Error(`started managed slice ${started.id} did not expose its worker kernel reference`)
    }
    ownedSliceId = null
    return started
  } catch {
    if (ownedSliceId) {
      try { await client.send(deleteSliceRequest(ownedSliceId)) }
      catch { throw new Error(`managed slice setup failed; cleanup failed for ${ownedSliceId}`) }
    }
    throw new Error("managed slice setup failed; owned slice cleaned")
  } finally {
    await client.close().catch(() => {})
  }
}

async function deleteHomeManagedSlice(homeKernelUrl, sliceRef, localAuthEnvironment) {
  if (!sliceRef) return
  const client = new LocalIpcClient(homeKernelUrl, {
    localAuthEnvironment,
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

async function runDrill(privateRoot) {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const runId = `${process.pid}-${Date.now()}`
  const root = privateRoot
  const evidenceRoot=path.join(os.homedir(),".codex","evidence","native-tui",runId)
  const ports = await makeAvailablePorts({
    candidateFactory: options.remoteKernelWorker ? makeNonEphemeralDrillPorts : undefined,
    additionalAvailability: options.remoteKernelWorker
      ? (candidate) => remoteKernelNativePortsAreAvailable(options, candidate)
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
  const remoteRuntimeRoot = options.remoteKernelWorker
    ? `/tmp/arb-remote-native-tui-${runId}`
    : null
  const remoteClaudeTrustStatePath = remoteRuntimeRoot
    ? path.posix.join(remoteRuntimeRoot, "claude-workspace-trust.json")
    : null
  const remoteTempDir = remoteRuntimeRoot
    ? remoteKernelNativeRuntimeTempDir(remoteRuntimeRoot)
    : null
  const workerKernelUrl = options.remoteKernelWorker ? null : `ws://127.0.0.1:${ports.workerKernelPort}`
  const workspace = repoRoot
  const worktree = repoRoot
  const homeDir = path.join(root, "home")
  const xdgConfigHome = path.join(root, "xdg-config")
  const xdgStateHome = path.join(root, "xdg-state")
  const homeLocalAuthEnvironment = { ...process.env, XDG_STATE_HOME: xdgStateHome }
  const workerLocalAuthEnvironment = { ...process.env, XDG_STATE_HOME: path.join(root, "worker-xdg-state") }
  const xdgDataHome = path.join(root, "xdg-data")
  const xdgCacheHome = path.join(root, "xdg-cache")
  const homeCapabilityRoot = path.join(root, "home-capabilities")
  const workerCapabilityRoot = options.remoteKernelWorker
    ? path.posix.join(remoteRuntimeRoot, "worker-capabilities")
    : path.join(root, "worker-capabilities")
  const sliceBuildImagePolicy = process.env.CHARIOX_NATIVE_TUI_SLICE_BUILD_IMAGE ?? "always"
  const rustMinStack = process.env.RUST_MIN_STACK ?? "16777216"
  let relay = null
  let relayTunnel = null
  let kernel = null
  let workerKernel = null
  let remoteKernelWorktreePrepared = false
  let remoteKernelClaudeTrustPrepared = false
  let remoteKernelClaudeTrustRestoreFailure = null
  const localOpenCodeCredentialPaths = []
  const managedSlices = []
  let succeeded = false
  let failure = null
  try {
    await mkdir(root, { recursive: true, mode: 0o700 })
    await assertBinary(kernelBinary, path.join(repoRoot, "apps/kernel/Cargo.toml"), "chariox-kernel")
    await assertBinary(relayBinary, path.join(repoRoot, "apps/relay/Cargo.toml"), "chariox-relay")
    await mkdir(homeDir, { recursive: true })
    await mkdir(xdgConfigHome, { recursive: true })
    await mkdir(xdgStateHome, { recursive: true })
    await mkdir(xdgDataHome, { recursive: true })
    await mkdir(xdgCacheHome, { recursive: true })
    if (options.providers.includes("opencode") && !options.homeManagedSliceLocalDocker) {
      const sourceXdgDataHome = process.env.XDG_DATA_HOME?.trim()
      const sourceOpenCodeDataHome = process.env.OPENCODE_DATA_HOME?.trim()
        || path.join(requireScopedProviderPath(process.env,"XDG_DATA_HOME"), "opencode")
      const sourceXdgCacheHome = requireScopedProviderPath(process.env,"XDG_CACHE_HOME")
      const homeCredentialPath = await seedLocalOpenCodeRuntimeProfile({
        sourceDataHome: sourceOpenCodeDataHome,
        sourceCacheHome: path.join(sourceXdgCacheHome, "opencode"),
        destinationXdgDataHome: xdgDataHome,
        destinationXdgCacheHome: xdgCacheHome,
      })
      if (homeCredentialPath) localOpenCodeCredentialPaths.push(homeCredentialPath)
      if (options.standardHomeWorker && !options.remoteKernelWorker) {
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
    if (options.standardHomeWorker && !options.remoteKernelWorker) {
      await writeIsolatedKernelConfig({
        xdgConfigHome: path.join(root, "worker-xdg-config"),
        storageRoot: path.join(root, "worker-kernel-storage"),
      })
    }
    if (options.homeManagedSliceLocalDocker) {
      await prebuildLocalDockerSliceImageIfNeeded(sliceBuildImagePolicy)
    }
    if (options.remoteKernelWorker) {
      await prepareRemoteKernelWorktree(options, worktree)
      remoteKernelWorktreePrepared = true
      if (options.providers.includes("claude")) {
        await syncRemoteKernelClaudeAuth(options, root)
        await prepareRemoteKernelClaudeWorkspaceTrust(options, worktree, remoteClaudeTrustStatePath)
        remoteKernelClaudeTrustPrepared = true
      }
      await syncRemoteKernelWorkerKernelConfig(options, root, remoteRuntimeRoot)
      await ensureExecutionDirectory(options, true, remoteTempDir)
      if (options.providers.includes("opencode")) {
        await seedRemoteKernelOpenCodeRuntimeProfile(options, remoteRuntimeRoot)
      }
      if (options.providers.includes("codex")) {
        await syncRemoteKernelCodexAuth(options)
      }
    }
    if (options.remoteKernelWorker) {
      relay = spawnRemoteEnv(options, {
        CHARIOX_REMOTE_REPO: options.remoteKernelRepo,
        CHARIOX_RELAY_HOST: "127.0.0.1",
        CHARIOX_RELAY_PORT: String(ports.relayPort),
        CHARIOX_RELAY_TOKEN: relayToken,
        RUST_MIN_STACK: rustMinStack,
      }, "./apps/relay/target/debug/chariox-relay", {
        stdio: ["ignore", "ignore", "inherit"],
      })
      relayTunnel = spawn("ssh", [
        "-i",
        options.remoteKernelKey,
        "-o",
        "BatchMode=yes",
        "-o",
        "StrictHostKeyChecking=accept-new",
        "-N",
        "-L",
        `127.0.0.1:${ports.relayPort}:127.0.0.1:${ports.relayPort}`,
        options.remoteKernelHost,
      ], {
        stdio: ["ignore", "ignore", "inherit"],
      })
      await waitForTcpPort(ports.relayPort, "127.0.0.1", 30_000)
    } else {
      relay = spawn(relayBinary, [], {
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
    kernel = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        HOME: realHomeDir,
        XDG_CONFIG_HOME: xdgConfigHome,
        XDG_STATE_HOME: xdgStateHome,
        XDG_DATA_HOME: xdgDataHome,
        XDG_CACHE_HOME: xdgCacheHome,
        ...(options.providers.includes("codex")?{CODEX_HOME: requireScopedProviderPath(process.env,"CODEX_HOME")} : {}),
        OPENCODE_CONFIG_DIR: options.providers.includes("opencode") ? requireScopedProviderPath(process.env,"OPENCODE_CONFIG_DIR") : undefined,
        CHARIOX_HOME: path.join(root,"kernel-home"),
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
    await waitForLocalDaemon(homeKernelUrl, workspace, worktree, homeLocalAuthEnvironment)
    await disableWorkspaceLiveSync(homeKernelUrl, homeLocalAuthEnvironment)
    await waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias)
    if (options.standardHomeWorker) {
      if (options.remoteKernelWorker) {
        workerKernel = spawnRemoteEnv(options, {
          CHARIOX_REMOTE_REPO: options.remoteKernelRepo,
          RUST_MIN_STACK: rustMinStack,
          PATH: `/root/.bun/bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`,
          HOME: "/root",
          XDG_CONFIG_HOME: path.posix.join(remoteRuntimeRoot, "xdg-config"),
          XDG_STATE_HOME: path.posix.join(remoteRuntimeRoot, "xdg-state"),
          XDG_DATA_HOME: path.posix.join(remoteRuntimeRoot, "xdg-data"),
          XDG_CACHE_HOME: path.posix.join(remoteRuntimeRoot, "xdg-cache"),
          ...(options.providers.includes("codex")?{CODEX_HOME: requireScopedProviderPath(process.env,"CHARIOX_DRILL_WORKER_CODEX_HOME")} : {}),
          OPENCODE_CONFIG_DIR: options.providers.includes("opencode") ? requireScopedProviderPath(process.env,"CHARIOX_DRILL_WORKER_OPENCODE_CONFIG_DIR") : undefined,
          OPENCODE_DATA_HOME: options.providers.includes("opencode") ? requireScopedProviderPath(process.env,"CHARIOX_DRILL_WORKER_OPENCODE_DATA_HOME") : undefined,
          CHARIOX_HOME: path.posix.join(remoteRuntimeRoot,"kernel-home"),
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
        }, `mkdir -p ${shellQuote(remoteRuntimeParent)} && ./apps/kernel/target/debug/chariox-kernel`, {
          stdio: ["ignore", "ignore", "inherit"],
        })
      } else {
        workerKernel = spawn(kernelBinary, [], {
          cwd: repoRoot,
          env: {
            ...process.env,
            HOME: realHomeDir,
            XDG_CONFIG_HOME: path.join(root, "worker-xdg-config"),
            XDG_STATE_HOME: path.join(root, "worker-xdg-state"),
            XDG_DATA_HOME: path.join(root, "worker-xdg-data"),
            XDG_CACHE_HOME: path.join(root, "worker-xdg-cache"),
            ...(options.providers.includes("codex")?{CODEX_HOME: requireScopedProviderPath(process.env,"CODEX_HOME")} : {}),
            OPENCODE_CONFIG_DIR: options.providers.includes("opencode") ? requireScopedProviderPath(process.env,"OPENCODE_CONFIG_DIR") : undefined,
            CHARIOX_LOG_DIR: path.join(root, "worker-logs"),
            CHARIOX_HOME: path.join(root,"worker-kernel-home"),
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
        await waitForLocalDaemon(workerKernelUrl, workspace, worktree, workerLocalAuthEnvironment)
        await disableWorkspaceLiveSync(workerKernelUrl, workerLocalAuthEnvironment)
      }
      await waitForRelayTarget(relayUrl, relayToken, workerDaemonAlias)
      await waitForRemoteMachine(relayUrl, relayToken, targetDaemonAlias, workerMachineAlias)
    }

    const scenarios = []
    for (const provider of options.providers) {
      let providerSlice = null
      if (options.homeManagedSliceLocalDocker) {
        providerSlice = await createHomeManagedLocalDockerSlice({
          localAuthEnvironment: homeLocalAuthEnvironment,
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
        options: { ...options, workerLocalAuthEnvironment },
        nativeEnv: options.remoteKernelWorker
          ? {
            HOME: realHomeDir,
            ...(options.providers.includes("codex")?{CODEX_HOME: requireScopedProviderPath(process.env,"CODEX_HOME")} : {}),
            CHARIOX_NATIVE_PROVIDER_ENDPOINT_SSH_HOST: options.remoteKernelHost,
            CHARIOX_NATIVE_PROVIDER_ENDPOINT_SSH_KEY: options.remoteKernelKey,
          }
          : {},
      }))
      if (providerSlice) {
        await deleteHomeManagedSlice(homeKernelUrl, providerSlice.id, homeLocalAuthEnvironment).catch((error) => {
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
      await deleteHomeManagedSlice(homeKernelUrl, slice.id, homeLocalAuthEnvironment).catch((error) => {
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
    if (options.remoteKernelWorker) {
      await stopRemoteKernelRuntimeBeforeClaudeTrustRestore({
        stopWorker: () => stopRemoteKernelProcessByEnv(options, {
          CHARIOX_DAEMON_ID: workerDaemonId,
          CHARIOX_RELAY_TOKEN: relayToken,
        }),
        stopRelay: () => stopRemoteKernelProcessByEnv(options, {
          CHARIOX_RELAY_PORT: String(ports.relayPort),
          CHARIOX_RELAY_TOKEN: relayToken,
        }),
        restoreTrust: remoteKernelClaudeTrustPrepared
          ? () => restoreRemoteKernelClaudeWorkspaceTrust(options, worktree, remoteClaudeTrustStatePath)
          : null,
      }).then(() => {
        remoteKernelClaudeTrustPrepared = false
      }).catch((error) => {
        remoteKernelClaudeTrustRestoreFailure = error
        console.error(`remote kernel Claude workspace trust restoration failed: ${error.message}`)
      })
      if (remoteRuntimeRoot && options.providers.includes("opencode")) {
        await removeExecutionFile(
          options,
          true,
          path.posix.join(remoteRuntimeRoot, "xdg-data", "opencode", "auth.json"),
        ).catch(() => {})
      }
      if (preserveFailedRun && remoteRuntimeRoot) {
        await copyRemoteKernelDirectoryToLocal(
          options,
          path.posix.join(remoteRuntimeRoot, "worker-logs"),
          path.join(root, "remote-worker-logs"),
        ).catch((error) => {
          console.error(`remote kernel worker log collection failed: ${error.message}`)
        })
      }
      const removableRuntimePaths = [remoteRuntimeParent]
      if (!remoteKernelClaudeTrustRestoreFailure) {
        removableRuntimePaths.push(remoteRuntimeRoot)
      } else {
        console.error(`remote kernel Claude workspace trust restoration state kept at ${remoteClaudeTrustStatePath}`)
      }
      await removeRemoteKernelNativeRuntimePaths(options, removableRuntimePaths)
      if (remoteKernelWorktreePrepared) {
        await removeRemoteKernelWorktree(options, worktree)
      }
    }
    const finalFailure = remoteKernelClaudeTrustRestoreFailure
      ? new AggregateError(
        failure && failure !== remoteKernelClaudeTrustRestoreFailure
          ? [failure, remoteKernelClaudeTrustRestoreFailure]
          : [remoteKernelClaudeTrustRestoreFailure],
        `remote kernel Claude workspace trust restoration failed; state remains at ${remoteClaudeTrustStatePath}`,
      )
      : failure
    await finalizeDrillArtifacts({
      rootDir: evidenceRoot,
      passed: succeeded && !remoteKernelClaudeTrustRestoreFailure,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure: finalFailure,
      metadata: {
        drill: "remote-native-tui",
        providers: options.providers.join(","),
        standardHomeWorker: options.standardHomeWorker,
        remoteKernelWorker: options.remoteKernelWorker,
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
    if (remoteKernelClaudeTrustRestoreFailure) {
      throw finalFailure
    }
  }
}

function main() {return withPrivateDrillRuntime("native-tui-runtime",runDrill)}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
