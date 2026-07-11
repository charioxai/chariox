import net from "node:net"
import path from "node:path"
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises"
import { setTimeout as sleep } from "node:timers/promises"
import { LocalIpcClient } from "../../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  endSessionRequest,
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
  getSessionStateRequest,
  listAgentsRequest,
  listRemoteMachinesRequest,
  listSessionsRequest,
  pumpTerminalOutputRequest,
} from "../../dist/ipc-requests.js"
import {
  cleanupNativeDrillCapabilities,
  installNativeDrillCapabilities,
  waitForProviderRunMcpGrant,
} from "./native-tui-capabilities.mjs"
import {
  ensureExecutionDirectory,
  removeExecutionFile,
  shellQuote,
  waitForExecutionFileContent,
} from "./native-tui-remote-execution.mjs"
import {
  providerAuthFailureFromTerminalText,
  resolveCommandPath,
  screenQuit,
  screenStuff,
  startScreen,
  terminateMatchingProcesses,
  waitForFileMatch,
  waitForLogOccurrences,
  waitForScreenMatch,
} from "./drill-runtime-helpers.mjs"
import {
  runNativeCodexPrompt,
  runNativeOpenCodePrompt,
  runNativeOpenCodePromptDetached,
  sendClaudeRenderedPromptViaKernelInput,
} from "./native-tui-provider-drivers.mjs"

const repoRoot = path.resolve(new URL("../../../..", import.meta.url).pathname)
const cliRoot = path.resolve(repoRoot, "apps/cli")
const cliPath = path.join(cliRoot, "dist/index.js")
const tinyPng = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=", "base64")

function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

export async function waitForLocalDaemon(kernelUrl, workspace, worktree) {
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

export async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias, targetDaemonId = null) {
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

export async function waitForRemoteMachine(relayUrl, relayToken, targetDaemonAlias, machineAlias) {
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

async function waitForAutomationReady(socketPath, cliLogDir, timeoutMs = 90_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      await automationRequest(socketPath, { action: "ping" })
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  const structuredLogDir = path.join(cliLogDir, ".arroba", "logs")
  const structuredLogs = await readdir(structuredLogDir).catch(() => [])
  const latestLog = structuredLogs.sort().at(-1)
  const logTail = latestLog
    ? (await readFile(path.join(structuredLogDir, latestLog), "utf8").catch(() => "")).slice(-4_000)
    : ""
  throw new Error([
    `observer automation socket did not become ready within ${timeoutMs}ms: ${socketPath}`,
    `last connection error: ${lastError?.message ?? "none"}`,
    logTail ? `observer log tail:\n${logTail}` : "observer emitted no structured log",
  ].join("\n"))
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

async function waitForSessionByAlias(client, alias, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const sessions = unwrap(await client.send(listSessionsRequest()), "SessionsListed").sessions ?? []
    const session = sessions.find((candidate) => candidate.alias === alias)
    if (session) return session
    await sleep(250)
  }
  throw new Error(`timed out waiting for run-owned session ${alias}`)
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
  let lastHistories = {}
  let lastMissing = {}
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    let ok = true
    const histories = {}
    const missing = {}
    for (const agent of agents) {
      const entries = await loadAgentHistoryEntries(client, sessionId, agent.id, 200)
      histories[agent.alias] = {
        all: entries.map((entry) => entry.text ?? "").join("\n"),
        prompts: entries.filter((entry) => entry.kind === "user_prompt").map((entry) => entry.text ?? "").join("\n"),
        outputs: entries.filter((entry) => entry.kind !== "user_prompt").map((entry) => entry.text ?? "").join(""),
      }
      const providerAuthFailure = providerAuthFailureFromTerminalText(histories[agent.alias].outputs)
      if (providerAuthFailure) {
        throw new Error(`provider authentication failed for ${agent.alias}: ${providerAuthFailure}`)
      }
      const expected = expectedByAgent[agent.alias] ?? {}
      for (const marker of expected.prompts ?? []) {
        if (!histories[agent.alias].prompts.includes(marker)) {
          ok = false
          missing[agent.alias] ??= []
          missing[agent.alias].push(`prompt:${marker}`)
        }
      }
      for (const marker of expected.outputs ?? []) {
        if (!histories[agent.alias].outputs.includes(marker)) {
          ok = false
          missing[agent.alias] ??= []
          missing[agent.alias].push(`output:${marker}`)
        }
      }
    }
    if (ok) return histories
    lastHistories = histories
    lastMissing = missing
    await sleep(1_000)
  }
  const seen = Object.fromEntries(Object.entries(lastHistories).map(([alias, history]) => [
    alias,
    { promptBytes: history.prompts.length, outputBytes: history.outputs.length },
  ]))
  throw new Error(`timed out waiting for all history markers; missing=${JSON.stringify(lastMissing)}; seen=${JSON.stringify(seen)}`)
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

async function waitForProviderToolCompletion(client, sessionId, attachmentId, agentId, matcher, timeoutMs = 240_000) {
  const deadline = Date.now() + timeoutMs
  let lastMatch = null
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const entries = await loadAgentHistoryEntries(client, sessionId, agentId, 20)
    for (const entry of entries) {
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

async function loadAgentHistoryEntries(client, sessionId, agentId, latestPromptCount) {
  const outline = unwrap(
    await client.send(getSessionHistoryOutlineRequest(sessionId, [agentId], latestPromptCount)),
    "SessionHistoryOutline",
  )
  const entries = []
  const agent = outline.agents?.find((entry) => entry.agent_id === agentId)
  for (const turn of agent?.turns ?? []) {
    if (turn.user_prompt?.entry) entries.push(turn.user_prompt.entry)
    for (const row of turn.entries ?? []) {
      if (row?.entry) entries.push(row.entry)
    }
    if (turn.summary?.entry) entries.push(turn.summary.entry)
    for (const blob of turn.blobs ?? []) {
      const content = unwrap(
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

export async function runProviderScenario({
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
  const sessionAlias = `remote-native-${provider}-${process.pid}`
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
    renderedA: path.join(scenarioRoot, "native-a-rendered.txt"),
    renderedB: path.join(scenarioRoot, "native-b-rendered.txt"),
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
      sessionAlias,
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
    client = relayClient(relayUrl, relayToken, targetDaemonAlias)
    sessionId = (await waitForSessionByAlias(client, sessionAlias)).id
    await client.close().catch(() => {})
    client = null
    const bannerSessionId = (await waitForFileMatch(logs.a, /arroba session:\s+([^\s(]+)/)).match[1]
    if (bannerSessionId !== sessionId) {
      throw new Error(`native TUI banner session ${bannerSessionId} did not match run-owned session ${sessionId}`)
    }

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
    await waitForAutomationReady(automationSocket, logs.cliDir)
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
        await waitForScreenMatch(screenA, logs.renderedA, new RegExp(expected), 90_000)
      }
      for (const expected of [markers.arrobaB, markers.nativeB]) {
        await waitForScreenMatch(screenB, logs.renderedB, new RegExp(expected), 90_000)
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
      await waitForScreenMatch(screenB, logs.renderedB, new RegExp(markers.nativeB), 90_000)
      await waitForScreenMatch(screenB, logs.renderedB, new RegExp(markers.arrobaB), 90_000)
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
        await waitForScreenMatch(screenA, logs.renderedA, new RegExp(markers.nativeSkill), 90_000)
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
        await waitForScreenMatch(screenA, logs.renderedA, new RegExp(markers.arrobaSkill), 90_000)
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
        await waitForScreenMatch(screenA, logs.renderedA, new RegExp(markers.nativeSkill), 90_000)
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
        await waitForScreenMatch(screenA, logs.renderedA, new RegExp(markers.arrobaSkill), 90_000)
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
      await waitForScreenMatch(screenA, logs.renderedA, new RegExp(markers.nativeAttachment), 60_000)
      if (provider === "claude") {
        await waitForScreenMatch(screenA, logs.renderedA, /native-attach/, 60_000)
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
      await waitForScreenMatch(screenA, logs.renderedA, new RegExp(markers.arrobaAttachment), 60_000)
      if (provider === "claude") {
        await waitForScreenMatch(screenA, logs.renderedA, /arroba-attach/, 60_000)
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
    await screenQuit(screenA)
    await screenQuit(screenB)
    await screenQuit(screenCli)
    await terminateMatchingProcesses([
      relayToken,
      automationSocket,
      screenA,
      screenB,
      screenCli,
      `remote-native-${provider}-${process.pid}`,
      `arroba-remote-native-observer-${provider}-${process.pid}`,
    ])
    if (sessionId) {
      let ended = false
      if (client) {
        ended = await client.send(endSessionRequest(sessionId)).then(() => true).catch(() => false)
      }
      if (!ended) {
        const cleanupClient = relayClient(relayUrl, relayToken, targetDaemonAlias)
        await cleanupClient.send(endSessionRequest(sessionId)).catch((error) => {
          console.error(`remote native TUI session cleanup failed: ${error.message}`)
        })
        await cleanupClient.close().catch(() => {})
      }
    }
    if (client) await client.close().catch(() => {})
    await rm(automationSocket, { force: true }).catch(() => {})
  }
}
