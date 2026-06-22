#!/usr/bin/env node
import assert from "node:assert/strict"
import { spawn } from "node:child_process"
import net from "node:net"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"

const cliRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function nowStamp() {
  return new Date().toISOString().replace(/[-:]/g, "").replace(/\.\d{3}Z$/, "Z")
}

function makePorts() {
  const kernelPort = 55000 + Math.floor(Math.random() * 1000)
  return {
    kernelPort,
    mcpPort: kernelPort + 1000,
    opencodePort: kernelPort + 2000,
    codexPort: kernelPort + 2001,
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

async function ensureBuilt() {
  const kernel = await run("cargo", ["build", "--manifest-path", path.join(repoRoot, "apps/kernel/Cargo.toml"), "--bin", "arroba-kernel"])
  if (kernel.code !== 0) throw new Error(`kernel build failed\n${kernel.stdout}\n${kernel.stderr}`)
  const kernelClient = await run("pnpm", ["--filter", "@arroba/kernel-client", "run", "build"])
  if (kernelClient.code !== 0) throw new Error(`kernel-client build failed\n${kernelClient.stdout}\n${kernelClient.stderr}`)
  const toolDisplay = await run("pnpm", ["--filter", "@arroba/tool-display", "run", "build"])
  if (toolDisplay.code !== 0) throw new Error(`tool-display build failed\n${toolDisplay.stdout}\n${toolDisplay.stderr}`)
  const cli = await run("node", [path.join(cliRoot, "scripts/build.mjs")])
  if (cli.code !== 0) throw new Error(`cli build failed\n${cli.stdout}\n${cli.stderr}`)
  return {
    kernelBinary: path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel"),
    cliDist: path.join(cliRoot, "dist/index.js"),
  }
}

async function waitForKernel(LocalIpcClient, listSessionsRequest, kernelUrl) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
}

async function waitForSocket(socketPath) {
  const deadline = Date.now() + 20_000
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const socket = net.createConnection(socketPath)
      await new Promise((resolve, reject) => {
        socket.once("connect", resolve)
        socket.once("error", reject)
      })
      socket.destroy()
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
      const request = { id, action, ...fields }
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(`${JSON.stringify(request)}\n`)
      })
    },
    close() {
      socket.destroy()
    },
  }
}

async function waitForSnapshot(automation, predicate, label, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await automation.send("snapshot")
    if (predicate(last)) return last
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${label}\nlast snapshot:\n${JSON.stringify(last, null, 2)}`)
}

async function waitForCondition(predicate, label, timeoutMs = 30_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await predicate()
    if (last) return last
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${label}\nlast value:\n${JSON.stringify(last, null, 2)}`)
}

async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill("SIGTERM")
  const exited = await Promise.race([
    new Promise((resolve) => child.once("exit", () => resolve(true))),
    sleep(5_000).then(() => false),
  ])
  if (!exited && child.exitCode === null && child.signalCode === null) {
    child.kill("SIGKILL")
    await Promise.race([
      new Promise((resolve) => child.once("exit", resolve)),
      sleep(2_000),
    ])
  }
}

function unwrap(response, ...keys) {
  for (const key of keys) {
    if (response?.[key]) return response[key]
  }
  return response
}

function refreshExternalProviderSessionsRequest(provider = null) {
  return { RefreshExternalProviderSessions: { provider } }
}

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

function hasExternalMetadata(snapshot, providerTurnId) {
  return allTranscriptEntries(snapshot).some((entry) => (
    entry?.source === "external_provider_observed"
    && entry?.externalProvider === "opencode"
    && entry?.externalProviderTurnId === providerTurnId
    && typeof entry?.observedAtMs === "number"
  ))
}

function hasQueuedPromptSteerDisabled(snapshot) {
  return allTranscriptEntries(snapshot).some((entry) => entry?.queuedPrompt?.steerDisabled === true)
}

function agentForExternal(session, externalSessionId) {
  return (session?.agents ?? []).find((agent) => agent?.external_provider_import?.external_provider_session_id === externalSessionId)
}

function promptStateForAgent(session, agentId) {
  const state = session?.prompt_states?.[agentId]
  return state && typeof state === "object" ? state : null
}

async function renderTerminalScreenshot(artifactsDir, fileName, title, lines) {
  await mkdir(artifactsDir, { recursive: true })
  const width = 1280
  const height = Math.max(280, 104 + lines.length * 28)
  const svgPath = path.join(artifactsDir, `${fileName}.svg`)
  const pngPath = path.join(artifactsDir, fileName)
  const escaped = (value) => String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
  const body = lines.map((line, index) => (
    `<text x="48" y="${116 + index * 28}" fill="${line.startsWith("PASS") ? "#8ef0a5" : "#d9e2ec"}" font-size="20">${escaped(line)}</text>`
  )).join("\n")
  await writeFile(svgPath, `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">
<rect width="100%" height="100%" fill="#101820"/>
<rect x="28" y="28" width="${width - 56}" height="${height - 56}" rx="8" fill="#141f2b" stroke="#3b5269"/>
<text x="48" y="72" fill="#ffffff" font-family="Menlo, Consolas, monospace" font-size="24" font-weight="700">${escaped(title)}</text>
<g font-family="Menlo, Consolas, monospace">${body}</g>
</svg>`, "utf8")
  const result = await run("sips", ["-s", "format", "png", svgPath, "--out", pngPath])
  if (result.code !== 0) throw new Error(`failed to render screenshot ${fileName}: ${result.stdout}\n${result.stderr}`)
  await rm(svgPath, { force: true })
  return pngPath
}

async function main() {
  const stamp = nowStamp()
  const marker = `ARROBA_ORPHAN_TUI_${stamp}_${process.pid}`
  const artifactRoot = path.join(repoRoot, ".artifacts", "orphan-agents-tui-parity", stamp)
  const runtimeRoot = path.join(os.tmpdir(), `arroba-orphan-tui-${process.pid}-${Date.now()}`)
  const workspace = repoRoot
  const automationSocket = path.join(runtimeRoot, "automation.sock")
  const ports = makePorts()
  const kernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const opencodeHome = path.join(runtimeRoot, "provider-homes", "opencode")
  const waitingProviderSessionId = `opencode-waiting-${marker}`
  const attachProviderSessionId = `opencode-attach-${marker}`
  const waitingExternalSessionId = `opencode:${waitingProviderSessionId}`
  const attachExternalSessionId = `opencode:${attachProviderSessionId}`
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
    const waitingFile = await seedOpenCodeSession(opencodeHome, waitingProviderSessionId, "OpenCode waiting-room orphan", marker, workspace)
    const attachFile = await seedOpenCodeSession(opencodeHome, attachProviderSessionId, "OpenCode attach orphan", marker, workspace)
    assert.ok(waitingFile && attachFile)

    const env = {
      ...process.env,
      HOME: path.join(runtimeRoot, "home"),
      XDG_CONFIG_HOME: path.join(runtimeRoot, "config"),
      XDG_STATE_HOME: path.join(runtimeRoot, "state"),
      OPENCODE_DATA_HOME: opencodeHome,
      ARROBA_KERNEL_PORT: String(ports.kernelPort),
      ARROBA_MCP_PORT: String(ports.mcpPort),
      ARROBA_OPENCODE_PORT: String(ports.opencodePort),
      ARROBA_CODEX_PORT: String(ports.codexPort),
      ARROBA_DAEMON_ID: `orphan-tui-drill-${process.pid}`,
      ARROBA_DAEMON_SOCKET: path.join(runtimeRoot, "daemon.sock"),
      ARROBA_SESSION_HISTORY_DIR: path.join(runtimeRoot, "history"),
    }

    const { LocalIpcClient } = await import("../../../packages/kernel-client/dist/ipc.js")
    const requests = await import("../../../packages/kernel-client/dist/ipc-requests.js")
    daemon = spawn(kernelBinary, [], { cwd: repoRoot, env, stdio: ["ignore", "pipe", "pipe"] })
    daemon.stdout.on("data", (chunk) => { daemonStdout += chunk.toString() })
    daemon.stderr.on("data", (chunk) => { daemonStderr += chunk.toString() })
    await waitForKernel(LocalIpcClient, requests.listSessionsRequest, kernelUrl)
    client = new LocalIpcClient(kernelUrl)
    const directRefresh = unwrap(
      await client.send(refreshExternalProviderSessionsRequest("opencode")),
      "ExternalProviderSessionsRefreshed",
    ).page
    const directIds = (directRefresh?.sessions ?? []).map((session) => session.external_session_id)
    assert.ok(
      directIds.includes(waitingExternalSessionId) && directIds.includes(attachExternalSessionId),
      `kernel direct external-provider refresh did not discover seeded OpenCode sessions: ${directIds.join(", ")}`,
    )
    const publicSnapshot = unwrap(
      await client.send(requests.getWaitingRoomPublicSnapshotRequest()),
      "WaitingRoomPublicSnapshot",
    ).snapshot
    const publicIds = (publicSnapshot?.external_provider_sessions ?? []).map((session) => session.external_session_id)
    assert.ok(
      publicIds.includes(waitingExternalSessionId) && publicIds.includes(attachExternalSessionId),
      `kernel waiting-room public snapshot did not include refreshed orphan agents: ${publicIds.join(", ")}`,
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
      "--model", "orphan-tui-drill-model",
      "--client-id", `orphan-tui-drill-${process.pid}`,
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
      },
      "detached TUI waiting room orphan rows",
      60_000,
    )
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "01-tui-waiting-room-orphan-agents.png", "TUI Waiting Room Orphan Agents", [
      `PASS waiting-room row visible: ${waitingExternalSessionId}`,
      `PASS attach row visible: ${attachExternalSessionId}`,
      `rows=${(waitingRoomSnapshot.waitingRoom?.rows ?? []).filter((row) => row.externalSessionId).map((row) => row.externalSessionId).join(", ")}`,
    ])))

    const importedSnapshot = await automation.send("activate_orphan_agent", { externalSessionId: waitingExternalSessionId })
    const waitingSessionId = importedSnapshot?.session?.id
    assert.ok(waitingSessionId, "TUI should attach to a new session after activating an orphan agent")
    await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, `OpenCode waiting-room orphan observed reply ${marker}.`).length > 0,
      "waiting-room imported orphan history",
      60_000,
    )
    let waitingSession = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
    const waitingAgent = agentForExternal(waitingSession, waitingExternalSessionId)
    assert.ok(waitingAgent, "waiting-room imported session should include external-provider import metadata")
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "02-tui-waiting-room-orphan-imported.png", "TUI Orphan Imported As Session", [
      `PASS session=${waitingSessionId}`,
      `PASS imported agent=${waitingAgent.id}`,
      `PASS external import=${waitingExternalSessionId}`,
      `PASS observed history visible in TUI snapshot`,
    ])))

    await automation.send("submit_prompt", { prompt: `/agent spawn --orphan-agent ${attachExternalSessionId}` })
    const attachedSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => Number(snapshot?.session?.agentCount ?? 0) >= 2
        && entriesForText(snapshot, `OpenCode attach orphan observed reply ${marker}.`).length > 0,
      "attached orphan agent through TUI slash command",
      60_000,
    )
    waitingSession = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
    const attachAgent = agentForExternal(waitingSession, attachExternalSessionId)
    assert.ok(attachAgent, "spawned orphan should attach to the existing Arroba session")
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "03-tui-spawn-orphan-agent.png", "TUI Spawn Orphan Agent", [
      `PASS session agent count=${attachedSnapshot.session?.agentCount}`,
      `PASS attached agent=${attachAgent.id}`,
      `PASS external import=${attachExternalSessionId}`,
      `PASS observed attach history visible`,
    ])))

    const userTurn = await appendOpenCodeTurn(
      attachFile,
      "opencode-user-2",
      "user",
      `opencode external prompt visible live in TUI ${marker}`,
    )
    unwrap(await client.send(refreshExternalProviderSessionsRequest("opencode")), "ExternalProviderSessionsRefreshed")
    const userMetadataSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => entriesForText(snapshot, userTurn.text).length > 0 && hasExternalMetadata(snapshot, userTurn.providerTurnId),
      "external user turn metadata in TUI",
      30_000,
    )
    await waitForCondition(async () => {
      const session = unwrap(await client.send(requests.resolveSessionRequest(waitingSessionId, workspace)), "SessionResolved").session
      const active = promptStateForAgent(session, attachAgent.id)?.active_prompt
      return active?.prompt_origin === "external" && String(active.id ?? "").includes(userTurn.providerTurnId)
        ? active
        : false
    }, "kernel external active prompt for TUI attached orphan", 30_000)
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "04-tui-external-live-metadata.png", "TUI External Turn Metadata", [
      `PASS source=external_provider_observed`,
      `PASS provider=opencode`,
      `PASS providerTurnId=${userTurn.providerTurnId}`,
      `PASS observedAtMs=${entriesForText(userMetadataSnapshot, userTurn.text)[0]?.observedAtMs ?? "recorded"}`,
    ])))

    const queuedPromptText = `arroba prompt queued behind external TUI turn ${marker}`
    await automation.send("submit_prompt", { prompt: queuedPromptText })
    const queuedSnapshot = await waitForSnapshot(
      automation,
      (snapshot) => hasQueuedPromptSteerDisabled(snapshot),
      "TUI queued prompt with steering disabled",
      30_000,
    )
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "05-tui-external-active-queued-steer-disabled.png", "TUI External Active Queue Guard", [
      `PASS queued prompt=${queuedPromptText}`,
      `PASS queuedPrompt.steerDisabled=true`,
      `PASS queued entries=${allTranscriptEntries(queuedSnapshot).filter((entry) => entry?.queuedPrompt).length}`,
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
      (snapshot) => entriesForText(snapshot, assistantTurn.text).length > 0 && hasExternalMetadata(snapshot, assistantTurn.providerTurnId),
      "external assistant turn metadata in TUI",
      30_000,
    )
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
    evidence.push(path.relative(repoRoot, await renderTerminalScreenshot(artifactRoot, "06-tui-external-output-queue-drained.png", "TUI External Output And Queue Drain", [
      `PASS providerTurnId=${assistantTurn.providerTurnId}`,
      `PASS external assistant output visible`,
      `PASS queued prompt drained`,
      `PASS total transcript entries=${allTranscriptEntries(finalSnapshot).length}`,
    ])))

    const manifest = {
      ok: true,
      drill: "orphan-agents-tui-parity",
      marker,
      kernelUrl,
      waitingExternalSessionId,
      attachExternalSessionId,
      waitingSessionId,
      waitingAgentId: waitingAgent.id,
      attachAgentId: attachAgent.id,
      queuedPromptText,
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
          drill: "orphan-agents-tui-parity",
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
