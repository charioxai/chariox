import { spawn, execFile } from "node:child_process"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { mkdir, readFile, rm } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { setTimeout as sleep } from "node:timers/promises"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  endSessionRequest,
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
  getSessionStateRequest,
  listAgentsRequest,
  pumpTerminalOutputRequest,
} from "../dist/ipc-requests.js"

const execFileAsync = promisify(execFile)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cliPath = path.join(cliRoot, "dist/index.js")
const kernelBinary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
const marker = `CLN_${process.pid.toString(36)}_${Date.now().toString(36)}`
const MAX_LOG_CHARS = 128_000

function unwrap(response, variant) {
  if (!response || !(variant in response)) throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  return response[variant]
}

function makePort() {
  return 61000 + Math.floor(Math.random() * 1000)
}

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, "-")
}

function appendOutput(buffer, chunk) {
  const next = buffer + chunk.toString("utf8")
  if (next.length <= MAX_LOG_CHARS) return next
  return next.slice(next.length - MAX_LOG_CHARS)
}

function tailLines(value, count = 80) {
  return value.split("\n").slice(-count).join("\n")
}

async function waitForDaemon(kernelUrl, workspace, worktree) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(kernelUrl)
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
  throw new Error("kernel did not become ready")
}

async function waitForFileMatch(file, pattern, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs
  let text = ""
  while (Date.now() < deadline) {
    text = await readFile(file, "utf8").catch(() => "")
    const match = text.match(pattern)
    if (match) return { match, text }
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${pattern} in ${file}\n${text.slice(-4000)}`)
}

function startScreen(name, logDir, command, args, env) {
  return execFileAsync("screen", ["-dmS", name, "-L", command, ...args], { env, cwd: logDir })
}

async function screen(name, args) {
  await execFileAsync("screen", ["-S", name, ...args])
}

async function screenQuit(name) {
  await screen(name, ["-X", "quit"]).catch(() => {})
}

function nativeTempRootForScreen(screenName) {
  const suffix = screenName.replace(/^arroba-claude-/, "")
  return path.join(os.tmpdir(), `arroba-claude-native-${suffix}`)
}

async function waitForNativeEvents(eventsFile, expectedPrompts, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs
  let raw = ""
  while (Date.now() < deadline) {
    raw = await readFile(eventsFile, "utf8").catch(() => "")
    const events = raw
      .split("\n")
      .filter((line) => line.trim())
      .map((line) => {
        try {
          return JSON.parse(line)
        } catch {
          return null
        }
      })
      .filter(Boolean)
    const prompts = events
      .filter((event) => event.hook_event_name === "UserPromptSubmit")
      .map((event) => event.prompt ?? "")
      .join("\n")
    if (expectedPrompts.every((prompt) => prompts.includes(prompt))) return events
    await sleep(500)
  }
  throw new Error(`timed out waiting for native Claude hook prompts in ${eventsFile}\n${raw}`)
}

async function automationRequest(socketPath, request, timeoutMs = 20_000) {
  return await new Promise((resolve, reject) => {
    const socket = net.createConnection(socketPath)
    let buffer = ""
    socket.setTimeout(timeoutMs)
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

async function waitForAutomation(socketPath) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      await automationRequest(socketPath, { action: "ping" })
      return
    } catch (error) {
      if (attempt === 79) throw error
      await sleep(250)
    }
  }
}

async function waitForNamedAgents(client, sessionId, aliases) {
  const deadline = Date.now() + 90_000
  while (Date.now() < deadline) {
    const agents = unwrap(await client.send(listAgentsRequest(sessionId)), "AgentsListed").agents ?? []
    const named = agents.filter((agent) => aliases.includes(agent.alias))
    if (new Set(named.map((agent) => agent.alias)).size === aliases.length) return named
    await sleep(500)
  }
  throw new Error(`timed out waiting for agents ${aliases.join(", ")}`)
}

async function waitForHistoryMarkers(client, sessionId, attachmentId, agents, expectedByAgent) {
  const deadline = Date.now() + 300_000
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    let ok = true
    const histories = {}
    for (const agent of agents) {
      const entries = await readAgentHistoryEntries(client, sessionId, agent.id)
      histories[agent.alias] = {
        all: entries.map((entry) => entry.text ?? "").join("\n"),
        prompts: entries.filter((entry) => entry.kind === "user_prompt").map((entry) => entry.text ?? "").join("\n"),
        outputs: entries.filter((entry) => entry.kind !== "user_prompt").map((entry) => entry.text ?? "").join("\n"),
      }
      const expected = expectedByAgent[agent.alias] ?? {}
      for (const markerText of expected.prompts ?? []) ok &&= histories[agent.alias].prompts.includes(markerText)
      for (const markerText of expected.outputs ?? []) ok &&= histories[agent.alias].outputs.includes(markerText)
    }
    if (ok) return histories
    await sleep(1_000)
  }
  throw new Error("timed out waiting for Claude native history markers")
}

async function readAgentHistoryEntries(client, sessionId, agentId) {
  const outline = unwrap(await client.send(getSessionHistoryOutlineRequest(sessionId, [agentId], 8)), "SessionHistoryOutline")
  const agent = outline.agents?.find((entry) => entry.agent_id === agentId) ?? null
  const entries = []
  for (const turn of agent?.turns ?? []) {
    if (turn.user_prompt?.entry) entries.push(turn.user_prompt.entry)
    for (const row of turn.entries ?? []) {
      if (row?.entry) entries.push(row.entry)
    }
    if (turn.summary?.entry) entries.push(turn.summary.entry)
    for (const blob of turn.blobs ?? []) {
      const blobContent = unwrap(await client.send(getSessionHistoryBlobContentRequest(sessionId, agentId, blob.blob_id)), "SessionHistoryBlobContent")
      for (const row of blobContent.entries ?? []) {
        if (row?.entry) entries.push(row.entry)
      }
    }
  }
  return entries
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

async function main() {
  const root = path.join(repoRoot, ".artifacts", "live-claude-native-tui-drill", nowStamp())
  const kernelPort = makePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const workspace = repoRoot
  const worktree = repoRoot
  const screenA = `arroba-claude-a-${process.pid}`
  const screenB = `arroba-claude-b-${process.pid}`
  const screenCli = `arroba-claude-cli-${process.pid}`
  const logs = {
    aDir: path.join(root, "claude-a-screen"),
    bDir: path.join(root, "claude-b-screen"),
    cliDir: path.join(root, "arroba-cli-screen"),
    a: path.join(root, "claude-a-screen", "screenlog.0"),
    b: path.join(root, "claude-b-screen", "screenlog.0"),
    cli: path.join(root, "arroba-cli-screen", "screenlog.0"),
  }
  const markers = {
    arrobaA: `${marker}_ARROBA_TO_A`,
    arrobaB: `${marker}_ARROBA_TO_B`,
    tuiA: `${marker}_TUI_A`,
    tuiB: `${marker}_TUI_B`,
  }
  const automationSocket = path.join("/tmp", `arb-claude-cli-${process.pid}.sock`)
  let daemon = null
  let client = null
  let sessionId = null
  let innerA = null
  let innerB = null
  let passed = false
  let failure = null
  let daemonStdout = ""
  let daemonStderr = ""
  let agents = []
  let snapshot = null
  try {
    await prepareDrillArtifacts(root)
    await mkdir(logs.aDir, { recursive: true })
    await mkdir(logs.bDir, { recursive: true })
    await mkdir(logs.cliDir, { recursive: true })
    daemon = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_KERNEL_PORT: String(kernelPort),
        ARROBA_MCP_PORT: String(kernelPort + 1000),
        ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
        ARROBA_CODEX_PORT: String(kernelPort + 2001),
        ARROBA_DAEMON_ID: `claude-native-tui-${process.pid}`,
        ARROBA_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        ARROBA_SESSION_HISTORY_DIR: path.join(root, "history"),
      },
      stdio: ["ignore", "pipe", "pipe"],
    })
    daemon.stdout.on("data", (chunk) => { daemonStdout = appendOutput(daemonStdout, chunk) })
    daemon.stderr.on("data", (chunk) => { daemonStderr = appendOutput(daemonStderr, chunk) })
    await waitForDaemon(kernelUrl, workspace, worktree)

    await startScreen(screenA, logs.aDir, "bun", [
      cliPath,
      "claude",
      "--detached-screen",
      "--kernel-url",
      kernelUrl,
      "--alias",
      `native-${marker}`,
      "--agent-alias",
      "claude-a",
      "--workspace",
      workspace,
      "--worktree",
      worktree,
      "--model",
      "sonnet",
      "--effort",
      "low",
      "--initial-prompt",
      `Reply with exactly ${markers.tuiA} and nothing else.`,
    ], process.env)
    sessionId = (await waitForFileMatch(logs.a, /arroba session:\s+([^\s(]+)/)).match[1]
    innerA = (await waitForFileMatch(logs.a, /screen:\s+([^\s]+)/)).match[1]
    logs.aEvents = path.join(nativeTempRootForScreen(innerA), "events.jsonl")

    await startScreen(screenB, logs.bDir, "bun", [
      cliPath,
      "claude",
      sessionId,
      "--detached-screen",
      "--kernel-url",
      kernelUrl,
      "--agent-alias",
      "claude-b",
      "--workspace",
      workspace,
      "--worktree",
      worktree,
      "--model",
      "sonnet",
      "--effort",
      "low",
      "--initial-prompt",
      `Reply with exactly ${markers.tuiB} and nothing else.`,
    ], process.env)
    innerB = (await waitForFileMatch(logs.b, /screen:\s+([^\s]+)/)).match[1]
    logs.bEvents = path.join(nativeTempRootForScreen(innerB), "events.jsonl")

    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(
      await client.send(attachToSessionRequest(sessionId, `claude-native-drill-${process.pid}`)),
      "SessionAttached",
    ).attachment
    agents = await waitForNamedAgents(client, sessionId, ["claude-a", "claude-b"])

    await startScreen(screenCli, logs.cliDir, "bun", [
      cliPath,
      "--kernel-url",
      kernelUrl,
      "--session",
      sessionId,
      "--client-id",
      `claude-observer-${process.pid}`,
      "--automation-socket",
      automationSocket,
      "--provider",
      "claude",
      "--model",
      "sonnet",
      "--effort",
      "low",
    ], process.env)
    await waitForAutomation(automationSocket)
    snapshot = await automationRequest(automationSocket, {
      action: "wait_for",
      sessionId,
      shellEntryCount: 0,
      timeoutMs: 20_000,
    })
    if (snapshot.session.agentCount < 2) throw new Error(`observer CLI did not see both agents: ${JSON.stringify(snapshot.session)}`)

    await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
      "claude-a": { prompts: [markers.tuiA], outputs: [markers.tuiA] },
      "claude-b": { prompts: [markers.tuiB], outputs: [markers.tuiB] },
    })

    const badgeTransitions = {
      "claude-a": { before: await waitForAgentBadgeTone(automationSocket, "claude-a", "idle") },
      "claude-b": { before: await waitForAgentBadgeTone(automationSocket, "claude-b", "idle") },
    }
    await fireAutomationRequest(automationSocket, {
      action: "workspace_shell_exec",
      command: `prompt claude-a Reply with exactly ${markers.arrobaA} and nothing else.`,
    })
    badgeTransitions["claude-a"].during = await waitForAgentBadgeTone(automationSocket, "claude-a", "working")
    await fireAutomationRequest(automationSocket, {
      action: "workspace_shell_exec",
      command: `prompt claude-b Reply with exactly ${markers.arrobaB} and nothing else.`,
    })
    badgeTransitions["claude-b"].during = await waitForAgentBadgeTone(automationSocket, "claude-b", "working")

    const histories = await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
      "claude-a": { prompts: [markers.arrobaA, markers.tuiA], outputs: [markers.arrobaA, markers.tuiA] },
      "claude-b": { prompts: [markers.arrobaB, markers.tuiB], outputs: [markers.arrobaB, markers.tuiB] },
    })
    badgeTransitions["claude-a"].after = await waitForAgentBadgeTone(automationSocket, "claude-a", "idle")
    badgeTransitions["claude-b"].after = await waitForAgentBadgeTone(automationSocket, "claude-b", "idle")

    if (histories["claude-a"].all.includes(markers.arrobaB) || histories["claude-a"].all.includes(markers.tuiB)) {
      throw new Error("agent claude-a history was contaminated with claude-b markers")
    }
    if (histories["claude-b"].all.includes(markers.arrobaA) || histories["claude-b"].all.includes(markers.tuiA)) {
      throw new Error("agent claude-b history was contaminated with claude-a markers")
    }

    await waitForNativeEvents(logs.aEvents, [markers.tuiA, markers.arrobaA])
    await waitForNativeEvents(logs.bEvents, [markers.tuiB, markers.arrobaB])
    const stateResponse = await client.send(getSessionStateRequest(sessionId))
    const state = (stateResponse.SessionState ?? stateResponse.SessionStateLoaded).session
    console.log(JSON.stringify({
      status: "ok",
      architecture: "claude-code native TUI + hooks + Arroba PTY injection",
      kernelUrl,
      sessionId,
      marker,
      agentAliases: agents.map((agent) => agent.alias),
      observerSawAgents: snapshot.session.agentCount,
      badgeTransitions,
      focusedAgentId: state.focused_agent_id ?? null,
      logs,
    }, null, 2))
    passed = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) await client.close().catch(() => {})
    if (innerA) await screenQuit(innerA)
    if (innerB) await screenQuit(innerB)
    await screenQuit(screenA)
    await screenQuit(screenB)
    await screenQuit(screenCli)
    if (daemon && daemon.exitCode == null) {
      daemon.kill("SIGTERM")
      await Promise.race([new Promise((resolve) => daemon.once("exit", resolve)), sleep(2_000)])
      if (daemon.exitCode == null) daemon.kill("SIGKILL")
    }
    const innerRoots = {
      "claude-a": innerA ? nativeTempRootForScreen(innerA) : null,
      "claude-b": innerB ? nativeTempRootForScreen(innerB) : null,
    }
    if (passed && process.env.ARROBA_KEEP_NATIVE_PROXY_DRILL_ARTIFACTS === "1") {
      console.log(JSON.stringify({ status: "kept-artifacts", root, automationSocket, innerRoots }))
    } else {
      await finalizeDrillArtifacts({
        rootDir: root,
        passed,
        preserveOnFailure: true,
        failure,
        metadata: {
          drill: "live-claude-native-tui",
          kernelUrl,
          sessionId,
          workspace,
          worktree,
          marker,
          markers,
          logs,
          automationSocket,
          innerScreens: {
            "claude-a": innerA,
            "claude-b": innerB,
          },
          innerRoots,
          agentAliases: agents.map((agent) => agent.alias),
          observerSawAgents: snapshot?.session?.agentCount ?? null,
          daemonStdoutTail: tailLines(daemonStdout),
          daemonStderrTail: tailLines(daemonStderr),
        },
      })
      if (passed) {
        if (innerA) await rm(nativeTempRootForScreen(innerA), { recursive: true, force: true }).catch(() => {})
        if (innerB) await rm(nativeTempRootForScreen(innerB), { recursive: true, force: true }).catch(() => {})
      }
    }
    await rm(automationSocket, { force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
