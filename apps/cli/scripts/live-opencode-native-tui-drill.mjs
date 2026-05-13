import { spawn, execFile } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { setTimeout as sleep } from "node:timers/promises"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  endSessionRequest,
  getSessionHistoryRequest,
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
const marker = `NOP_${process.pid.toString(36)}_${Date.now().toString(36)}`

function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

function makePort() {
  return 57000 + Math.floor(Math.random() * 2000)
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
      const page = unwrap(await client.send(getSessionHistoryRequest(sessionId, 200, 100_000, null, agent.id)), "SessionHistory")
      const entries = page.entries.map((entry) => entry.entry).filter(Boolean)
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

async function runNativeOpenCodePrompt(proxyUrl, providerSessionId, worktree, prompt, logFile) {
  const executable = process.env.ARROBA_OPENCODE_BIN?.trim() || "opencode"
  const output = await new Promise((resolve, reject) => {
    const child = spawn(executable, [
      "run",
      "--attach",
      proxyUrl,
      "--session",
      providerSessionId,
      "--dir",
      worktree,
      prompt,
    ], {
      cwd: worktree,
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    const timer = setTimeout(() => {
      child.kill("SIGTERM")
      reject(new Error(`opencode run --attach timed out for ${providerSessionId}`))
    }, 180_000)
    child.stdout?.on("data", (chunk) => {
      stdout += chunk.toString("utf8")
    })
    child.stderr?.on("data", (chunk) => {
      stderr += chunk.toString("utf8")
    })
    child.once("error", (error) => {
      clearTimeout(timer)
      reject(error)
    })
    child.once("exit", (code, signal) => {
      clearTimeout(timer)
      if (code === 0) {
        resolve(`${stdout}\n${stderr}`)
        return
      }
      reject(new Error(`opencode run --attach exited with ${signal ?? code}\n${stdout}\n${stderr}`))
    })
  })
  await writeFile(logFile, output)
}

async function main() {
  const root = path.join("/tmp", `arb-oc-native-${process.pid}-${Date.now()}`)
  const kernelPort = makePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const workspace = repoRoot
  const worktree = repoRoot
  const screenA = `arroba-opencode-a-${process.pid}`
  const screenB = `arroba-opencode-b-${process.pid}`
  const screenCli = `arroba-opencode-cli-${process.pid}`
  const logs = {
    aDir: path.join(root, "opencode-a-screen"),
    bDir: path.join(root, "opencode-b-screen"),
    cliDir: path.join(root, "arroba-cli-screen"),
    a: path.join(root, "opencode-a-screen", "screenlog.0"),
    b: path.join(root, "opencode-b-screen", "screenlog.0"),
    cli: path.join(root, "arroba-cli-screen", "screenlog.0"),
    nativeA: path.join(root, "opencode-a-native-run.log"),
    nativeB: path.join(root, "opencode-b-native-run.log"),
    proxyA: path.join(root, "opencode-a.proxy.log"),
    proxyB: path.join(root, "opencode-b.proxy.log"),
  }
  const markers = {
    arrobaA: `${marker}_ARROBA_A`,
    arrobaB: `${marker}_ARROBA_B`,
    nativeA: `${marker}_NATIVE_A`,
    nativeB: `${marker}_NATIVE_B`,
  }
  const automationSocket = path.join("/tmp", `arb-oc-cli-${process.pid}.sock`)
  let daemon = null
  let client = null
  let sessionId = null
  try {
    await mkdir(root, { recursive: true })
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
        ARROBA_DAEMON_ID: `opencode-native-proxy-${process.pid}`,
        ARROBA_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        ARROBA_SESSION_HISTORY_DIR: path.join(root, "history"),
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForDaemon(kernelUrl, workspace, worktree)

    await startScreen(screenA, logs.aDir, "bun", [
      cliPath,
      "opencode",
      "--kernel-url",
      kernelUrl,
      "--alias",
      `native-${marker}`,
      "--agent-alias",
      "oc-a",
      "--workspace",
      workspace,
      "--worktree",
      worktree,
    ], {
      ...process.env,
      ARROBA_OPENCODE_NATIVE_DEBUG: "1",
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: logs.proxyA,
    })
    sessionId = (await waitForFileMatch(logs.a, /arroba session:\s+([^\s(]+)/)).match[1]
    const proxyA = (await waitForFileMatch(logs.a, /proxy:\s+(http:\/\/127\.0\.0\.1:\d+)/)).match[1]
    const providerSessionA = (await waitForFileMatch(logs.a, /opencode sess:\s+([^\s]+)/)).match[1]

    await startScreen(screenB, logs.bDir, "bun", [
      cliPath,
      "opencode",
      sessionId,
      "--kernel-url",
      kernelUrl,
      "--agent-alias",
      "oc-b",
      "--workspace",
      workspace,
      "--worktree",
      worktree,
    ], {
      ...process.env,
      ARROBA_OPENCODE_NATIVE_DEBUG: "1",
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: logs.proxyB,
    })
    const proxyB = (await waitForFileMatch(logs.b, /proxy:\s+(http:\/\/127\.0\.0\.1:\d+)/)).match[1]
    const providerSessionB = (await waitForFileMatch(logs.b, /opencode sess:\s+([^\s]+)/)).match[1]

    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(
      await client.send(attachToSessionRequest(sessionId, `opencode-native-drill-${process.pid}`)),
      "SessionAttached",
    ).attachment
    const agents = await waitForNamedAgents(client, sessionId, ["oc-a", "oc-b"])
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
      "opencode",
      "--model",
      "default",
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
      throw new Error(`observer CLI did not see both agents: ${JSON.stringify(snapshot.session)}`)
    }

    await runNativeOpenCodePrompt(proxyA, providerSessionA, worktree, `Reply with exactly ${markers.nativeA} and nothing else.`, logs.nativeA)
    await runNativeOpenCodePrompt(proxyB, providerSessionB, worktree, `Reply with exactly ${markers.nativeB} and nothing else.`, logs.nativeB)
    await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
      "oc-a": { prompts: [markers.nativeA], outputs: [markers.nativeA] },
      "oc-b": { prompts: [markers.nativeB], outputs: [markers.nativeB] },
    })
    const badgeTransitions = {
      "oc-a": {
        before: await waitForAgentBadgeTone(automationSocket, "oc-a", "idle"),
      },
      "oc-b": {
        before: await waitForAgentBadgeTone(automationSocket, "oc-b", "idle"),
      },
    }

    await fireAutomationRequest(automationSocket, {
      action: "workspace_shell_exec",
      command: `prompt oc-a Reply with exactly ${markers.arrobaA} and nothing else.`,
    })
    badgeTransitions["oc-a"].during = await waitForAgentBadgeTone(automationSocket, "oc-a", "working")
    await fireAutomationRequest(automationSocket, {
      action: "workspace_shell_exec",
      command: `prompt oc-b Reply with exactly ${markers.arrobaB} and nothing else.`,
    })
    badgeTransitions["oc-b"].during = await waitForAgentBadgeTone(automationSocket, "oc-b", "working")

    const histories = await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
      "oc-a": { prompts: [markers.arrobaA, markers.nativeA], outputs: [markers.arrobaA, markers.nativeA] },
      "oc-b": { prompts: [markers.arrobaB, markers.nativeB], outputs: [markers.arrobaB, markers.nativeB] },
    })
    badgeTransitions["oc-a"].after = await waitForAgentBadgeTone(automationSocket, "oc-a", "idle")
    badgeTransitions["oc-b"].after = await waitForAgentBadgeTone(automationSocket, "oc-b", "idle")
    if (histories["oc-a"].all.includes(markers.arrobaB) || histories["oc-a"].all.includes(markers.nativeB)) {
      throw new Error("agent oc-a history was contaminated with oc-b markers")
    }
    if (histories["oc-b"].all.includes(markers.arrobaA) || histories["oc-b"].all.includes(markers.nativeA)) {
      throw new Error("agent oc-b history was contaminated with oc-a markers")
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
    for (const [label, log] of [["oc-a", tuiALog], ["oc-b", tuiBLog]]) {
      const own = label === "oc-a" ? [markers.arrobaA, markers.nativeA] : [markers.arrobaB, markers.nativeB]
      for (const expected of own) {
        if (!log.includes(expected)) throw new Error(`${label} TUI log did not include ${expected}`)
      }
    }
    const proxyALog = await readFile(logs.proxyA, "utf8").catch(() => "")
    const proxyBLog = await readFile(logs.proxyB, "utf8").catch(() => "")
    if (!proxyALog.includes(markers.nativeA) || !proxyBLog.includes(markers.nativeB)) {
      throw new Error("native OpenCode prompts did not pass through both Arroba proxies")
    }

    const stateResponse = await client.send(getSessionStateRequest(sessionId))
    const state = (stateResponse.SessionState ?? stateResponse.SessionStateLoaded).session
    console.log(JSON.stringify({
      status: "ok",
      architecture: "opencode-tui + opencode-native-cli + arroba-cli -> native HTTP proxy -> kernel-managed opencode server",
      kernelUrl,
      sessionId,
      marker,
      agentAliases: agents.map((agent) => agent.alias),
      observerSawAgents: snapshot.session.agentCount,
      badgeTransitions,
      focusedAgentId: state.focused_agent_id ?? null,
      providerSessions: {
        "oc-a": providerSessionA,
        "oc-b": providerSessionB,
      },
      proxies: {
        "oc-a": proxyA,
        "oc-b": proxyB,
      },
      logs,
    }, null, 2))
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
    if (process.env.ARROBA_KEEP_NATIVE_PROXY_DRILL_ARTIFACTS !== "1") {
      await rm(root, { recursive: true, force: true }).catch(() => {})
      await rm(automationSocket, { force: true }).catch(() => {})
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
