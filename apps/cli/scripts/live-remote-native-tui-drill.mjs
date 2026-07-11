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
  waitForProviderRunMcpGrant,
} from "./lib/native-tui-capabilities.mjs"
import {
  copyHetznerDirectoryToLocal,
  ensureExecutionDirectory,
  prepareHetznerWorktree,
  remoteEnvCommand,
  removeExecutionFile,
  removeHetznerNativeRuntimePaths,
  removeHetznerWorktree,
  shellQuote,
  sshArgs,
  stopHetznerProcessByEnv,
  waitForExecutionFileContent,
} from "./lib/native-tui-remote-execution.mjs"
import {
  assertBinary,
  makeAvailablePorts,
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

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cliPath = path.join(cliRoot, "dist/index.js")
const kernelBinary = resolveBuiltBinarySync(
  path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel"),
  path.join(repoRoot, "apps/kernel/Cargo.toml"),
  "arroba-kernel",
)
const relayBinary = resolveBuiltBinarySync(
  path.join(repoRoot, "apps/relay/target/debug/arroba-relay"),
  path.join(repoRoot, "apps/relay/Cargo.toml"),
  "arroba-relay",
)
const defaultLocalDockerSliceImage = process.env.ARROBA_SLICE_DOCKER_IMAGE ?? "arroba-slice-linux:0.1.0"
const realHomeDir = os.homedir()
const tinyPng = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=", "base64")
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

function parseArgs(argv) {
  const options = {
    providers: ["opencode", "codex", "claude"],
    keepArtifactsOnFailure: false,
    homeManagedSliceLocalDocker: false,
    standardHomeWorker: false,
    hetznerWorker: false,
    hetznerHost: process.env.ARROBA_NATIVE_TUI_HETZNER_HOST ?? "root@195.201.123.115",
    hetznerRelayHost: process.env.ARROBA_NATIVE_TUI_HETZNER_RELAY_HOST ?? "195.201.123.115",
    hetznerKey: process.env.ARROBA_NATIVE_TUI_HETZNER_KEY ?? path.join(os.homedir(), ".ssh/arroba_hetzner_staging"),
    hetznerRepo: process.env.ARROBA_NATIVE_TUI_HETZNER_REPO ?? "/tmp/arroba-native-remote-validate",
    includePermissions: false,
    includeAttachments: false,
    includeMcpSkills: false,
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
  return options
}

function printHelp() {
  console.log([
    "Usage: node apps/cli/scripts/live-remote-native-tui-drill.mjs [options]",
    "",
    "Runs relay-attached native TUI drills for provider-native CLI mode:",
    "- starts an isolated relay and home kernel",
    "- launches two native TUIs through --relay-url into one Arroba session",
    "- opens an Arroba CLI observer through the same relay",
    "- verifies native-origin and Arroba-origin prompts, no cross-contamination, and badge transitions",
    "",
    "  --providers opencode,codex,claude",
    "  --standard-home-worker     Run home and worker kernels through the relay",
    "  --hetzner-worker           Run relay and worker kernel on the configured Hetzner host",
    "  --hetzner-host HOST        SSH host for --hetzner-worker (default root@195.201.123.115)",
    "  --hetzner-relay-host HOST  Relay host clients connect to for --hetzner-worker",
    "  --hetzner-key PATH         SSH key for --hetzner-worker",
    "  --hetzner-repo PATH        Remote Arroba checkout for --hetzner-worker",
    "  --home-managed-slice-local-docker  Run native TUIs through the home kernel into a managed local Docker slice",
    "  --include-permissions         Validate provider-native permissions through the Arroba observer",
    "  --include-attachments         Validate prompt attachment transfer through native TUI providers",
    "  --include-mcp-skills          Validate pre-granted MCP/skill propagation for native TUI providers",
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

async function syncHetznerWorkerKernelConfig(options, root, remoteRuntimeRoot) {
  const localConfigPath = path.join(root, "hetzner-worker-config.toml")
  const remoteConfigDir = path.posix.join(remoteRuntimeRoot, "xdg-config", "arroba")
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

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    printHelp()
    return
  }
  const runId = `${process.pid}-${Date.now()}`
  const root = path.join("/tmp", `arb-remote-native-tui-${runId}`)
  const ports = await makeAvailablePorts()
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
  const sliceBuildImagePolicy = process.env.ARROBA_NATIVE_TUI_SLICE_BUILD_IMAGE ?? "always"
  const rustMinStack = process.env.RUST_MIN_STACK ?? "16777216"
  let relay = null
  let relayTunnel = null
  let kernel = null
  let workerKernel = null
  let hetznerWorktreePrepared = false
  const managedSlices = []
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(root)
    await assertBinary(kernelBinary, path.join(repoRoot, "apps/kernel/Cargo.toml"), "arroba-kernel")
    await assertBinary(relayBinary, path.join(repoRoot, "apps/relay/Cargo.toml"), "arroba-relay")
    await mkdir(homeDir, { recursive: true })
    await mkdir(xdgConfigHome, { recursive: true })
    await mkdir(xdgStateHome, { recursive: true })
    await mkdir(xdgDataHome, { recursive: true })
    await mkdir(xdgCacheHome, { recursive: true })
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
      await syncHetznerWorkerKernelConfig(options, root, remoteRuntimeRoot)
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
        ARROBA_REMOTE_REPO: options.hetznerRepo,
        ARROBA_RELAY_HOST: "127.0.0.1",
        ARROBA_RELAY_PORT: String(ports.relayPort),
        ARROBA_RELAY_TOKEN: relayToken,
        RUST_MIN_STACK: rustMinStack,
      }, "./apps/relay/target/debug/arroba-relay")), {
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
      relay = spawn(relayBinary, [], {
        cwd: repoRoot,
        env: {
          ...process.env,
          ARROBA_RELAY_HOST: "127.0.0.1",
          ARROBA_RELAY_PORT: String(ports.relayPort),
          ARROBA_RELAY_TOKEN: relayToken,
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
        CODEX_HOME: process.env.CODEX_HOME ?? path.join(realHomeDir, ".codex"),
        OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(realHomeDir, ".config", "opencode"),
        ARROBA_LOG_DIR: path.join(root, "logs"),
        ARROBA_KERNEL_PORT: String(ports.kernelPort),
        ARROBA_MCP_PORT: String(ports.mcpPort),
        ARROBA_OPENCODE_PORT: String(ports.openCodePort),
        ARROBA_CODEX_PORT: String(ports.codexPort),
        ARROBA_RELAY_URL: relayUrl,
        ARROBA_RELAY_TOKEN: relayToken,
        ARROBA_DAEMON_ID: homeDaemonId,
        ARROBA_DAEMON_ALIAS: targetDaemonAlias,
        ARROBA_MACHINE_ID: `remote-native-machine-${process.pid}`,
        ARROBA_MACHINE_ALIAS: targetDaemonAlias,
        ARROBA_ACCEPT_REMOTE_LEASES: "0",
        ARROBA_DAEMON_SOCKET: path.join(root, "home.sock"),
        ARROBA_SESSION_HISTORY_DIR: path.join(root, "history"),
        ARROBA_CAPABILITY_ISOLATION_ROOT: homeCapabilityRoot,
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
          ARROBA_REMOTE_REPO: options.hetznerRepo,
          RUST_MIN_STACK: rustMinStack,
          PATH: `/root/.bun/bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`,
          HOME: "/root",
          XDG_CONFIG_HOME: path.posix.join(remoteRuntimeRoot, "xdg-config"),
          XDG_STATE_HOME: path.posix.join(remoteRuntimeRoot, "xdg-state"),
          XDG_DATA_HOME: path.posix.join(remoteRuntimeRoot, "xdg-data"),
          XDG_CACHE_HOME: path.posix.join(remoteRuntimeRoot, "xdg-cache"),
          CODEX_HOME: "/root/.codex",
          OPENCODE_CONFIG_DIR: "/root/.config/opencode",
          ARROBA_LOG_DIR: path.posix.join(remoteRuntimeRoot, "worker-logs"),
          ARROBA_KERNEL_PORT: String(ports.workerKernelPort),
          ARROBA_MCP_PORT: String(ports.workerMcpPort),
          ARROBA_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
          ARROBA_RELAY_TOKEN: relayToken,
          ARROBA_DAEMON_ID: workerDaemonId,
          ARROBA_DAEMON_ALIAS: workerDaemonAlias,
          ARROBA_MACHINE_ID: workerMachineAlias,
          ARROBA_MACHINE_ALIAS: workerMachineAlias,
          ARROBA_ACCEPT_REMOTE_LEASES: "1",
          ARROBA_DAEMON_SOCKET: path.posix.join(remoteRuntimeRoot, "worker.sock"),
          ARROBA_SESSION_HISTORY_DIR: path.posix.join(remoteRuntimeRoot, "worker-history"),
          ARROBA_CAPABILITY_ISOLATION_ROOT: workerCapabilityRoot,
        }, `mkdir -p ${shellQuote(remoteRuntimeParent)} && ./apps/kernel/target/debug/arroba-kernel`)), {
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
            CODEX_HOME: process.env.CODEX_HOME ?? path.join(realHomeDir, ".codex"),
            OPENCODE_CONFIG_DIR: process.env.OPENCODE_CONFIG_DIR ?? path.join(realHomeDir, ".config", "opencode"),
            ARROBA_LOG_DIR: path.join(root, "worker-logs"),
            ARROBA_KERNEL_PORT: String(ports.workerKernelPort),
            ARROBA_MCP_PORT: String(ports.workerMcpPort),
            ARROBA_RELAY_URL: relayUrl,
            ARROBA_RELAY_TOKEN: relayToken,
            ARROBA_DAEMON_ID: workerDaemonId,
            ARROBA_DAEMON_ALIAS: workerDaemonAlias,
            ARROBA_MACHINE_ID: workerMachineAlias,
            ARROBA_MACHINE_ALIAS: workerMachineAlias,
            ARROBA_ACCEPT_REMOTE_LEASES: "1",
            ARROBA_DAEMON_SOCKET: path.join(root, "worker.sock"),
            ARROBA_SESSION_HISTORY_DIR: path.join(root, "worker-history"),
            ARROBA_CAPABILITY_ISOLATION_ROOT: workerCapabilityRoot,
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
            ARROBA_NATIVE_PROVIDER_ENDPOINT_SSH_HOST: options.hetznerHost,
            ARROBA_NATIVE_PROVIDER_ENDPOINT_SSH_KEY: options.hetznerKey,
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
    if (options.hetznerWorker) {
      await stopHetznerProcessByEnv(options, {
        ARROBA_DAEMON_ID: workerDaemonId,
        ARROBA_RELAY_TOKEN: relayToken,
      })
      await stopHetznerProcessByEnv(options, {
        ARROBA_RELAY_PORT: String(ports.relayPort),
        ARROBA_RELAY_TOKEN: relayToken,
      })
      if (preserveFailedRun && remoteRuntimeRoot) {
        await copyHetznerDirectoryToLocal(
          options,
          path.posix.join(remoteRuntimeRoot, "worker-logs"),
          path.join(root, "remote-worker-logs"),
        ).catch((error) => {
          console.error(`Hetzner worker log collection failed: ${error.message}`)
        })
      }
      await removeHetznerNativeRuntimePaths(options, [remoteRuntimeParent, remoteRuntimeRoot])
      if (hetznerWorktreePrepared) {
        await removeHetznerWorktree(options, worktree)
      }
    }
    await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded,
      preserveOnFailure: options.keepArtifactsOnFailure,
      failure,
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
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
