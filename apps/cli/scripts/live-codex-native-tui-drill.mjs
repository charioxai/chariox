import { spawn, execFile } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { mkdir, readFile, rm } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { setTimeout as sleep } from "node:timers/promises"
import { historyOutlineRows } from "./lib/drill-history-outline.mjs"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  endSessionRequest,
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
const marker = `NP_${process.pid.toString(36)}_${Date.now().toString(36)}`
const MAX_LOG_CHARS = 128_000

function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

function makePort() {
  return 59000 + Math.floor(Math.random() * 2000)
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

async function screen(name, args) {
  await execFileAsync("screen", ["-S", name, ...args])
}

async function screenQuit(name) {
  await screen(name, ["-X", "quit"]).catch(() => {})
}

function startScreen(name, logDir, command, args, env) {
  return execFileAsync("screen", [
    "-dmS",
    name,
    "-L",
    command,
    ...args,
  ], { env, cwd: logDir })
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
  const deadline = Date.now() + 240_000
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    let ok = true
    const histories = {}
    for (const agent of agents) {
      const outline = unwrap(
        await client.send(getSessionHistoryOutlineRequest(sessionId, [agent.id], 8)),
        "SessionHistoryOutline",
      )
      const entries = historyOutlineRows(outline, { includeUserPrompt: true })
        .map((row) => row.entry)
        .filter(Boolean)
      histories[agent.alias] = {
        all: entries.map((entry) => entry.text ?? "").join("\n"),
        prompts: entries.filter((entry) => entry.kind === "user_prompt").map((entry) => entry.text ?? "").join("\n"),
        outputs: entries.filter((entry) => entry.kind !== "user_prompt").map((entry) => entry.text ?? "").join(""),
      }
      const expected = expectedByAgent[agent.alias] ?? {}
      for (const marker of expected.prompts ?? []) {
        ok &&= histories[agent.alias].prompts.includes(marker)
      }
      for (const marker of expected.outputs ?? []) {
        ok &&= histories[agent.alias].outputs.includes(marker)
      }
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

async function main() {
  const root = path.join(repoRoot, ".artifacts", "live-codex-native-tui-drill", nowStamp())
  const kernelPort = makePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const workspace = repoRoot
  const worktree = repoRoot
  const screenA = `arroba-codex-a-${process.pid}`
  const screenB = `arroba-codex-b-${process.pid}`
  const screenCli = `arroba-codex-cli-${process.pid}`
  const logs = {
    aDir: path.join(root, "codex-a-screen"),
    bDir: path.join(root, "codex-b-screen"),
    cliDir: path.join(root, "arroba-cli-screen"),
    a: path.join(root, "codex-a-screen", "screenlog.0"),
    b: path.join(root, "codex-b-screen", "screenlog.0"),
    cli: path.join(root, "arroba-cli-screen", "screenlog.0"),
    proxyA: path.join(root, "codex-a.proxy.log"),
    proxyB: path.join(root, "codex-b.proxy.log"),
  }
  const markers = {
    arrobaA: `${marker}_ARROBA_TO_A`,
    arrobaB: `${marker}_ARROBA_TO_B`,
    tuiA: `${marker}_TUI_A`,
    tuiB: `${marker}_TUI_B`,
  }
  const automationSocket = path.join("/tmp", `arb-cdx-cli-${process.pid}.sock`)
  let daemon = null
  let client = null
  let sessionId = null
  let passed = false
  let failure = null
  let daemonStdout = ""
  let daemonStderr = ""
  let agents = []
  let snapshot = null
  let proxyUpstreamConnections = null
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
        ARROBA_DAEMON_ID: `codex-native-tui-${process.pid}`,
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
      "codex",
      "--kernel-url",
      kernelUrl,
      "--alias",
      `native-${marker}`,
      "--agent-alias",
      "cdx-a",
      "--workspace",
      workspace,
      "--worktree",
      worktree,
      "--model",
      "gpt-5.4-mini",
      "--effort",
      "high",
      "--initial-prompt",
      `Reply with exactly ${markers.tuiA} and nothing else.`,
    ], {
      ...process.env,
      ARROBA_CODEX_NATIVE_DEBUG: "1",
      ARROBA_CODEX_NATIVE_DEBUG_FILE: logs.proxyA,
    })
    sessionId = (await waitForFileMatch(logs.a, /arroba session:\s+([^\s(]+)/)).match[1]

    await startScreen(screenB, logs.bDir, "bun", [
      cliPath,
      "codex",
      sessionId,
      "--kernel-url",
      kernelUrl,
      "--agent-alias",
      "cdx-b",
      "--workspace",
      workspace,
      "--worktree",
      worktree,
      "--model",
      "gpt-5.4-mini",
      "--effort",
      "high",
      "--initial-prompt",
      `Reply with exactly ${markers.tuiB} and nothing else.`,
    ], {
      ...process.env,
      ARROBA_CODEX_NATIVE_DEBUG: "1",
      ARROBA_CODEX_NATIVE_DEBUG_FILE: logs.proxyB,
    })

    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(
      await client.send(attachToSessionRequest(sessionId, `codex-native-drill-${process.pid}`)),
      "SessionAttached",
    ).attachment
    agents = await waitForNamedAgents(client, sessionId, ["cdx-a", "cdx-b"])
    await waitForActiveProviderRun(client, sessionId)

    await startScreen(screenCli, logs.cliDir, "bun", [
      cliPath,
      "--kernel-url",
      kernelUrl,
      "--session",
      sessionId,
      "--client-id",
      `arroba-observer-${process.pid}`,
      "--automation-socket",
      automationSocket,
      "--provider",
      "codex",
      "--model",
      "gpt-5.4-mini",
      "--effort",
      "high",
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
    snapshot = await automationRequest(automationSocket, {
      action: "wait_for",
      sessionId,
      shellEntryCount: 0,
      timeoutMs: 20_000,
    })
    if (snapshot.session.agentCount < 2) {
      throw new Error(`observer CLI did not see both agents: ${JSON.stringify(snapshot.session)}`)
    }

    await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
      "cdx-a": { prompts: [markers.tuiA], outputs: [markers.tuiA] },
      "cdx-b": { prompts: [markers.tuiB], outputs: [markers.tuiB] },
    })
    const badgeTransitions = {
      "cdx-a": {
        before: await waitForAgentBadgeTone(automationSocket, "cdx-a", "idle"),
      },
      "cdx-b": {
        before: await waitForAgentBadgeTone(automationSocket, "cdx-b", "idle"),
      },
    }
    await fireAutomationRequest(automationSocket, {
      action: "workspace_shell_exec",
      command: `prompt cdx-a Reply with exactly ${markers.arrobaA} and nothing else.`,
    })
    badgeTransitions["cdx-a"].during = await waitForAgentBadgeTone(automationSocket, "cdx-a", "working")
    await fireAutomationRequest(automationSocket, {
      action: "workspace_shell_exec",
      command: `prompt cdx-b Reply with exactly ${markers.arrobaB} and nothing else.`,
    })
    badgeTransitions["cdx-b"].during = await waitForAgentBadgeTone(automationSocket, "cdx-b", "working")

    const histories = await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
      "cdx-a": { prompts: [markers.arrobaA, markers.tuiA], outputs: [markers.arrobaA, markers.tuiA] },
      "cdx-b": { prompts: [markers.arrobaB, markers.tuiB], outputs: [markers.arrobaB, markers.tuiB] },
    })
    badgeTransitions["cdx-a"].after = await waitForAgentBadgeTone(automationSocket, "cdx-a", "idle")
    badgeTransitions["cdx-b"].after = await waitForAgentBadgeTone(automationSocket, "cdx-b", "idle")
    if (histories["cdx-a"].all.includes(markers.arrobaB) || histories["cdx-a"].all.includes(markers.tuiB)) {
      throw new Error("agent cdx-a history was contaminated with cdx-b markers")
    }
    if (histories["cdx-b"].all.includes(markers.arrobaA) || histories["cdx-b"].all.includes(markers.tuiA)) {
      throw new Error("agent cdx-b history was contaminated with cdx-a markers")
    }

    await automationRequest(automationSocket, { action: "switch_screen", screen: "agents" })
    await sleep(1_000)
    const cliLog = await readFile(logs.cli, "utf8").catch(() => "")
    const expectedCliMarkers = Object.values(markers)
    if (expectedCliMarkers.some((expected) => !cliLog.includes(expected))) {
      throw new Error("observer Arroba CLI screen did not include all native and Arroba-submitted markers")
    }
    const tuiALog = await readFile(logs.a, "utf8").catch(() => "")
    const tuiBLog = await readFile(logs.b, "utf8").catch(() => "")
    const proxyALog = await readFile(logs.proxyA, "utf8").catch(() => "")
    const proxyBLog = await readFile(logs.proxyB, "utf8").catch(() => "")
    for (const [label, log] of [["cdx-a", tuiALog], ["cdx-b", tuiBLog]]) {
      const own = label === "cdx-a" ? [markers.arrobaA, markers.tuiA] : [markers.arrobaB, markers.tuiB]
      for (const expected of own) {
        if (!log.includes(expected)) throw new Error(`${label} TUI log did not include ${expected}`)
      }
    }
    if ((proxyALog.match(/upstream_connected/g) ?? []).length !== 1 || (proxyBLog.match(/upstream_connected/g) ?? []).length !== 1) {
      throw new Error("expected exactly one upstream Codex app-server websocket per native pair")
    }
    if (!proxyALog.includes("kernel_connected") || !proxyBLog.includes("kernel_connected")) {
      throw new Error("kernel did not connect downstream to both native proxies")
    }
    proxyUpstreamConnections = {
      "cdx-a": (proxyALog.match(/upstream_connected/g) ?? []).length,
      "cdx-b": (proxyBLog.match(/upstream_connected/g) ?? []).length,
    }

    const stateResponse = await client.send(getSessionStateRequest(sessionId))
    const state = (stateResponse.SessionState ?? stateResponse.SessionStateLoaded).session
    console.log(JSON.stringify({
      status: "ok",
      architecture: "codex-tui + arroba-cli -> native proxy -> single codex app-server websocket",
      kernelUrl,
      sessionId,
      marker,
      agentAliases: agents.map((agent) => agent.alias),
      observerSawAgents: snapshot.session.agentCount,
      badgeTransitions,
      focusedAgentId: state.focused_agent_id ?? null,
      proxyUpstreamConnections,
      logs,
    }, null, 2))
    passed = true
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) await client.close().catch(() => {})
    await screenQuit(screenA)
    await screenQuit(screenB)
    await screenQuit(screenCli)
    if (daemon && daemon.exitCode == null) {
      daemon.kill("SIGTERM")
      await Promise.race([new Promise((resolve) => daemon.once("exit", resolve)), sleep(2_000)])
      if (daemon.exitCode == null) daemon.kill("SIGKILL")
    }
    if (passed && process.env.ARROBA_KEEP_NATIVE_PROXY_DRILL_ARTIFACTS === "1") {
      console.log(JSON.stringify({ status: "kept-artifacts", root, automationSocket }))
    } else {
      await finalizeDrillArtifacts({
        rootDir: root,
        passed,
        preserveOnFailure: true,
        failure,
        metadata: {
          drill: "live-codex-native-tui",
          kernelUrl,
          sessionId,
          workspace,
          worktree,
          marker,
          markers,
          logs,
          automationSocket,
          agentAliases: agents.map((agent) => agent.alias),
          observerSawAgents: snapshot?.session?.agentCount ?? null,
          proxyUpstreamConnections,
          daemonStdoutTail: tailLines(daemonStdout),
          daemonStderrTail: tailLines(daemonStderr),
        },
      })
    }
    await rm(automationSocket, { force: true }).catch(() => {})
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
