import { spawn } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { access, mkdir, readFile, rm, symlink, writeFile } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { setTimeout as sleep } from "node:timers/promises"
import os from "node:os"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSliceRequest,
  createSessionRequest,
  deleteSliceRequest,
  endSessionRequest,
  getSessionHistoryRequest,
  getSessionStateRequest,
  importSliceProviderAuthRequest,
  listAgentsRequest,
  listRemoteMachinesRequest,
  pumpTerminalOutputRequest,
  startSliceRequest,
} from "../dist/ipc-requests.js"
import {
  cleanupNativeDrillCapabilities,
  installNativeDrillCapabilities,
  waitForProviderRunMcpGrant,
} from "./lib/native-tui-capabilities.mjs"
import {
  ensureExecutionDirectory,
  prepareHetznerWorktree,
  remoteEnvCommand,
  removeExecutionFile,
  shellQuote,
  sshArgs,
  waitForExecutionFileContent,
} from "./lib/native-tui-remote-execution.mjs"
import {
  assertBinary,
  makeAvailablePorts,
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
const kernelBinary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
const relayBinary = path.join(repoRoot, "apps/relay/target/debug/arroba-relay")
const realHomeDir = os.homedir()
const tinyPng = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=", "base64")

function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

function unwrapVariant(response, variant) {
  return unwrap(response, variant)
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

async function waitForLocalDaemon(kernelUrl, workspace, worktree) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(kernelUrl, {
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      const session = unwrap(await client.send(createSessionRequest(workspace, worktree)), "SessionCreated").session
      await client.send(endSessionRequest(session.id)).catch(() => {})
      await client.close()
      return
    } catch {
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error("home kernel did not become ready")
}

async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias, targetDaemonId = null) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias: targetDaemonId ? undefined : targetDaemonAlias,
      targetDaemonId: targetDaemonId ?? undefined,
      kernelPingIntervalMs: 60_000,
      kernelMaxMissedPongs: 10,
    })
    try {
      await Promise.race([
        client.send(listRemoteMachinesRequest()),
        sleep(2_000).then(() => {
          throw new Error("relay target probe timed out")
        }),
      ])
      await client.close().catch(() => {})
      return
    } catch {
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`relay target ${targetDaemonId ?? targetDaemonAlias} did not become reachable`)
}

async function waitForRemoteMachine(relayUrl, relayToken, targetDaemonAlias, machineAlias) {
  const client = relayClient(relayUrl, relayToken, targetDaemonAlias)
  try {
    for (let attempt = 0; attempt < 120; attempt += 1) {
      const machines = unwrap(await client.send(listRemoteMachinesRequest()), "RemoteMachinesListed").machines ?? []
      if (machines.some((machine) => machine.alias === machineAlias || machine.machine_alias === machineAlias || machine.id === machineAlias || machine.machine_id === machineAlias)) {
        return
      }
      await sleep(500)
    }
  } finally {
    await client.close().catch(() => {})
  }
  throw new Error(`remote machine ${machineAlias} did not appear in home kernel inventory`)
}

async function automationRequest(socketPath, request) {
  return await new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath)
    let buffer = ""
    socket.setTimeout(20_000)
    socket.once("error", reject)
    socket.once("timeout", () => reject(new Error(`automation request timed out: ${JSON.stringify(request)}`)))
    socket.on("data", (chunk) => {
      buffer += chunk.toString("utf8")
      const index = buffer.indexOf("\n")
      if (index < 0) return
      const line = buffer.slice(0, index)
      socket.end()
      const response = JSON.parse(line)
      if (!response.ok) reject(new Error(response.error ?? "automation request failed"))
      else resolve(response.data)
    })
    socket.once("connect", () => {
      socket.write(`${JSON.stringify({ id: Date.now(), ...request })}\n`)
    })
  })
}

async function fireAutomationRequest(socketPath, request) {
  await new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath)
    socket.setTimeout(5_000)
    socket.once("error", reject)
    socket.once("timeout", () => reject(new Error(`automation fire timed out: ${JSON.stringify(request)}`)))
    socket.once("connect", () => {
      socket.write(`${JSON.stringify({ id: Date.now(), ...request })}\n`, () => {
        socket.end()
        resolve()
      })
    })
  })
}

async function waitForAgents(client, sessionId, count) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const agents = unwrap(await client.send(listAgentsRequest(sessionId)), "AgentsListed").agents ?? []
    if (agents.length >= count) return agents
    await sleep(500)
  }
  throw new Error(`timed out waiting for ${count} agents`)
}

async function waitForNamedAgents(client, sessionId, aliases) {
  const deadline = Date.now() + 90_000
  while (Date.now() < deadline) {
    const agents = await waitForAgents(client, sessionId, aliases.length)
    const named = agents.filter((agent) => aliases.includes(agent.alias))
    if (new Set(named.map((agent) => agent.alias)).size === aliases.length) return named
    await sleep(500)
  }
  const agents = unwrap(await client.send(listAgentsRequest(sessionId)), "AgentsListed").agents ?? []
  throw new Error(`timed out waiting for agents ${aliases.join(", ")}; saw ${agents.map((agent) => agent.alias ?? agent.id).join(", ")}`)
}

async function waitForActiveProviderRun(client, sessionId) {
  const deadline = Date.now() + 90_000
  while (Date.now() < deadline) {
    const response = await client.send(getSessionStateRequest(sessionId))
    const session = (response.SessionState ?? response.SessionStateLoaded)?.session
    if (session?.active_provider_run_id) return session.active_provider_run_id
    await sleep(500)
  }
  throw new Error("timed out waiting for an active provider run")
}

async function waitForHistoryMarkers(client, sessionId, attachmentId, agents, expectedByAgent) {
  const deadline = Date.now() + 300_000
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    let ok = true
    const histories = {}
    for (const agent of agents) {
      const page = unwrap(await client.send(getSessionHistoryRequest(sessionId, 240, 100_000, null, agent.id)), "SessionHistory")
      const entries = page.entries.map((entry) => entry.entry).filter(Boolean)
      histories[agent.alias] = {
        all: entries.map((entry) => entry.text ?? "").join("\n"),
        prompts: entries.filter((entry) => entry.kind === "user_prompt").map((entry) => entry.text ?? "").join("\n"),
        outputs: entries.filter((entry) => entry.kind !== "user_prompt").map((entry) => entry.text ?? "").join(""),
      }
      const expected = expectedByAgent[agent.alias] ?? {}
      for (const marker of expected.prompts ?? []) ok &&= histories[agent.alias].prompts.includes(marker)
      for (const marker of expected.outputs ?? []) ok &&= histories[agent.alias].outputs.includes(marker)
    }
    if (ok) return histories
    await sleep(1_000)
  }
  throw new Error("timed out waiting for all history markers")
}

function badgeSnapshotForAlias(snapshot, alias) {
  return snapshot.session?.agents?.find((agent) => agent.alias === alias)?.badge ?? null
}

async function waitForAgentBadgeTone(socketPath, alias, tone, timeoutMs = 90_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const snapshot = await automationRequest(socketPath, { action: "snapshot" })
    const badge = badgeSnapshotForAlias(snapshot, alias)
    last = { alias, badge, agents: snapshot.session?.agents ?? [] }
    if (badge?.tone === tone) return badge
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${alias} badge tone ${tone}; last=${JSON.stringify(last)}`)
}

async function waitForInteraction(socketPath, alias, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const snapshot = await automationRequest(socketPath, { action: "snapshot" })
    last = snapshot
    const agent = snapshot.session?.agents?.find((entry) => entry.alias === alias)
    const interaction = snapshot.interactions?.find((entry) => entry.agentId === agent?.id && entry.kind === "permission")
    if (agent && interaction) return { snapshot, agent, interaction }
    await sleep(250)
  }
  throw new Error(`timed out waiting for permission interaction for ${alias}; last=${JSON.stringify(last)}`)
}

async function waitForInteractionFocused(socketPath, interactionId, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const snapshot = await automationRequest(socketPath, { action: "snapshot" })
    last = snapshot
    const interaction = snapshot.interactions?.find((entry) => entry.id === interactionId)
    if (interaction?.focused) return snapshot
    await sleep(250)
  }
  throw new Error(`timed out waiting for focused interaction ${interactionId}; last=${JSON.stringify(last)}`)
}

async function waitForInteractionCleared(socketPath, interactionId, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const snapshot = await automationRequest(socketPath, { action: "snapshot" })
    last = snapshot
    const interaction = snapshot.interactions?.find((entry) => entry.id === interactionId)
    if (!interaction) return snapshot
    await sleep(250)
  }
  throw new Error(`timed out waiting for interaction ${interactionId} to clear; last=${JSON.stringify(last)}`)
}

async function answerPermissionFromCli(socketPath, alias) {
  const pending = await waitForInteraction(socketPath, alias)
  if (!pending.interaction.focused) {
    await automationRequest(socketPath, {
      action: "workspace_shell_exec",
      command: `agent focus ${pending.agent.id}`,
    })
    await waitForInteractionFocused(socketPath, pending.interaction.id)
  }
  const allowIndex = pending.interaction.choices.findIndex((choice) =>
    choice.id === "allow_once"
      || choice.id === "allow"
      || /allow|yes|proceed/i.test(choice.label ?? "")
  )
  const response = await automationRequest(socketPath, {
    action: "interaction_submit",
    choiceIndex: allowIndex >= 0 ? allowIndex : 0,
  })
  const stillPending = response.interactions?.some((entry) => entry.id === pending.interaction.id)
  if (stillPending) {
    throw new Error(`permission interaction did not clear after submit: ${JSON.stringify(response.interactions)}`)
  }
  await waitForInteractionCleared(socketPath, pending.interaction.id)
  return pending.interaction
}

async function waitForClaudeProviderRunId(logFile) {
  return (await waitForFileMatch(logFile, /provider run:\s+([^\s]+)/, 90_000)).match[1]
}

async function waitForProviderToolCompletion(client, sessionId, attachmentId, agentId, matcher, timeoutMs = 240_000) {
  const deadline = Date.now() + timeoutMs
  let lastMatch = null
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const page = unwrap(await client.send(getSessionHistoryRequest(sessionId, 300, 100_000, null, agentId)), "SessionHistory")
    for (const row of page.entries) {
      const entry = row.entry
      if (!entry || entry.kind !== "provider_tool" || entry.agent_id !== agentId || typeof entry.text !== "string") continue
      let update = null
      try {
        update = JSON.parse(entry.text)
      } catch {
        continue
      }
      if (!matcher(update, entry.text)) continue
      lastMatch = update
      if (update.status === "completed") return update
    }
    await sleep(1_000)
  }
  throw new Error(`timed out waiting for provider tool completion; last=${JSON.stringify(lastMatch)}`)
}

async function waitForFileContent(filePath, expected, timeoutMs = 240_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const content = await readFile(filePath, "utf8").catch(() => "")
    if (content === expected) return content
    await sleep(500)
  }
  throw new Error(`timed out waiting for ${filePath} to contain ${JSON.stringify(expected)}`)
}

function permissionPrompt(markerText, filePath, content) {
  const shellCommand = `printf '${content}\\n' > ${filePath}`
  return `Use the shell to run \`${shellCommand}\`. After the command succeeds, reply with exactly ${markerText}.`
}

function claudePermissionPrompt(markerText, filePath, content) {
  return `Please create the file ${filePath} with exactly this content: ${content}. You can use Bash if convenient. After the file is written, reply with exactly ${markerText}.`
}

function attachedFilePrompt(markerText) {
  return `Read the attached file and reply with exactly ${markerText} and nothing else.`
}

function attachedImagePrompt(markerText) {
  return `Reply with exactly ${markerText} and nothing else after receiving the attached image.`
}

function relayClient(relayUrl, relayToken, targetDaemonAlias) {
  return new LocalIpcClient(relayUrl, {
    relayAuthToken: relayToken,
    targetDaemonAlias,
    kernelPingIntervalMs: 60_000,
    kernelMaxMissedPongs: 10,
  })
}

async function runProviderScenario({
  provider,
  root,
  relayUrl,
  relayToken,
  targetDaemonAlias,
  workerKernelUrl = null,
  machineRef = null,
  sliceRef = null,
  workspace,
  worktree,
  nativeEnv = {},
  options,
}) {
  const scenarioRoot = path.join(root, provider)
  const remotePlacement = Boolean(machineRef || sliceRef)
  const screenA = `arroba-rnt-${provider}-a-${process.pid}`
  const screenB = `arroba-rnt-${provider}-b-${process.pid}`
  const screenCli = `arroba-rnt-${provider}-cli-${process.pid}`
  const aliases = provider === "opencode"
    ? ["oc-remote-a", "oc-remote-b"]
    : provider === "codex"
      ? ["cdx-remote-a", "cdx-remote-b"]
      : ["cc-remote-a", "cc-remote-b"]
  const providerArgs = provider === "codex"
    ? ["--model", "gpt-5.4-mini", "--effort", "high", "--server-in-kernel"]
    : provider === "opencode"
      ? ["--server-in-kernel"]
    : []
  if (options.includePermissions) {
    providerArgs.push("--permissions", "required")
  }
  const marker = provider === "opencode" ? "OPENCODE" : provider === "codex" ? "CODEX" : "CLAUDE"
  const markers = {
    arrobaA: `${marker}ALPHA`,
    arrobaB: `${marker}BRAVO`,
    nativeA: `${marker}CHARLIE`,
    nativeB: `${marker}DELTA`,
    nativePermission: `${marker}NATIVEPERMISSION`,
    arrobaPermission: `${marker}ARROBAPERMISSION`,
    nativeAttachment: `${marker}NATIVEATTACHMENT`,
    arrobaAttachment: `${marker}ARROBAATTACHMENT`,
    nativeSkill: `${marker}NATIVESKILL`,
    arrobaSkill: `${marker}ARROBASKILL`,
  }
  const skipBaselineTurns = provider === "claude" && options.includePermissions
  const providerBinaryEnv = {}
  if (provider === "opencode" && !process.env.ARROBA_OPENCODE_BIN) {
    providerBinaryEnv.ARROBA_OPENCODE_BIN = await resolveCommandPath("opencode")
  } else if (provider === "codex" && !process.env.ARROBA_CODEX_BIN) {
    providerBinaryEnv.ARROBA_CODEX_BIN = await resolveCommandPath("codex")
  } else if (provider === "claude" && !process.env.ARROBA_CLAUDE_BIN) {
    providerBinaryEnv.ARROBA_CLAUDE_BIN = await resolveCommandPath("claude")
  }
  const logs = {
    aDir: path.join(scenarioRoot, "native-a-screen"),
    bDir: path.join(scenarioRoot, "native-b-screen"),
    cliDir: path.join(scenarioRoot, "arroba-cli-screen"),
    a: path.join(scenarioRoot, "native-a-screen", "screenlog.0"),
    b: path.join(scenarioRoot, "native-b-screen", "screenlog.0"),
    cli: path.join(scenarioRoot, "arroba-cli-screen", "screenlog.0"),
    nativeA: path.join(scenarioRoot, "native-a-run.log"),
    nativeB: path.join(scenarioRoot, "native-b-run.log"),
    proxyA: path.join(scenarioRoot, "native-a.proxy.log"),
    proxyB: path.join(scenarioRoot, "native-b.proxy.log"),
  }
  const automationSocket = path.join("/tmp", `arb-rnt-${provider}-${process.pid}.sock`)
  let client = null
  let sessionId = null
  let nativeCapabilities = null
  try {
    await mkdir(logs.aDir, { recursive: true })
    await mkdir(logs.bDir, { recursive: true })
    await mkdir(logs.cliDir, { recursive: true })
    client = relayClient(relayUrl, relayToken, targetDaemonAlias)
    nativeCapabilities = await installNativeDrillCapabilities({
      homeClient: client,
      workerKernelUrl,
      provider,
      scenarioRoot,
      workspace,
      options,
      markers,
    })
    await client.close().catch(() => {})
    client = null

    await startScreen(screenA, logs.aDir, "bun", [
      cliPath,
      provider,
      "--relay-url",
      relayUrl,
      "--relay-token",
      relayToken,
      "--target-daemon-alias",
      targetDaemonAlias,
      "--alias",
      `remote-native-${provider}-${process.pid}`,
      "--agent-alias",
      aliases[0],
      "--workspace",
      workspace,
      "--worktree",
      worktree,
      ...(machineRef ? ["--machine", machineRef] : []),
      ...(sliceRef ? ["--slice", sliceRef] : []),
      ...(nativeCapabilities ? ["--grant-mcp", nativeCapabilities.mcpName, "--grant-skill", nativeCapabilities.skillName] : []),
      ...providerArgs,
      ...(provider === "claude" && !skipBaselineTurns ? ["--initial-prompt", `Reply with exactly ${markers.nativeA} and nothing else.`] : []),
      ...(provider === "claude" ? ["--remote-rendered"] : []),
    ], {
      ...process.env,
      ...providerBinaryEnv,
      ...nativeEnv,
      ARROBA_CODEX_NATIVE_DEBUG: "1",
      ARROBA_CODEX_NATIVE_DEBUG_FILE: logs.proxyA,
      ARROBA_OPENCODE_NATIVE_DEBUG: "1",
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: logs.proxyA,
      ARROBA_CLAUDE_NATIVE_DEBUG: "1",
      ARROBA_CLAUDE_NATIVE_DEBUG_FILE: logs.proxyA,
    })
    sessionId = (await waitForFileMatch(logs.a, /arroba session:\s+([^\s(]+)/)).match[1]

    await startScreen(screenB, logs.bDir, "bun", [
      cliPath,
      provider,
      sessionId,
      "--relay-url",
      relayUrl,
      "--relay-token",
      relayToken,
      "--target-daemon-alias",
      targetDaemonAlias,
      "--agent-alias",
      aliases[1],
      "--workspace",
      workspace,
      "--worktree",
      worktree,
      ...(machineRef ? ["--machine", machineRef] : []),
      ...(sliceRef ? ["--slice", sliceRef] : []),
      ...(nativeCapabilities ? ["--grant-mcp", nativeCapabilities.mcpName, "--grant-skill", nativeCapabilities.skillName] : []),
      ...providerArgs,
      ...(provider === "claude" && !skipBaselineTurns ? ["--initial-prompt", `Reply with exactly ${markers.nativeB} and nothing else.`] : []),
      ...(provider === "claude" ? ["--remote-rendered"] : []),
    ], {
      ...process.env,
      ...providerBinaryEnv,
      ...nativeEnv,
      ARROBA_CODEX_NATIVE_DEBUG: "1",
      ARROBA_CODEX_NATIVE_DEBUG_FILE: logs.proxyB,
      ARROBA_OPENCODE_NATIVE_DEBUG: "1",
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: logs.proxyB,
      ARROBA_CLAUDE_NATIVE_DEBUG: "1",
      ARROBA_CLAUDE_NATIVE_DEBUG_FILE: logs.proxyB,
    })

    let proxyA = null
    let proxyB = null
    let providerSessionA = null
    let providerSessionB = null
    if (provider === "opencode") {
      proxyA = (await waitForFileMatch(logs.a, /proxy:\s+(http:\/\/127\.0\.0\.1:\d+)/)).match[1]
      proxyB = (await waitForFileMatch(logs.b, /proxy:\s+(http:\/\/127\.0\.0\.1:\d+)/)).match[1]
      providerSessionA = (await waitForFileMatch(logs.a, /opencode sess:\s+([^\s]+)/)).match[1]
      providerSessionB = (await waitForFileMatch(logs.b, /opencode sess:\s+([^\s]+)/)).match[1]
    } else if (provider === "codex") {
      proxyA = (await waitForFileMatch(logs.a, /proxy:\s+(ws:\/\/127\.0\.0\.1:\d+)/)).match[1]
      proxyB = (await waitForFileMatch(logs.b, /proxy:\s+(ws:\/\/127\.0\.0\.1:\d+)/)).match[1]
      await dismissCodexUpdatePromptIfPresent(screenA, logs.a)
      await dismissCodexUpdatePromptIfPresent(screenB, logs.b)
      if (!remotePlacement) {
        providerSessionA = (await waitForFileMatch(logs.proxyA, /thread_observed:\s+\{"threadId":"([^"]+)"/)).match[1]
        providerSessionB = (await waitForFileMatch(logs.proxyB, /thread_observed:\s+\{"threadId":"([^"]+)"/)).match[1]
      }
    }

    client = relayClient(relayUrl, relayToken, targetDaemonAlias)
    const attachment = unwrap(
      await client.send(attachToSessionRequest(sessionId, `remote-native-${provider}-drill-${process.pid}`)),
      "SessionAttached",
    ).attachment
    const agents = await waitForNamedAgents(client, sessionId, aliases)
    if (!remotePlacement) {
      await waitForActiveProviderRun(client, sessionId)
    }
    if (nativeCapabilities) {
      const providerRunIds = []
      if (provider === "opencode" || provider === "claude" || remotePlacement) {
        providerRunIds.push((await waitForFileMatch(logs.a, /provider run:\s+([^\s]+)/, 90_000)).match[1])
        providerRunIds.push((await waitForFileMatch(logs.b, /provider run:\s+([^\s]+)/, 90_000)).match[1])
      }
      for (const providerRunId of providerRunIds) {
        await waitForProviderRunMcpGrant(client, providerRunId, nativeCapabilities.mcpName)
      }
    }

    await startScreen(screenCli, logs.cliDir, "bun", [
      cliPath,
      "--relay-url",
      relayUrl,
      "--relay-token",
      relayToken,
      "--target-daemon-alias",
      targetDaemonAlias,
      "--session",
      sessionId,
      "--client-id",
      `arroba-remote-native-observer-${provider}-${process.pid}`,
      "--automation-socket",
      automationSocket,
      "--provider",
      provider,
      "--model",
      provider === "codex" ? "gpt-5.4-mini" : provider === "claude" ? "sonnet" : "default",
      ...(provider === "codex" ? ["--effort", "high"] : []),
    ], process.env)
    for (let attempt = 0; attempt < 80; attempt += 1) {
      try {
        await automationRequest(automationSocket, { action: "ping" })
        break
      } catch (error) {
        if (attempt === 79) throw error
        await sleep(250)
      }
    }
    const snapshot = await automationRequest(automationSocket, {
      action: "wait_for",
      sessionId,
      shellEntryCount: 0,
      timeoutMs: 20_000,
    })
    if (snapshot.session.agentCount < 2) {
      throw new Error(`observer CLI did not see both ${provider} agents: ${JSON.stringify(snapshot.session)}`)
    }

    const badgeTransitions = {
      [aliases[0]]: {
        before: await waitForAgentBadgeTone(automationSocket, aliases[0], "idle"),
      },
      [aliases[1]]: {
        before: await waitForAgentBadgeTone(automationSocket, aliases[1], "idle"),
      },
    }

    if (!skipBaselineTurns) {
      if (provider === "opencode") {
        await runNativeOpenCodePrompt(proxyA, providerSessionA, worktree, `Reply with exactly ${markers.nativeA} and nothing else.`, logs.nativeA)
        await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
          [aliases[0]]: { prompts: [markers.nativeA], outputs: [markers.nativeA] },
        })
        await runNativeOpenCodePrompt(proxyB, providerSessionB, worktree, `Reply with exactly ${markers.nativeB} and nothing else.`, logs.nativeB)
      } else if (provider === "codex") {
        await runNativeCodexPrompt(proxyA, providerSessionA, `Reply with exactly ${markers.nativeA} and nothing else.`)
        await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
          [aliases[0]]: { prompts: [markers.nativeA], outputs: [markers.nativeA] },
        })
        await runNativeCodexPrompt(proxyB, providerSessionB, `Reply with exactly ${markers.nativeB} and nothing else.`)
      }

      await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
        [aliases[0]]: { prompts: [markers.nativeA], outputs: [markers.nativeA] },
        [aliases[1]]: { prompts: [markers.nativeB], outputs: [markers.nativeB] },
      })

      await fireAutomationRequest(automationSocket, {
        action: "workspace_shell_exec",
        command: `prompt ${aliases[0]} Reply with exactly ${markers.arrobaA} and nothing else.`,
      })
      badgeTransitions[aliases[0]].during = await waitForAgentBadgeTone(automationSocket, aliases[0], "working")
      await fireAutomationRequest(automationSocket, {
        action: "workspace_shell_exec",
        command: `prompt ${aliases[1]} Reply with exactly ${markers.arrobaB} and nothing else.`,
      })
      badgeTransitions[aliases[1]].during = await waitForAgentBadgeTone(automationSocket, aliases[1], "working")

      const histories = await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
        [aliases[0]]: { prompts: [markers.arrobaA, markers.nativeA], outputs: [markers.arrobaA, markers.nativeA] },
        [aliases[1]]: { prompts: [markers.arrobaB, markers.nativeB], outputs: [markers.arrobaB, markers.nativeB] },
      })
      badgeTransitions[aliases[0]].after = await waitForAgentBadgeTone(automationSocket, aliases[0], "idle")
      badgeTransitions[aliases[1]].after = await waitForAgentBadgeTone(automationSocket, aliases[1], "idle")

      if (histories[aliases[0]].all.includes(markers.arrobaB) || histories[aliases[0]].all.includes(markers.nativeB)) {
        throw new Error(`${aliases[0]} history was contaminated with ${aliases[1]} markers`)
      }
      if (histories[aliases[1]].all.includes(markers.arrobaA) || histories[aliases[1]].all.includes(markers.nativeA)) {
        throw new Error(`${aliases[1]} history was contaminated with ${aliases[0]} markers`)
      }

      await automationRequest(automationSocket, { action: "switch_screen", screen: "agents" })
      await sleep(1_000)
      for (const expected of [markers.arrobaA, markers.nativeA]) {
        await waitForFileMatch(logs.a, new RegExp(expected), 90_000)
      }
      for (const expected of [markers.arrobaB, markers.nativeB]) {
        await waitForFileMatch(logs.b, new RegExp(expected), 90_000)
      }
    } else if (provider === "claude") {
      const providerRunB = await waitForClaudeProviderRunId(logs.b)
      await sendClaudeRenderedPromptViaKernelInput(
        client,
        sessionId,
        attachment.id,
        providerRunB,
        `Reply with exactly ${markers.nativeB} and nothing else.`,
      )
      badgeTransitions[aliases[1]].during = await waitForAgentBadgeTone(automationSocket, aliases[1], "working")
      await waitForHistoryMarkers(client, sessionId, attachment.id, [agents[1]], {
        [aliases[1]]: { prompts: [markers.nativeB], outputs: [markers.nativeB] },
      })
      await fireAutomationRequest(automationSocket, {
        action: "workspace_shell_exec",
        command: `prompt ${aliases[1]} Reply with exactly ${markers.arrobaB} and nothing else.`,
      })
      const histories = await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
        [aliases[1]]: { prompts: [markers.arrobaB, markers.nativeB], outputs: [markers.arrobaB, markers.nativeB] },
      })
      badgeTransitions[aliases[1]].after = await waitForAgentBadgeTone(automationSocket, aliases[1], "idle")
      if (histories[aliases[0]].all.includes(markers.arrobaB) || histories[aliases[0]].all.includes(markers.nativeB)) {
        throw new Error(`${aliases[0]} history was contaminated with ${aliases[1]} markers`)
      }
      await waitForFileMatch(logs.b, new RegExp(markers.nativeB), 90_000)
      await waitForFileMatch(logs.b, new RegExp(markers.arrobaB), 90_000)
    }

    const proxyALog = await readFile(logs.proxyA, "utf8").catch(() => "")
    const proxyBLog = await readFile(logs.proxyB, "utf8").catch(() => "")
    if (provider === "codex") {
      const expectedProxySignal = nativeEnv.ARROBA_CODEX_KERNEL_SERVER_PORT_RANGE
        ? "provider_run_bound"
        : remotePlacement
          ? "native_prompt_submitted"
        : "kernel_connected"
      if (!proxyALog.includes(expectedProxySignal) || !proxyBLog.includes(expectedProxySignal)) {
        throw new Error(`remote Codex native proxies did not record ${expectedProxySignal}`)
      }
    } else if (provider === "opencode" && (!proxyALog.includes(markers.nativeA) || !proxyBLog.includes(markers.nativeB))) {
      throw new Error("native OpenCode prompts did not pass through both remote native proxies")
    }

    const extendedChecks = {}
    if (nativeCapabilities) {
      let nativeSkillCheck = "validated"
      let skillPromptContext = "validated_native_and_arroba_origin"
      if (provider !== "claude") {
        const nativeSkillPrompt = `Use the ${nativeCapabilities.skillName} skill. Give the native skill marker.`
        if (provider === "opencode") {
          await runNativeOpenCodePrompt(proxyA, providerSessionA, worktree, nativeSkillPrompt, logs.nativeA)
        } else {
          await runNativeCodexPrompt(proxyA, providerSessionA, nativeSkillPrompt)
        }
        await waitForHistoryMarkers(client, sessionId, attachment.id, [agents[0]], {
          [aliases[0]]: { prompts: [nativeCapabilities.skillName], outputs: [markers.nativeSkill] },
        })
        await waitForFileMatch(logs.a, new RegExp(markers.nativeSkill), 90_000)
        await automationRequest(automationSocket, {
          action: "workspace_shell_exec",
          command: `agent focus ${agents[0].id}`,
        })
        await automationRequest(automationSocket, {
          action: "workspace_shell_exec",
          command: `prompt ${aliases[0]} ${shellQuote(`Use the ${nativeCapabilities.skillName} skill. Give the Arroba skill marker.`)}`,
        })
        await waitForHistoryMarkers(client, sessionId, attachment.id, [agents[0]], {
          [aliases[0]]: { prompts: [nativeCapabilities.skillName], outputs: [markers.arrobaSkill] },
        })
        await waitForFileMatch(logs.a, new RegExp(markers.arrobaSkill), 90_000)
      } else {
        const providerRunA = await waitForClaudeProviderRunId(logs.a)
        await sendClaudeRenderedPromptViaKernelInput(
          client,
          sessionId,
          attachment.id,
          providerRunA,
          `Use the ${nativeCapabilities.skillName} skill. Give the native skill marker.`,
        )
        await waitForHistoryMarkers(client, sessionId, attachment.id, [agents[0]], {
          [aliases[0]]: { prompts: [nativeCapabilities.skillName], outputs: [markers.nativeSkill] },
        })
        await waitForFileMatch(logs.a, new RegExp(markers.nativeSkill), 90_000)
        await automationRequest(automationSocket, {
          action: "workspace_shell_exec",
          command: `agent focus ${agents[0].id}`,
        })
        await automationRequest(automationSocket, {
          action: "workspace_shell_exec",
          command: `prompt ${aliases[0]} ${shellQuote(`Use the ${nativeCapabilities.skillName} skill. Give the Arroba skill marker.`)}`,
        })
        await waitForHistoryMarkers(client, sessionId, attachment.id, [agents[0]], {
          [aliases[0]]: { prompts: [nativeCapabilities.skillName], outputs: [markers.arrobaSkill] },
        })
        await waitForFileMatch(logs.a, new RegExp(markers.arrobaSkill), 90_000)
        const claudeScreenLog = await readFile(logs.a, "utf8").catch(() => "")
        if (claudeScreenLog.includes("Full instructions for explicitly requested Arroba skills")) {
          throw new Error("Claude native TUI displayed hidden Arroba skill context")
        }
      }
      extendedChecks.mcpSkills = {
        mcp: nativeCapabilities.mcpName,
        skill: nativeCapabilities.skillName,
        providerRunMcpConfig: provider === "codex" && !remotePlacement ? "not directly observable before local codex bind" : "validated",
        skillPromptContext,
        nativeSkillCheck,
      }
    }

    if (options.includeAttachments) {
      const nativeAttachmentPath = path.join(
        scenarioRoot,
        provider === "opencode" ? "native-attachment.txt" : "native-attachment.png",
      )
      const arrobaAttachmentPath = path.join(
        scenarioRoot,
        provider === "opencode" ? "arroba-attachment.txt" : "arroba-attachment.png",
      )
      if (provider === "codex") {
        await writeFile(nativeAttachmentPath, tinyPng)
        await writeFile(arrobaAttachmentPath, tinyPng)
        await runNativeCodexPrompt(proxyA, providerSessionA, attachedImagePrompt(markers.nativeAttachment), [
          { type: "localImage", path: nativeAttachmentPath },
        ])
      } else if (provider === "claude") {
        await writeFile(nativeAttachmentPath, tinyPng)
        await writeFile(arrobaAttachmentPath, tinyPng)
        await screenStuff(screenA, `@${nativeAttachmentPath} ${attachedImagePrompt(markers.nativeAttachment)}`)
        await sleep(250)
        await screenStuff(screenA, "\r")
      } else {
        await writeFile(nativeAttachmentPath, `native ${provider} attachment ${markers.nativeAttachment}\n`)
        await writeFile(arrobaAttachmentPath, `arroba ${provider} attachment ${markers.arrobaAttachment}\n`)
        await runNativeOpenCodePrompt(
          proxyA,
          providerSessionA,
          worktree,
          attachedFilePrompt(markers.nativeAttachment),
          logs.nativeA,
          nativeAttachmentPath,
        )
      }
      if (provider !== "claude") {
        await waitForLogOccurrences(
          logs.proxyA,
          provider === "codex" ? "attachmentCount\":1" : "native_prompt_attachments_observed",
          1,
        )
      } else {
        await waitForLogOccurrences(logs.proxyA, "remote_rendered_attachments_intercepted", 1)
      }
      await waitForHistoryMarkers(client, sessionId, attachment.id, [agents[0]], {
        [aliases[0]]: { prompts: [markers.nativeAttachment], outputs: [markers.nativeAttachment] },
      })
      await waitForFileMatch(logs.a, new RegExp(markers.nativeAttachment), 60_000)
      if (provider === "claude") {
        await waitForFileMatch(logs.a, /native-attach/, 60_000)
      }

      await automationRequest(automationSocket, {
        action: "workspace_shell_exec",
        command: `agent focus ${agents[0].id}`,
      })
      await automationRequest(automationSocket, {
        action: "submit_prompt",
        prompt: provider === "opencode"
          ? attachedFilePrompt(markers.arrobaAttachment)
          : provider === "claude"
            ? attachedImagePrompt(markers.arrobaAttachment)
            : attachedImagePrompt(markers.arrobaAttachment),
        attachments: [{
          url: arrobaAttachmentPath,
          mime: provider === "opencode" ? "text/plain" : "image/png",
          filename: path.basename(arrobaAttachmentPath),
        }],
      })
      await waitForHistoryMarkers(client, sessionId, attachment.id, [agents[0]], {
        [aliases[0]]: { prompts: [markers.arrobaAttachment], outputs: [markers.arrobaAttachment] },
      })
      await waitForFileMatch(logs.a, new RegExp(markers.arrobaAttachment), 60_000)
      if (provider === "claude") {
        await waitForFileMatch(logs.a, /arroba-attach/, 60_000)
      }
      extendedChecks.attachments = "validated"
    }

    if (options.includePermissions) {
      const remoteExecution = Boolean(options.hetznerWorker && machineRef)
      await ensureExecutionDirectory(options, remoteExecution, path.join(worktree, "outputs"))
      const nativePermissionFile = path.join(worktree, "outputs", `remote-native-${provider}-${process.pid}-native-permission.txt`)
      const arrobaPermissionFile = path.join(worktree, "outputs", `remote-native-${provider}-${process.pid}-arroba-permission.txt`)
      await removeExecutionFile(options, remoteExecution, nativePermissionFile)
      await removeExecutionFile(options, remoteExecution, arrobaPermissionFile)

      const nativePermissionContent = `native-${provider}`
      const arrobaPermissionContent = `arroba-${provider}`
      const nativePrompt = provider === "claude"
        ? claudePermissionPrompt(markers.nativePermission, nativePermissionFile, nativePermissionContent)
        : permissionPrompt(markers.nativePermission, nativePermissionFile, nativePermissionContent)
      await automationRequest(automationSocket, {
        action: "workspace_shell_exec",
        command: `agent focus ${agents[0].id}`,
      })
      if (provider === "opencode") {
        const nativeRun = await runNativeOpenCodePromptDetached(proxyA, providerSessionA, worktree, nativePrompt)
        const interaction = await answerPermissionFromCli(automationSocket, aliases[0])
        await nativeRun.wait()
        extendedChecks.nativePermissionInteraction = interaction.title ?? interaction.message
      } else if (provider === "codex") {
        await runNativeCodexPrompt(proxyA, providerSessionA, nativePrompt)
        const interaction = await answerPermissionFromCli(automationSocket, aliases[0])
        extendedChecks.nativePermissionInteraction = interaction.title ?? interaction.message
      } else {
        const providerRunA = await waitForClaudeProviderRunId(logs.a)
        await sendClaudeRenderedPromptViaKernelInput(client, sessionId, attachment.id, providerRunA, nativePrompt)
        if (!badgeTransitions[aliases[0]].during) {
          badgeTransitions[aliases[0]].during = await waitForAgentBadgeTone(automationSocket, aliases[0], "working")
        }
        const interaction = await answerPermissionFromCli(automationSocket, aliases[0])
        extendedChecks.nativePermissionInteraction = interaction.title ?? interaction.message
      }
      if (provider !== "claude") {
        await waitForHistoryMarkers(client, sessionId, attachment.id, [agents[0]], {
          [aliases[0]]: { prompts: [markers.nativePermission], outputs: [markers.nativePermission] },
        })
        await waitForProviderToolCompletion(client, sessionId, attachment.id, agents[0].id, (_update, raw) =>
          raw.includes(nativePermissionFile))
      }
      await waitForExecutionFileContent(
        options,
        remoteExecution,
        nativePermissionFile,
        provider === "claude" ? nativePermissionContent : `${nativePermissionContent}\n`,
        10_000,
      )
      if (provider === "claude" && !badgeTransitions[aliases[0]].after) {
        badgeTransitions[aliases[0]].after = await waitForAgentBadgeTone(automationSocket, aliases[0], "idle")
      }
      const arrobaPrompt = provider === "claude"
        ? claudePermissionPrompt(markers.arrobaPermission, arrobaPermissionFile, arrobaPermissionContent)
        : permissionPrompt(markers.arrobaPermission, arrobaPermissionFile, arrobaPermissionContent)
      await automationRequest(automationSocket, {
        action: "workspace_shell_exec",
        command: `prompt ${aliases[0]} ${shellQuote(arrobaPrompt)}`,
      })
      if (provider === "claude") {
        const interaction = await answerPermissionFromCli(automationSocket, aliases[0])
        extendedChecks.arrobaPermissionInteraction = interaction.title ?? interaction.message
      } else {
        const interaction = await answerPermissionFromCli(automationSocket, aliases[0])
        extendedChecks.arrobaPermissionInteraction = interaction.title ?? interaction.message
      }
      if (provider !== "claude") {
        await waitForHistoryMarkers(client, sessionId, attachment.id, [agents[0]], {
          [aliases[0]]: { prompts: [markers.arrobaPermission], outputs: [markers.arrobaPermission] },
        })
        await waitForProviderToolCompletion(client, sessionId, attachment.id, agents[0].id, (_update, raw) =>
          raw.includes(arrobaPermissionFile))
      }
      await waitForExecutionFileContent(
        options,
        remoteExecution,
        arrobaPermissionFile,
        provider === "claude" ? arrobaPermissionContent : `${arrobaPermissionContent}\n`,
        10_000,
      )
      await removeExecutionFile(options, remoteExecution, nativePermissionFile)
      await removeExecutionFile(options, remoteExecution, arrobaPermissionFile)
      extendedChecks.permissions = "validated"
    }

    return {
      provider,
      sessionId,
      marker,
      relayUrl,
      targetDaemonAlias,
      machineRef: machineRef ?? null,
      sliceRef: sliceRef ?? null,
      agentAliases: aliases,
      observerSawAgents: snapshot.session.agentCount,
      badgeTransitions,
      providerSessions: provider === "opencode" || provider === "codex" ? {
        [aliases[0]]: providerSessionA,
        [aliases[1]]: providerSessionB,
      } : null,
      extendedChecks,
      logs,
      note: provider === "claude"
        ? "remote-rendered Claude TUI validated through kernel-owned PTY"
        : options.hetznerWorker
          ? "server-in-kernel native TUI validated against a Hetzner worker through the SSH provider endpoint bridge"
          : sliceRef
            ? "server-in-kernel native TUI validated through a home-managed slice_ref placement"
          : "server-in-kernel native TUI validated on one host; use --hetzner-worker to validate cross-host provider endpoints",
    }
  } finally {
    await cleanupNativeDrillCapabilities(workspace, nativeCapabilities)
    if (client) await client.close().catch(() => {})
    await screenQuit(screenA)
    await screenQuit(screenB)
    await screenQuit(screenCli)
    await rm(automationSocket, { force: true }).catch(() => {})
  }
}

async function createHomeManagedLocalDockerSlice({ homeKernelUrl, workspace, providers, relayUrl, relayToken }) {
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
    await waitForRelayTarget(relayUrl, relayToken, started.worker_kernel_ref, started.worker_kernel_id ?? null)
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
    path.join(repoRoot, "experiments/slice-spike/docker/Dockerfile"),
    "-t",
    "arroba-slice-linux-spike:local",
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
  await assertBinary(kernelBinary, path.join(repoRoot, "apps/kernel/Cargo.toml"), "arroba-kernel")
  await assertBinary(relayBinary, path.join(repoRoot, "apps/relay/Cargo.toml"), "arroba-relay")

  const root = path.join("/tmp", `arb-remote-native-tui-${process.pid}-${Date.now()}`)
  const ports = await makeAvailablePorts()
  const relayToken = `remote-native-token-${process.pid}`
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const homeKernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const targetDaemonAlias = `remote-native-home-${process.pid}`
  const workerDaemonAlias = `remote-native-worker-${process.pid}`
  const workerMachineAlias = `remote-native-worker-machine-${process.pid}`
  const workerKernelUrl = options.hetznerWorker ? null : `ws://127.0.0.1:${ports.workerKernelPort}`
  const workspace = repoRoot
  const worktree = repoRoot
  const homeDir = path.join(root, "home")
  const xdgConfigHome = path.join(root, "xdg-config")
  const xdgStateHome = path.join(root, "xdg-state")
  const xdgDataHome = path.join(root, "xdg-data")
  const xdgCacheHome = path.join(root, "xdg-cache")
  const sliceBuildImagePolicy = process.env.ARROBA_NATIVE_TUI_SLICE_BUILD_IMAGE ?? "always"
  const rustMinStack = process.env.RUST_MIN_STACK ?? "16777216"
  let relay = null
  let relayTunnel = null
  let kernel = null
  let workerKernel = null
  const managedSlices = []
  let succeeded = false
  try {
    await mkdir(root, { recursive: true })
    await mkdir(homeDir, { recursive: true })
    await mkdir(xdgConfigHome, { recursive: true })
    await mkdir(xdgStateHome, { recursive: true })
    await mkdir(xdgDataHome, { recursive: true })
    await mkdir(xdgCacheHome, { recursive: true })
    if (options.homeManagedSliceLocalDocker) {
      await prebuildLocalDockerSliceImageIfNeeded(sliceBuildImagePolicy)
      const configDir = path.join(xdgConfigHome, "arroba")
      await mkdir(configDir, { recursive: true })
      await writeFile(path.join(configDir, "config.toml"), [
        "version = 1",
        "",
        "[slices]",
        `root = ${JSON.stringify(path.join(root, "slices"))}`,
        "",
        "[slices.linux]",
        "docker_image = \"arroba-slice-linux-spike:local\"",
        `build_image = ${JSON.stringify(sliceBuildImagePolicy === "always" ? "auto" : sliceBuildImagePolicy)}`,
        "",
      ].join("\n"))
    }
    if (options.hetznerWorker) {
      await prepareHetznerWorktree(options, worktree)
    }
    await access(path.join(realHomeDir, ".claude"))
      .then(() => symlink(path.join(realHomeDir, ".claude"), path.join(homeDir, ".claude"), "dir"))
      .catch(() => {})
    await access(path.join(realHomeDir, ".claude.json"))
      .then(() => symlink(path.join(realHomeDir, ".claude.json"), path.join(homeDir, ".claude.json")))
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
        ARROBA_DAEMON_ID: `remote-native-home-${process.pid}-${Date.now()}`,
        ARROBA_DAEMON_ALIAS: targetDaemonAlias,
        ARROBA_MACHINE_ID: `remote-native-machine-${process.pid}`,
        ARROBA_MACHINE_ALIAS: targetDaemonAlias,
        ARROBA_ACCEPT_REMOTE_LEASES: "0",
        ARROBA_DAEMON_SOCKET: path.join(root, "home.sock"),
        ARROBA_SESSION_HISTORY_DIR: path.join(root, "history"),
        RUST_MIN_STACK: rustMinStack,
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForLocalDaemon(homeKernelUrl, workspace, worktree)
    await waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias)
    if (options.standardHomeWorker) {
      if (options.hetznerWorker) {
        const remoteRoot = `/tmp/arb-remote-native-tui-${process.pid}-${Date.now()}`
        workerKernel = spawn("ssh", sshArgs(options, remoteEnvCommand({
          ARROBA_REMOTE_REPO: options.hetznerRepo,
          RUST_MIN_STACK: rustMinStack,
          PATH: `/root/.bun/bin:/root/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin`,
          HOME: "/root",
          XDG_CONFIG_HOME: "/root/.config",
          XDG_STATE_HOME: "/root/.local/state",
          XDG_DATA_HOME: "/root/.local/share",
          XDG_CACHE_HOME: "/root/.cache",
          OPENCODE_CONFIG_DIR: "/root/.config/opencode",
          ARROBA_LOG_DIR: path.posix.join(remoteRoot, "worker-logs"),
          ARROBA_KERNEL_PORT: String(ports.workerKernelPort),
          ARROBA_MCP_PORT: String(ports.workerMcpPort),
          ARROBA_RELAY_URL: `ws://127.0.0.1:${ports.relayPort}`,
          ARROBA_RELAY_TOKEN: relayToken,
          ARROBA_DAEMON_ID: `remote-native-worker-${process.pid}-${Date.now()}`,
          ARROBA_DAEMON_ALIAS: workerDaemonAlias,
          ARROBA_MACHINE_ID: workerMachineAlias,
          ARROBA_MACHINE_ALIAS: workerMachineAlias,
          ARROBA_ACCEPT_REMOTE_LEASES: "1",
          ARROBA_DAEMON_SOCKET: path.posix.join(remoteRoot, "worker.sock"),
          ARROBA_SESSION_HISTORY_DIR: path.posix.join(remoteRoot, "worker-history"),
        }, `mkdir -p /tmp/arb-remote-native-tui-${process.pid} && ./apps/kernel/target/debug/arroba-kernel`)), {
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
            ARROBA_DAEMON_ID: `remote-native-worker-${process.pid}-${Date.now()}`,
            ARROBA_DAEMON_ALIAS: workerDaemonAlias,
            ARROBA_MACHINE_ID: workerMachineAlias,
            ARROBA_MACHINE_ALIAS: workerMachineAlias,
            ARROBA_ACCEPT_REMOTE_LEASES: "1",
            ARROBA_DAEMON_SOCKET: path.join(root, "worker.sock"),
            ARROBA_SESSION_HISTORY_DIR: path.join(root, "worker-history"),
            RUST_MIN_STACK: rustMinStack,
          },
          stdio: ["ignore", "ignore", "inherit"],
        })
        await waitForLocalDaemon(workerKernelUrl, workspace, worktree)
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
          relayUrl,
          relayToken,
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
  } finally {
    if (succeeded || !options.keepArtifactsOnFailure) {
      for (const slice of managedSlices.splice(0)) {
        await deleteHomeManagedSlice(homeKernelUrl, slice.id).catch((error) => {
          console.error(`home-managed slice cleanup failed: ${error.message}`)
        })
      }
    }
    await terminateChild(workerKernel)
    await terminateChild(kernel)
    await terminateChild(relayTunnel)
    await terminateChild(relay)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(root, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`remote native TUI drill artifacts kept at ${root}`)
      for (const slice of managedSlices) {
        console.error(`home-managed slice ${slice.id} left running`)
      }
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
