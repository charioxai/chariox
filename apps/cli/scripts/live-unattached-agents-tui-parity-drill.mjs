#!/usr/bin/env node
import assert from "node:assert/strict"
import { randomUUID } from "node:crypto"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import { createUnattachedAgentsTuiParityDrillRuntime } from "./lib/live-unattached-agents-tui-parity-runtime.mjs"

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const {
  createAutomationClient,
  ensureBuilt,
  makePorts,
  nowStamp,
  refreshExternalProviderSessionsRequest,
  renderTerminalScreenshot,
  stopChild,
  unwrap,
  waitForCondition,
  waitForKernel,
  waitForSnapshot,
  waitForSocket,
} = createUnattachedAgentsTuiParityDrillRuntime({ repoRoot, cliRoot })

async function seedOpenCodeSession(home, id, title, marker, workspace) {
  const file = path.join(home, "sessions", `${id}-session.json`)
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, JSON.stringify({
    id,
    title,
    cwd: workspace,
    updatedAt: new Date().toISOString(),
    messages: [
      {
        id: `${id}-user-1`,
        role: "user",
        content: `${title} external prompt ${marker}.`,
        createdAt: "2026-06-09T12:00:01.000Z",
      },
      {
        id: `${id}-assistant-1`,
        role: "assistant",
        content: `${title} observed reply ${marker}.`,
        createdAt: "2026-06-09T12:00:02.000Z",
      },
    ],
  }, null, 2))
  return file
}

async function seedCodexSession(home, id, title, marker, workspace) {
  const file = path.join(home, "sessions", `${id}.jsonl`)
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, [
    JSON.stringify({
      timestamp: "2026-06-09T12:10:00.000Z",
      type: "session_meta",
      payload: { id, cwd: workspace, model_provider: "openai" },
    }),
    JSON.stringify({
      timestamp: "2026-06-09T12:10:01.000Z",
      type: "response_item",
      payload: {
        id: `${id}-user-1`,
        type: "message",
        role: "user",
        content: [{ type: "input_text", text: `${title} external prompt ${marker}.` }],
      },
    }),
    JSON.stringify({
      timestamp: "2026-06-09T12:10:02.000Z",
      type: "response_item",
      payload: {
        id: `${id}-assistant-1`,
        type: "message",
        role: "assistant",
        content: [{ type: "output_text", text: `${title} observed reply ${marker}.` }],
      },
    }),
    "",
  ].join("\n"))
  return file
}

async function seedClaudeSession(home, id, title, marker, workspace) {
  const file = path.join(home, "projects", "unattached-tui", `${id}.jsonl`)
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, [
    JSON.stringify({
      sessionId: id,
      cwd: workspace,
      timestamp: "2026-06-09T12:20:01.000Z",
      type: "user",
      uuid: `${id}-user-1`,
      message: { content: `${title} external prompt ${marker}.` },
    }),
    JSON.stringify({
      sessionId: id,
      cwd: workspace,
      timestamp: "2026-06-09T12:20:02.000Z",
      type: "assistant",
      uuid: `${id}-assistant-1`,
      message: {
        id: `${id}-assistant-1`,
        content: `${title} observed reply ${marker}.`,
      },
    }),
    "",
  ].join("\n"))
  return file
}

async function appendOpenCodeTurn(file, id, role, text) {
  const payload = JSON.parse(await readFile(file, "utf8"))
  payload.updatedAt = new Date().toISOString()
  payload.messages.push({
    id,
    role,
    content: text,
    createdAt: new Date().toISOString(),
  })
  await writeFile(file, JSON.stringify(payload, null, 2))
  return { providerTurnId: id, text }
}

async function appendOpenCodeCompletionMetadata(file, id) {
  const payload = JSON.parse(await readFile(file, "utf8"))
  payload.updatedAt = new Date().toISOString()
  payload.messages.push({
    info: {
      id,
      role: "assistant",
      providerID: "moonshot",
      modelID: "kimi-k2.6",
      finish: "stop",
      tokens: { input: 12, output: 6, reasoning: 2 },
      time: { completed: Date.now() },
    },
    parts: [],
  })
  await writeFile(file, JSON.stringify(payload, null, 2))
  return { providerTurnId: `message-status-${id}`, text: "opencode message completed" }
}

async function appendCodexTurn(file, id, role, text) {
  const line = JSON.stringify({
    timestamp: new Date().toISOString(),
    type: "response_item",
    payload: {
      id,
      type: "message",
      role,
      content: [{ type: role === "user" ? "input_text" : "output_text", text }],
    },
  })
  await writeFile(file, `${await readFile(file, "utf8")}${line}\n`)
  return { providerTurnId: id, text }
}

async function appendCodexCompletionMetadata(file, marker) {
  const tokenLine = JSON.stringify({
    timestamp: new Date().toISOString(),
    type: "event_msg",
    payload: {
      type: "token_count",
      turn_id: "codex-tui-queue-token-metadata",
      info: {
        total_token_usage: {
          input_tokens: 123,
          output_tokens: 45,
          reasoning_output_tokens: 6,
          total_tokens: 174,
        },
      },
      rate_limits: {
        limit_id: "codex",
        used_percent: 1,
      },
    },
  })
  const completionLine = JSON.stringify({
    timestamp: new Date().toISOString(),
    type: "event_msg",
    payload: {
      type: "task_complete",
      turn_id: "codex-tui-queue-completion-metadata",
      completed_at: Math.floor(Date.now() / 1000),
      duration_ms: 1234,
      last_agent_message: `codex external output releases queued prompt in TUI ${marker}`,
      time_to_first_token_ms: 100,
    },
  })
  await writeFile(file, `${await readFile(file, "utf8")}${tokenLine}\n${completionLine}\n`)
  return { providerTurnId: "task_complete-codex-tui-queue-completion-metadata", text: "codex task_complete" }
}

async function appendClaudeTurn(file, sessionId, id, role, text, workspace) {
  const line = JSON.stringify({
    sessionId,
    cwd: workspace,
    timestamp: new Date().toISOString(),
    type: role,
    uuid: id,
    message: {
      id,
      content: text,
    },
  })
  await writeFile(file, `${await readFile(file, "utf8")}${line}\n`)
  return { providerTurnId: id, text }
}

function allTranscriptEntries(snapshot) {
  const entries = []
  if (Array.isArray(snapshot?.transcript?.entries)) entries.push(...snapshot.transcript.entries)
  for (const paneEntries of Object.values(snapshot?.agentPanes ?? {})) {
    if (Array.isArray(paneEntries)) entries.push(...paneEntries)
  }
  return entries
}

function entriesForText(snapshot, text) {
  return allTranscriptEntries(snapshot).filter((entry) => String(entry?.text ?? "").includes(text))
}

function hasExternalMetadata(snapshot, provider, providerTurnId) {
  return allTranscriptEntries(snapshot).some((entry) => (
    entry?.source === "external_provider_observed"
    && entry?.externalProvider === provider
    && entry?.externalProviderTurnId === providerTurnId
    && typeof entry?.observedAtMs === "number"
  ))
}

function hasQueuedPromptSteerDisabled(snapshot, promptText = null) {
  return allTranscriptEntries(snapshot).some((entry) => (
    entry?.queuedPrompt?.steerDisabled === true
    && (!promptText || String(entry?.text ?? "").includes(promptText))
  ))
}

function agentForExternal(session, externalSessionId) {
  return (session?.agents ?? []).find((agent) => agent?.external_provider_import?.external_provider_session_id === externalSessionId)
}

function promptStateForAgent(session, agentId) {
  const state = session?.prompt_states?.[agentId]
  return state && typeof state === "object" ? state : null
}

async function main() {
  const stamp = nowStamp()
  const marker = `CHARIOX_UNATTACHED_TUI_${stamp}_${process.pid}`
  const artifactRoot = path.join(repoRoot, ".artifacts", "unattached-agents-tui-parity", stamp)
  const runtimeRoot = path.join(os.tmpdir(), `chariox-unattached-tui-${process.pid}-${Date.now()}`)
  const workspace = repoRoot
  const automationSocket = path.join(os.tmpdir(), `chariox-utui-${process.pid}.sock`)
  const ports = await makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const opencodeHome = path.join(runtimeRoot, "provider-homes", "opencode")
  const codexHome = path.join(runtimeRoot, "provider-homes", "codex")
  const claudeHome = path.join(runtimeRoot, "provider-homes", "claude")
  const waitingProviderSessionId = `opencode-waiting-${marker}`
  const attachProviderSessionId = `opencode-attach-${marker}`
  const codexProviderSessionId = randomUUID()
  const claudeProviderSessionId = randomUUID()
  const waitingExternalSessionId = `opencode:${waitingProviderSessionId}`
  const attachExternalSessionId = `opencode:${attachProviderSessionId}`
  const codexExternalSessionId = `codex:${codexProviderSessionId}`
  const claudeExternalSessionId = `claude:${claudeProviderSessionId}`
  const evidence = []
  let daemon = null
  let cli = null
  let automation = null
  let client = null
  let failure = null
  let cliStdout = ""
  let cliStderr = ""
  let daemonStdout = ""
  let daemonStderr = ""
  try {
    await prepareDrillArtifacts(artifactRoot)
    await mkdir(runtimeRoot, { recursive: true })
    const { kernelBinary, cliDist } = await ensureBuilt()
    const waitingFile = await seedOpenCodeSession(opencodeHome, waitingProviderSessionId, "OpenCode waiting-room external", marker, workspace)
    const attachFile = await seedOpenCodeSession(opencodeHome, attachProviderSessionId, "OpenCode attach external", marker, workspace)
    const codexFile = await seedCodexSession(codexHome, codexProviderSessionId, "Codex attach external", marker, workspace)
    const claudeFile = await seedClaudeSession(claudeHome, claudeProviderSessionId, "Claude attach external", marker, workspace)
    assert.ok(waitingFile && attachFile && codexFile && claudeFile)

    const env = {
      ...process.env,
      HOME: path.join(runtimeRoot, "home"),
      XDG_CONFIG_HOME: path.join(runtimeRoot, "config"),
      XDG_STATE_HOME: path.join(runtimeRoot, "state"),
      CODEX_HOME: codexHome,
      CLAUDE_HOME: claudeHome,
      OPENCODE_DATA_HOME: opencodeHome,
      CHARIOX_KERNEL_PORT: String(ports.kernelPort),
      CHARIOX_MCP_PORT: String(ports.mcpPort),
      CHARIOX_OPENCODE_PORT: String(ports.opencodePort),
      CHARIOX_CODEX_PORT: String(ports.codexPort),
      CHARIOX_DAEMON_ID: `unattached-tui-drill-${process.pid}`,
      CHARIOX_DAEMON_SOCKET: path.join(runtimeRoot, "daemon.sock"),
      CHARIOX_SESSION_HISTORY_DIR: path.join(runtimeRoot, "history"),
    }

    const { LocalIpcClient } = await import("../../../packages/kernel-client/dist/ipc.js")
    const requests = await import("../../../packages/kernel-client/dist/ipc-requests.js")
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ["ignore", "pipe", "pipe"] })
    daemon.stdout.on("data", (chunk) => { daemonStdout += chunk.toString() })
    daemon.stderr.on("data", (chunk) => { daemonStderr += chunk.toString() })
    await waitForKernel(LocalIpcClient, requests.listSessionsRequest, kernelUrl)
    client = new LocalIpcClient(kernelUrl)
    const directRefresh = unwrap(
      await client.send(refreshExternalProviderSessionsRequest()),
      "ExternalProviderSessionsRefreshed",
    ).page
    const directIds = (directRefresh?.sessions ?? []).map((session) => session.external_session_id)
    assert.ok(
      directIds.includes(waitingExternalSessionId)
        && directIds.includes(attachExternalSessionId)
        && directIds.includes(codexExternalSessionId)
        && directIds.includes(claudeExternalSessionId),
      `kernel direct external-provider refresh did not discover seeded provider sessions: ${directIds.join(", ")}`,
    )
    const publicSnapshot = unwrap(
      await client.send(requests.getWaitingRoomPublicSnapshotRequest()),
      "WaitingRoomPublicSnapshot",
    ).snapshot
    const publicIds = (publicSnapshot?.external_provider_sessions ?? []).map((session) => session.external_session_id)
    assert.ok(
      publicIds.includes(waitingExternalSessionId)
        && publicIds.includes(attachExternalSessionId)
        && publicIds.includes(codexExternalSessionId)
        && publicIds.includes(claudeExternalSessionId),
      `kernel waiting-room public snapshot did not include refreshed unattached agents: ${publicIds.join(", ")}`,
    )

    const cliArgs = [
      "-q",
      "/dev/null",
      "env",
      ...Object.entries(env).map(([key, value]) => `${key}=${value}`),
      "bun",
      cliDist,
      "--kernel-url", kernelUrl,
      "--automation-socket", automationSocket,
      "--workspace", workspace,
      "--worktree", workspace,
      "--provider", "dev-stub",
      "--model", "unattached-tui-drill-model",
      "--client-id", `unattached-tui-drill-${process.pid}`,
    ]
    cli = spawn("script", cliArgs, { cwd: repoRoot, env, stdio: ["ignore", "pipe", "pipe"] })
    cli.stdout.on("data", (chunk) => { cliStdout += chunk.toString() })
    cli.stderr.on("data", (chunk) => { cliStderr += chunk.toString() })
    const cliStartupFailure = new Promise((resolve) => {
      cli.once("error", (error) => resolve(error))
      cli.once("exit", (code, signal) => {
        if (code !== 0) resolve(new Error(`CLI exited before automation socket was ready: code=${code} signal=${signal ?? "none"}`))
      })
    })
    const startupFailure = await Promise.race([
      waitForSocket(automationSocket).then(() => null),
      cliStartupFailure,
    ])
    if (startupFailure) throw startupFailure
    automation = createAutomationClient(automationSocket)
    await automation.send("ping")
    await automation.send("connect_detached_kernel")

    const waitingRoomSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => {
        const rows = snapshot?.waitingRoom?.rows ?? []
        return rows.some((row) => row.externalSessionId === waitingExternalSessionId)
          && rows.some((row) => row.externalSessionId === attachExternalSessionId)
          && rows.some((row) => row.externalSessionId === codexExternalSessionId)
          && rows.some((row) => row.externalSessionId === claudeExternalSessionId)
      },
      "detached TUI waiting room external rows",
      60_000,
    )
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "01-tui-waiting-room-external-agents.png", "TUI Waiting Room External Agents", [
      `PASS waiting-room row visible: ${waitingExternalSessionId}`,
      `PASS attach row visible: ${attachExternalSessionId}`,
      `PASS codex row visible: ${codexExternalSessionId}`,
      `PASS claude row visible: ${claudeExternalSessionId}`,
      `rows=${(waitingRoomSnapshot.waitingRoom?.rows ?? []).filter((row) => row.externalSessionId).map((row) => row.externalSessionId).join(", ")}`,
    ])))

    const importedSnapshot = await automation.send("activate_unattached_agent", { externalSessionId: waitingExternalSessionId })
    const waitingSessionId = importedSnapshot?.session?.id
    assert.ok(waitingSessionId, "TUI should attach to a new session after activating an unattached agent")
    await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, `OpenCode waiting-room external observed reply ${marker}.`).length > 0,
      "waiting-room imported external history",
      60_000,
    )
    let waitingSession = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
    const waitingAgent = agentForExternal(waitingSession, waitingExternalSessionId)
    assert.ok(waitingAgent, "waiting-room imported session should include external-provider import metadata")
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "02-tui-waiting-room-external-imported.png", "TUI External Imported As Session", [
      `PASS session=${waitingSessionId}`,
      `PASS imported agent=${waitingAgent.id}`,
      `PASS external import=${waitingExternalSessionId}`,
      `PASS observed history visible in TUI snapshot`,
    ])))

    await automation.send("submit_prompt", { prompt: `/agent spawn --unattached-agent ${attachExternalSessionId}` })
    const attachedSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => Number(snapshot?.session?.agentCount ?? 0) >= 2
        && entriesForText(snapshot, `OpenCode attach external observed reply ${marker}.`).length > 0,
      "attached agent through TUI slash command",
      60_000,
    )
    waitingSession = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
    const attachAgent = agentForExternal(waitingSession, attachExternalSessionId)
    assert.ok(attachAgent, "spawned unattached agent should attach to the existing Chariox session")
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "03-tui-spawn-external-agent.png", "TUI Spawn External Agent", [
      `PASS session agent count=${attachedSnapshot.session?.agentCount}`,
      `PASS attached agent=${attachAgent.id}`,
      `PASS external import=${attachExternalSessionId}`,
      `PASS observed attach history visible`,
    ])))

    await automation.send("submit_prompt", { prompt: `/agent spawn --unattached-agent ${codexExternalSessionId}` })
    await waitForSnapshot(
      automation,
      (snapshot) => Number(snapshot?.session?.agentCount ?? 0) >= 3
        && entriesForText(snapshot, `Codex attach external observed reply ${marker}.`).length > 0,
      "attached Codex agent through TUI slash command",
      60_000,
    )
    await automation.send("submit_prompt", { prompt: `/agent spawn --unattached-agent ${claudeExternalSessionId}` })
    const providerMatrixAttachedSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => Number(snapshot?.session?.agentCount ?? 0) >= 4
        && entriesForText(snapshot, `Claude attach external observed reply ${marker}.`).length > 0,
      "attached Claude agent through TUI slash command",
      60_000,
    )
    waitingSession = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
    const codexAgent = agentForExternal(waitingSession, codexExternalSessionId)
    const claudeAgent = agentForExternal(waitingSession, claudeExternalSessionId)
    assert.ok(codexAgent, "Codex unattached agent should attach to the existing Chariox session")
    assert.ok(claudeAgent, "Claude unattached agent should attach to the existing Chariox session")
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "04-tui-provider-matrix-external-agents-attached.png", "TUI Provider Matrix External Agents Attached", [
      `PASS session agent count=${providerMatrixAttachedSnapshot.session?.agentCount}`,
      `PASS opencode agent=${attachAgent.id}`,
      `PASS codex agent=${codexAgent.id}`,
      `PASS claude agent=${claudeAgent.id}`,
    ])))
    await automation.send("submit_prompt", { prompt: `/agent focus ${attachAgent.id}` })
    await waitForSnapshot(
      automation,
      (snapshot) => snapshot?.session?.focusedAgentId === attachAgent.id,
      "focused OpenCode agent before external queue guard",
      30_000,
    )

    const userTurn = await appendOpenCodeTurn(
      attachFile,
      "opencode-user-2",
      "user",
      `opencode external prompt visible live in TUI ${marker}`,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("opencode")), "ExternalProviderSessionsRefreshed")
    const userMetadataSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, userTurn.text).length > 0 && hasExternalMetadata(snapshot, "opencode", userTurn.providerTurnId),
      "external user turn metadata in TUI",
      30_000,
    )
    await waitForCondition(async () => {
      const session = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
      const active = promptStateForAgent(session, attachAgent.id)?.active_prompt
      return active?.prompt_origin === "external" && String(active.id ?? "").includes(userTurn.providerTurnId)
        ? active
        : false
    }, "kernel external active prompt for TUI attached agent", 30_000)
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "05-tui-opencode-external-live-metadata.png", "TUI OpenCode External Turn Metadata", [
      `PASS source=external_provider_observed`,
      `PASS provider=opencode`,
      `PASS providerTurnId=${userTurn.providerTurnId}`,
      `PASS observedAtMs=${entriesForText(userMetadataSnapshot, userTurn.text)[0]?.observedAtMs ?? "recorded"}`,
    ])))

    const assistantTurn = await appendOpenCodeTurn(
      attachFile,
      "opencode-assistant-2",
      "assistant",
      `opencode external output visible live in TUI ${marker}`,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("opencode")), "ExternalProviderSessionsRefreshed")
    await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, assistantTurn.text).length > 0 && hasExternalMetadata(snapshot, "opencode", userTurn.providerTurnId),
      "external assistant turn metadata in TUI",
      30_000,
    )

    const queuedGuardUserTurn = await appendOpenCodeTurn(
      attachFile,
      "opencode-user-3",
      "user",
      `opencode external prompt queues chariox input in TUI ${marker}`,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("opencode")), "ExternalProviderSessionsRefreshed")
    await waitForCondition(async () => {
      const session = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
      const active = promptStateForAgent(session, attachAgent.id)?.active_prompt
      return active?.prompt_origin === "external" && String(active.id ?? "").includes(queuedGuardUserTurn.providerTurnId)
        ? active
        : false
    }, "kernel external active prompt for TUI queue guard", 30_000)

    const queuedPromptText = `chariox prompt queued behind external TUI turn ${marker}`
    await automation.send("submit_prompt", { prompt: queuedPromptText })
    const queuedSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => hasQueuedPromptSteerDisabled(snapshot, queuedPromptText),
      "TUI queued prompt with steering disabled",
      30_000,
    )
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "06-tui-external-active-queued-steer-disabled.png", "TUI External Active Queue Guard", [
      `PASS queued prompt=${queuedPromptText}`,
      `PASS queuedPrompt.steerDisabled=true`,
      `PASS queued entries=${allTranscriptEntries(queuedSnapshot).filter((entry) => entry?.queuedPrompt).length}`,
    ])))

    const settlingAssistantTurn = await appendOpenCodeTurn(
      attachFile,
      "opencode-assistant-3",
      "assistant",
      `opencode external output releases queued prompt in TUI ${marker}`,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("opencode")), "ExternalProviderSessionsRefreshed")
    await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, settlingAssistantTurn.text).length > 0 && hasExternalMetadata(snapshot, "opencode", queuedGuardUserTurn.providerTurnId),
      "external assistant settling turn metadata in TUI",
      30_000,
    )
    const openCodeCompletion = await appendOpenCodeCompletionMetadata(attachFile, "opencode-completion-3")
    unwrap(await client.send(refreshExternalProviderSessionsRequest("opencode")), "ExternalProviderSessionsRefreshed")
    await waitForCondition(async () => {
      const session = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
      const state = promptStateForAgent(session, attachAgent.id)
      const active = state?.active_prompt
      const queued = Array.isArray(state?.queued_prompts) ? state.queued_prompts : []
      return active?.prompt_origin !== "external" && !queued.some((prompt) => prompt?.prompt === queuedPromptText)
        ? { active, queued }
        : false
    }, "TUI queued prompt drained after external turn settled", 60_000)
    const finalSnapshot = await automation.send("snapshot")
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "07-tui-opencode-external-output-queue-drained.png", "TUI OpenCode External Output And Queue Drain", [
      `PASS providerTurnId=${settlingAssistantTurn.providerTurnId}`,
      `PASS completion=${openCodeCompletion.providerTurnId}`,
      `PASS external assistant output visible`,
      `PASS queued prompt drained`,
      `PASS total transcript entries=${allTranscriptEntries(finalSnapshot).length}`,
    ])))

    const codexUserTurn = await appendCodexTurn(
      codexFile,
      "codex-user-2",
      "user",
      `codex external prompt visible live in TUI ${marker}`,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("codex")), "ExternalProviderSessionsRefreshed")
    const codexUserSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, codexUserTurn.text).length > 0 && hasExternalMetadata(snapshot, "codex", codexUserTurn.providerTurnId),
      "Codex external user turn metadata in TUI",
      30_000,
    )
    const codexAssistantTurn = await appendCodexTurn(
      codexFile,
      "codex-assistant-2",
      "assistant",
      `codex external output visible live in TUI ${marker}`,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("codex")), "ExternalProviderSessionsRefreshed")
    await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, codexAssistantTurn.text).length > 0 && hasExternalMetadata(snapshot, "codex", codexUserTurn.providerTurnId),
      "Codex external assistant turn metadata in TUI",
      30_000,
    )
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "08-tui-codex-external-live-metadata.png", "TUI Codex External Metadata", [
      `PASS source=external_provider_observed`,
      `PASS provider=codex`,
      `PASS user providerTurnId=${codexUserTurn.providerTurnId}`,
      `PASS assistant providerTurnId=${codexAssistantTurn.providerTurnId}`,
      `PASS observedAtMs=${entriesForText(codexUserSnapshot, codexUserTurn.text)[0]?.observedAtMs ?? "recorded"}`,
    ])))

    await automation.send("submit_prompt", { prompt: `/agent focus ${codexAgent.id}` })
    await waitForSnapshot(
      automation,
      (snapshot) => snapshot?.session?.focusedAgentId === codexAgent.id,
      "focused Codex agent before external queue guard",
      30_000,
    )
    const codexQueuedGuardUserTurn = await appendCodexTurn(
      codexFile,
      "codex-user-3",
      "user",
      `codex external prompt queues chariox input in TUI ${marker}`,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("codex")), "ExternalProviderSessionsRefreshed")
    await waitForCondition(async () => {
      const session = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
      const active = promptStateForAgent(session, codexAgent.id)?.active_prompt
      return active?.prompt_origin === "external" && String(active.id ?? "").includes(codexQueuedGuardUserTurn.providerTurnId)
        ? active
        : false
    }, "kernel external active prompt for Codex TUI queue guard", 30_000)

    const codexQueuedPromptText = `chariox prompt queued behind external Codex TUI turn ${marker}`
    await automation.send("submit_prompt", { prompt: codexQueuedPromptText })
    const codexQueuedSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => hasQueuedPromptSteerDisabled(snapshot, codexQueuedPromptText),
      "Codex TUI queued prompt with steering disabled",
      30_000,
    )
    const codexSettlingAssistantTurn = await appendCodexTurn(
      codexFile,
      "codex-assistant-3",
      "assistant",
      `codex external output releases queued prompt in TUI ${marker}`,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("codex")), "ExternalProviderSessionsRefreshed")
    await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, codexSettlingAssistantTurn.text).length > 0 && hasExternalMetadata(snapshot, "codex", codexQueuedGuardUserTurn.providerTurnId),
      "Codex external assistant settling turn metadata in TUI",
      30_000,
    )
    const codexCompletion = await appendCodexCompletionMetadata(codexFile, marker)
    unwrap(await client.send(refreshExternalProviderSessionsRequest("codex")), "ExternalProviderSessionsRefreshed")
    await waitForCondition(async () => {
      const session = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
      const state = promptStateForAgent(session, codexAgent.id)
      const active = state?.active_prompt
      const queued = Array.isArray(state?.queued_prompts) ? state.queued_prompts : []
      return active?.prompt_origin !== "external" && !queued.some((prompt) => prompt?.prompt === codexQueuedPromptText)
        ? { active, queued }
        : false
    }, "Codex TUI queued prompt drained after external turn settled", 60_000)
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "09-tui-codex-external-queue-drained.png", "TUI Codex External Queue Guard", [
      `PASS queued prompt=${codexQueuedPromptText}`,
      `PASS queuedPrompt.steerDisabled=${hasQueuedPromptSteerDisabled(codexQueuedSnapshot, codexQueuedPromptText)}`,
      `PASS providerTurnId=${codexSettlingAssistantTurn.providerTurnId}`,
      `PASS completion=${codexCompletion.providerTurnId}`,
      `PASS queued prompt drained`,
    ])))

    const claudeUserTurn = await appendClaudeTurn(
      claudeFile,
      claudeProviderSessionId,
      "claude-user-2",
      "user",
      `claude external prompt visible live in TUI ${marker}`,
      workspace,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("claude")), "ExternalProviderSessionsRefreshed")
    const claudeUserSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, claudeUserTurn.text).length > 0 && hasExternalMetadata(snapshot, "claude", claudeUserTurn.providerTurnId),
      "Claude external user turn metadata in TUI",
      30_000,
    )
    const claudeAssistantTurn = await appendClaudeTurn(
      claudeFile,
      claudeProviderSessionId,
      "claude-assistant-2",
      "assistant",
      `claude external output visible live in TUI ${marker}`,
      workspace,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("claude")), "ExternalProviderSessionsRefreshed")
    await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, claudeAssistantTurn.text).length > 0 && hasExternalMetadata(snapshot, "claude", claudeUserTurn.providerTurnId),
      "Claude external assistant turn metadata in TUI",
      30_000,
    )
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "09-tui-claude-external-live-metadata.png", "TUI Claude External Metadata", [
      `PASS source=external_provider_observed`,
      `PASS provider=claude`,
      `PASS user providerTurnId=${claudeUserTurn.providerTurnId}`,
      `PASS assistant providerTurnId=${claudeAssistantTurn.providerTurnId}`,
      `PASS observedAtMs=${entriesForText(claudeUserSnapshot, claudeUserTurn.text)[0]?.observedAtMs ?? "recorded"}`,
    ])))

    await automation.send("submit_prompt", { prompt: `/agent focus ${claudeAgent.id}` })
    await waitForSnapshot(
      automation,
      (snapshot) => snapshot?.session?.focusedAgentId === claudeAgent.id,
      "focused Claude agent before external queue guard",
      30_000,
    )
    const claudeQueuedGuardUserTurn = await appendClaudeTurn(
      claudeFile,
      claudeProviderSessionId,
      "claude-user-3",
      "user",
      `claude external prompt queues chariox input in TUI ${marker}`,
      workspace,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("claude")), "ExternalProviderSessionsRefreshed")
    await waitForCondition(async () => {
      const session = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
      const active = promptStateForAgent(session, claudeAgent.id)?.active_prompt
      return active?.prompt_origin === "external" && String(active.id ?? "").includes(claudeQueuedGuardUserTurn.providerTurnId)
        ? active
        : false
    }, "kernel external active prompt for Claude TUI queue guard", 30_000)

    const claudeQueuedPromptText = `chariox prompt queued behind external Claude TUI turn ${marker}`
    await automation.send("submit_prompt", { prompt: claudeQueuedPromptText })
    const claudeQueuedSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => hasQueuedPromptSteerDisabled(snapshot, claudeQueuedPromptText),
      "Claude TUI queued prompt with steering disabled",
      30_000,
    )
    const claudeSettlingAssistantTurn = await appendClaudeTurn(
      claudeFile,
      claudeProviderSessionId,
      "claude-assistant-3",
      "assistant",
      `claude external output releases queued prompt in TUI ${marker}`,
      workspace,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("claude")), "ExternalProviderSessionsRefreshed")
    await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, claudeSettlingAssistantTurn.text).length > 0 && hasExternalMetadata(snapshot, "claude", claudeQueuedGuardUserTurn.providerTurnId),
      "Claude external assistant settling turn metadata in TUI",
      30_000,
    )
    // Claude has no mandatory completion sentinel, so the kernel waits for the
    // assistant observation to be stable across a subsequent scanner pass.
    unwrap(await client.send(refreshExternalProviderSessionsRequest("claude")), "ExternalProviderSessionsRefreshed")
    await waitForCondition(async () => {
      const session = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
      const state = promptStateForAgent(session, claudeAgent.id)
      const active = state?.active_prompt
      const queued = Array.isArray(state?.queued_prompts) ? state.queued_prompts : []
      return active?.prompt_origin !== "external" && !queued.some((prompt) => prompt?.prompt === claudeQueuedPromptText)
        ? { active, queued }
        : false
    }, "Claude TUI queued prompt drained after external turn settled", 60_000)
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "10-tui-claude-external-queue-drained.png", "TUI Claude External Queue Guard", [
      `PASS queued prompt=${claudeQueuedPromptText}`,
      `PASS queuedPrompt.steerDisabled=${hasQueuedPromptSteerDisabled(claudeQueuedSnapshot, claudeQueuedPromptText)}`,
      `PASS providerTurnId=${claudeSettlingAssistantTurn.providerTurnId}`,
      `PASS queued prompt drained`,
    ])))

    const manifest = {
      ok: true,
      drill: "unattached-agents-tui-parity",
      marker,
      kernelUrl,
      waitingExternalSessionId,
      attachExternalSessionId,
      codexExternalSessionId,
      claudeExternalSessionId,
      waitingSessionId,
      waitingAgentId: waitingAgent.id,
      attachAgentId: attachAgent.id,
      codexAgentId: codexAgent.id,
      claudeAgentId: claudeAgent.id,
      queuedPromptTexts: {
        opencode: queuedPromptText,
        codex: codexQueuedPromptText,
        claude: claudeQueuedPromptText,
      },
      evidence,
    }
    await writeFile(path.join(artifactRoot, "manifest.json"), JSON.stringify(manifest, null, 2))
    console.log(JSON.stringify(manifest, null, 2))
    await automation.send("exit").catch(() => {})
  } catch (error) {
    failure = error
    throw error
  } finally {
    await client?.close().catch(() => {})
    automation?.close()
    await stopChild(cli)
    await stopChild(daemon)
    if (failure) {
      await finalizeDrillArtifacts({
        rootDir: artifactRoot,
        passed: false,
        preserveOnFailure: true,
        failure,
        metadata: {
          drill: "unattached-agents-tui-parity",
          marker,
          kernelUrl,
          runtimeRoot,
          cliStdoutTail: cliStdout.slice(-4000),
          cliStderrTail: cliStderr.slice(-4000),
          daemonStdoutTail: daemonStdout.slice(-4000),
          daemonStderrTail: daemonStderr.slice(-4000),
        },
      })
    } else {
      await rm(runtimeRoot, { recursive: true, force: true }).catch(() => {})
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
