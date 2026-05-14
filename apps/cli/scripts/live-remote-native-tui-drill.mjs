import { spawn, execFile } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { access, mkdir, readFile, rm, symlink, writeFile } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { setTimeout as sleep } from "node:timers/promises"
import os from "node:os"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  endSessionRequest,
  getSessionHistoryRequest,
  getSessionStateRequest,
  listAgentsRequest,
  listRemoteMachinesRequest,
  pumpTerminalOutputRequest,
} from "../dist/ipc-requests.js"

const execFileAsync = promisify(execFile)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cliPath = path.join(cliRoot, "dist/index.js")
const kernelBinary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
const relayBinary = path.join(repoRoot, "apps/relay/target/debug/arroba-relay")
const sliceDockerScript = path.join(repoRoot, "experiments/slice-spike/scripts/provision-linux-docker-slice.sh")
const realHomeDir = os.homedir()

function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

function parseArgs(argv) {
  const options = {
    providers: ["opencode", "codex", "claude"],
    keepArtifactsOnFailure: false,
    sliceLocalDocker: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") {
      continue
    } else if (arg === "--providers") {
      options.providers = argv[++index].split(",").map((provider) => provider.trim()).filter(Boolean)
    } else if (arg === "--keep-artifacts-on-failure") {
      options.keepArtifactsOnFailure = true
    } else if (arg === "--slice-local-docker") {
      options.sliceLocalDocker = true
    } else if (arg === "--help" || arg === "-h") {
      options.help = true
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
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
    "  --slice-local-docker          Run against a local Docker slice kernel instead of a host kernel",
    "  --keep-artifacts-on-failure",
  ].join("\n"))
}

function makePorts(base = 52000 + Math.floor(Math.random() * 4000)) {
  return {
    relayPort: base,
    kernelPort: base + 1000,
    mcpPort: base + 1001,
    openCodePort: base + 2000,
    codexPort: base + 2001,
  }
}

async function makeAvailablePorts({ includeSliceRanges = false } = {}) {
  const preferredBases = includeSliceRanges ? [43000, 44000, 45000, 46000, 47000, 48000] : []
  for (const base of preferredBases) {
    const ports = makePorts(base)
    if (await portsAreAvailable(ports, includeSliceRanges)) return ports
  }
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const ports = makePorts()
    if (await portsAreAvailable(ports, includeSliceRanges)) return ports
  }
  throw new Error("could not find available drill ports")
}

async function portsAreAvailable(ports, includeSliceRanges) {
  const candidates = [
    ports.relayPort,
    ports.kernelPort,
    ports.mcpPort,
    ports.openCodePort,
    ports.codexPort,
  ]
  if (includeSliceRanges) {
    candidates.push(ports.relayPort + 3000)
    for (let offset = 10; offset < 30; offset += 1) {
      candidates.push(ports.openCodePort + offset)
      candidates.push(ports.codexPort + 100 + offset)
    }
  }
  for (const port of candidates) {
    if (!(await portIsAvailable(port))) return false
  }
  return true
}

async function portIsAvailable(port) {
  return await new Promise((resolve) => {
    const server = net.createServer()
    server.once("error", () => resolve(false))
    server.listen(port, "127.0.0.1", () => {
      server.close(() => resolve(true))
    })
  })
}

async function assertBinary(binaryPath, manifestPath, binName) {
  try {
    await access(binaryPath)
  } catch {
    throw new Error(`missing built binary ${binaryPath}; run cargo build --manifest-path ${manifestPath} --bin ${binName} first`)
  }
}

async function waitForTcpPort(port, host = "127.0.0.1", timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const ready = await new Promise((resolve) => {
      const socket = net.connect({ host, port })
      socket.once("connect", () => {
        socket.destroy()
        resolve(true)
      })
      socket.once("error", () => {
        socket.destroy()
        resolve(false)
      })
    })
    if (ready) return
    await sleep(100)
  }
  throw new Error(`TCP listener ${host}:${port} did not become reachable`)
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

async function waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias) {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const client = new LocalIpcClient(relayUrl, {
      relayAuthToken: relayToken,
      targetDaemonAlias,
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
  throw new Error(`relay target ${targetDaemonAlias} did not become reachable`)
}

async function terminateChild(child, signal = "SIGTERM") {
  if (!child || child.exitCode != null) return
  child.kill(signal)
  await Promise.race([
    new Promise((resolve) => child.once("exit", resolve)),
    sleep(5_000),
  ])
  if (child.exitCode == null) {
    child.kill("SIGKILL")
    await Promise.race([
      new Promise((resolve) => child.once("exit", resolve)),
      sleep(2_000),
    ])
  }
}

async function runLogged(command, args, options = {}) {
  await new Promise((resolve, reject) => {
    const child = spawn(command, args, {
      cwd: options.cwd ?? repoRoot,
      env: options.env ?? process.env,
      stdio: "inherit",
    })
    child.once("error", reject)
    child.once("exit", (code, signal) => {
      if (code === 0) {
        resolve()
        return
      }
      reject(new Error(`${command} ${args.join(" ")} exited with ${signal ?? code}`))
    })
  })
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

async function waitForFileMatch(file, pattern, timeoutMs = 90_000) {
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
  workspace,
  worktree,
  nativeEnv = {},
}) {
  const scenarioRoot = path.join(root, provider)
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
  const marker = provider === "opencode" ? "OPENCODE" : provider === "codex" ? "CODEX" : "CLAUDE"
  const markers = {
    arrobaA: `${marker}ALPHA`,
    arrobaB: `${marker}BRAVO`,
    nativeA: `${marker}CHARLIE`,
    nativeB: `${marker}DELTA`,
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
  try {
    await mkdir(logs.aDir, { recursive: true })
    await mkdir(logs.bDir, { recursive: true })
    await mkdir(logs.cliDir, { recursive: true })

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
      ...providerArgs,
      ...(provider === "codex" || provider === "claude" ? ["--initial-prompt", `Reply with exactly ${markers.nativeA} and nothing else.`] : []),
      ...(provider === "claude" ? ["--remote-rendered"] : []),
    ], {
      ...process.env,
      ...nativeEnv,
      ARROBA_CODEX_NATIVE_DEBUG: "1",
      ARROBA_CODEX_NATIVE_DEBUG_FILE: logs.proxyA,
      ARROBA_OPENCODE_NATIVE_DEBUG: "1",
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: logs.proxyA,
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
      ...providerArgs,
      ...(provider === "codex" || provider === "claude" ? ["--initial-prompt", `Reply with exactly ${markers.nativeB} and nothing else.`] : []),
      ...(provider === "claude" ? ["--remote-rendered"] : []),
    ], {
      ...process.env,
      ...nativeEnv,
      ARROBA_CODEX_NATIVE_DEBUG: "1",
      ARROBA_CODEX_NATIVE_DEBUG_FILE: logs.proxyB,
      ARROBA_OPENCODE_NATIVE_DEBUG: "1",
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: logs.proxyB,
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
    }

    client = relayClient(relayUrl, relayToken, targetDaemonAlias)
    const attachment = unwrap(
      await client.send(attachToSessionRequest(sessionId, `remote-native-${provider}-drill-${process.pid}`)),
      "SessionAttached",
    ).attachment
    const agents = await waitForNamedAgents(client, sessionId, aliases)
    await waitForActiveProviderRun(client, sessionId)

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

    if (provider === "opencode") {
      await runNativeOpenCodePrompt(proxyA, providerSessionA, worktree, `Reply with exactly ${markers.nativeA} and nothing else.`, logs.nativeA)
      await runNativeOpenCodePrompt(proxyB, providerSessionB, worktree, `Reply with exactly ${markers.nativeB} and nothing else.`, logs.nativeB)
    }

    await waitForHistoryMarkers(client, sessionId, attachment.id, agents, {
      [aliases[0]]: { prompts: [markers.nativeA], outputs: [markers.nativeA] },
      [aliases[1]]: { prompts: [markers.nativeB], outputs: [markers.nativeB] },
    })

    const badgeTransitions = {
      [aliases[0]]: {
        before: await waitForAgentBadgeTone(automationSocket, aliases[0], "idle"),
      },
      [aliases[1]]: {
        before: await waitForAgentBadgeTone(automationSocket, aliases[1], "idle"),
      },
    }

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
      await waitForFileMatch(logs.a, new RegExp(expected), 30_000)
    }
    for (const expected of [markers.arrobaB, markers.nativeB]) {
      await waitForFileMatch(logs.b, new RegExp(expected), 30_000)
    }

    const proxyALog = await readFile(logs.proxyA, "utf8").catch(() => "")
    const proxyBLog = await readFile(logs.proxyB, "utf8").catch(() => "")
    if (provider === "codex") {
      const expectedProxySignal = nativeEnv.ARROBA_CODEX_KERNEL_SERVER_PORT_RANGE
        ? "provider_run_bound"
        : "kernel_connected"
      if (!proxyALog.includes(expectedProxySignal) || !proxyBLog.includes(expectedProxySignal)) {
        throw new Error(`remote Codex native proxies did not record ${expectedProxySignal}`)
      }
    } else if (provider === "opencode" && (!proxyALog.includes(markers.nativeA) || !proxyBLog.includes(markers.nativeB))) {
      throw new Error("native OpenCode prompts did not pass through both remote native proxies")
    }

    return {
      provider,
      sessionId,
      marker,
      relayUrl,
      targetDaemonAlias,
      agentAliases: aliases,
      observerSawAgents: snapshot.session.agentCount,
      badgeTransitions,
      providerSessions: provider === "opencode" ? {
        [aliases[0]]: providerSessionA,
        [aliases[1]]: providerSessionB,
      } : null,
      logs,
      note: provider === "claude"
        ? "remote-rendered Claude TUI validated through kernel-owned PTY"
        : "server-in-kernel native TUI validated on one host; true network-isolated provider endpoints still require the reverse provider tunnel",
    }
  } finally {
    if (client) await client.close().catch(() => {})
    await screenQuit(screenA)
    await screenQuit(screenB)
    await screenQuit(screenCli)
    await rm(automationSocket, { force: true }).catch(() => {})
  }
}

function sliceDockerEnv({ root, ports, targetDaemonAlias }) {
  const sliceName = `arroba-rnt-slice-${process.pid}`
  const codexRangeStart = ports.codexPort + 110
  const opencodeRangeStart = ports.openCodePort + 10
  return {
    ...process.env,
    ARROBA_SLICE_NAME: sliceName,
    ARROBA_SLICE_HOME_VOLUME: `${sliceName}-home`,
    ARROBA_SLICE_RECREATE: "1",
    ARROBA_SLICE_WORKSPACE: repoRoot,
    ARROBA_SLICE_BUILD_IMAGE: process.env.ARROBA_SLICE_BUILD_IMAGE ?? "always",
    ARROBA_SLICE_START_DESKTOP: "0",
    ARROBA_SLICE_START_PROVIDER_SERVERS: "0",
    ARROBA_SLICE_START_RUNTIME: "1",
    ARROBA_SLICE_IMPORT_PROVIDER_AUTH: "1",
    ARROBA_SLICE_CODEX_PORT: String(ports.codexPort),
    ARROBA_SLICE_OPENCODE_PORT: String(ports.openCodePort),
    ARROBA_SLICE_CODEX_PORT_RANGE: `${codexRangeStart}-${codexRangeStart + 19}`,
    ARROBA_SLICE_OPENCODE_PORT_RANGE: `${opencodeRangeStart}-${opencodeRangeStart + 19}`,
    ARROBA_SLICE_PROVIDER_BIND_HOST: "0.0.0.0",
    ARROBA_SLICE_KERNEL_PORT: String(ports.kernelPort),
    ARROBA_SLICE_MCP_PORT: String(ports.mcpPort),
    ARROBA_SLICE_RELAY_PORT: String(ports.relayPort),
    ARROBA_SLICE_NOVNC_PORT: String(ports.relayPort + 3000),
    ARROBA_SLICE_DAEMON_ALIAS: targetDaemonAlias,
    ARROBA_SLICE_MACHINE_ID: `slice-rnt-machine-${process.pid}`,
    ARROBA_SLICE_MACHINE_ALIAS: targetDaemonAlias,
    ARROBA_SLICE_ROOT: path.join(root, "slice-root"),
  }
}

async function runSliceDockerScenarios({ options, root, ports }) {
  const targetDaemonAlias = `rnt-slice-${process.pid}`
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const env = sliceDockerEnv({ root, ports, targetDaemonAlias })
  let succeeded = false
  try {
    await runLogged("bash", [sliceDockerScript, "provision"], { env })
    await waitForTcpPort(ports.relayPort)
    await waitForRelayTarget(relayUrl, env.ARROBA_SLICE_RELAY_TOKEN ?? "slice-local", targetDaemonAlias)

    const scenarios = []
    for (const provider of options.providers) {
      scenarios.push(await runProviderScenario({
        provider,
        root,
        relayUrl,
        relayToken: env.ARROBA_SLICE_RELAY_TOKEN ?? "slice-local",
        targetDaemonAlias,
        workspace: repoRoot,
        worktree: repoRoot,
        nativeEnv: {
          ARROBA_CODEX_KERNEL_SERVER_PORT_RANGE: env.ARROBA_SLICE_CODEX_PORT_RANGE,
          ARROBA_CODEX_KERNEL_SERVER_BIND_HOST: "0.0.0.0",
        },
      }))
    }
    console.log(JSON.stringify({
      status: "ok",
      mode: "remote-native-tui-slice-local-docker-drill",
      relayUrl,
      targetDaemonAlias,
      providers: options.providers,
      scenarios,
    }, null, 2))
    succeeded = true
  } finally {
    if (succeeded || !options.keepArtifactsOnFailure) {
      await runLogged("bash", [sliceDockerScript, "destroy"], { env }).catch((error) => {
        console.error(`slice cleanup failed: ${error.message}`)
      })
    } else {
      console.error(`remote native TUI slice drill artifacts kept at ${root}; Docker slice ${env.ARROBA_SLICE_NAME} left running`)
    }
  }
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
  const ports = await makeAvailablePorts({ includeSliceRanges: options.sliceLocalDocker })
  if (options.sliceLocalDocker) {
    await mkdir(root, { recursive: true })
    await runSliceDockerScenarios({ options, root, ports })
    return
  }
  const relayToken = `remote-native-token-${process.pid}`
  const relayUrl = `ws://127.0.0.1:${ports.relayPort}`
  const homeKernelUrl = `ws://127.0.0.1:${ports.kernelPort}`
  const targetDaemonAlias = `remote-native-home-${process.pid}`
  const workspace = repoRoot
  const worktree = repoRoot
  const homeDir = path.join(root, "home")
  const xdgConfigHome = path.join(root, "xdg-config")
  const xdgStateHome = path.join(root, "xdg-state")
  const xdgDataHome = path.join(root, "xdg-data")
  const xdgCacheHome = path.join(root, "xdg-cache")
  let relay = null
  let kernel = null
  let succeeded = false
  try {
    await mkdir(root, { recursive: true })
    await mkdir(homeDir, { recursive: true })
    await mkdir(xdgConfigHome, { recursive: true })
    await mkdir(xdgStateHome, { recursive: true })
    await mkdir(xdgDataHome, { recursive: true })
    await mkdir(xdgCacheHome, { recursive: true })
    await access(path.join(realHomeDir, ".claude"))
      .then(() => symlink(path.join(realHomeDir, ".claude"), path.join(homeDir, ".claude"), "dir"))
      .catch(() => {})
    await access(path.join(realHomeDir, ".claude.json"))
      .then(() => symlink(path.join(realHomeDir, ".claude.json"), path.join(homeDir, ".claude.json")))
      .catch(() => {})
    relay = spawn(relayBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_RELAY_HOST: "127.0.0.1",
        ARROBA_RELAY_PORT: String(ports.relayPort),
        ARROBA_RELAY_TOKEN: relayToken,
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForTcpPort(ports.relayPort)
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
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForLocalDaemon(homeKernelUrl, workspace, worktree)
    await waitForRelayTarget(relayUrl, relayToken, targetDaemonAlias)

    const scenarios = []
    for (const provider of options.providers) {
      scenarios.push(await runProviderScenario({
        provider,
        root,
        relayUrl,
        relayToken,
        targetDaemonAlias,
        workspace,
        worktree,
      }))
    }

    console.log(JSON.stringify({
      status: "ok",
      mode: "remote-native-tui-relay-drill",
      relayUrl,
      homeKernelUrl,
      targetDaemonAlias,
      providers: options.providers,
      scenarios,
    }, null, 2))
    succeeded = true
  } finally {
    await terminateChild(kernel)
    await terminateChild(relay)
    if (succeeded || !options.keepArtifactsOnFailure) {
      await rm(root, { recursive: true, force: true }).catch(() => {})
    } else {
      console.error(`remote native TUI drill artifacts kept at ${root}`)
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
