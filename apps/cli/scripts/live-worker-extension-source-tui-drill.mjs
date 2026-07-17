#!/usr/bin/env node
import { spawn } from "node:child_process"
import { access, mkdtemp, mkdir, rm, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import {
  stripPtyControlSequences,
  writePtyFrameArtifacts,
} from "./lib/pty-terminal-frame.mjs"

const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const DEFAULT_TIMEOUT_MS = 90_000

const sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds))

function parseArgs(argv) {
  const options = {
    workspace: repoRoot,
    worktree: repoRoot,
    timeoutMs: DEFAULT_TIMEOUT_MS,
    buildRust: true,
    preserveOnSuccess: true,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]
    if (argument === "--") continue
    else if (argument === "--workspace") options.workspace = path.resolve(argv[++index])
    else if (argument === "--worktree") options.worktree = path.resolve(argv[++index])
    else if (argument === "--timeout-ms") options.timeoutMs = Number(argv[++index])
    else if (argument === "--skip-rust-build") options.buildRust = false
    else if (argument === "--discard-artifacts-on-success") options.preserveOnSuccess = false
    else if (argument === "--help" || argument === "-h") options.help = true
    else throw new Error(`unknown argument: ${argument}`)
  }
  return options
}

function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, "-")
}

async function reserveFreePort(excluded) {
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const server = net.createServer()
    await new Promise((resolve, reject) => {
      server.once("error", reject)
      server.listen(0, "127.0.0.1", resolve)
    })
    const address = server.address()
    const port = typeof address === "object" && address ? address.port : 0
    await new Promise((resolve, reject) => {
      server.close((error) => (error ? reject(error) : resolve()))
    })
    if (port && !excluded.has(port)) {
      excluded.add(port)
      return port
    }
  }
  throw new Error("could not reserve a unique free local port")
}

async function makePorts() {
  const excluded = new Set()
  return {
    relay: await reserveFreePort(excluded),
    homeKernel: await reserveFreePort(excluded),
    workerKernel: await reserveFreePort(excluded),
    homeMcp: await reserveFreePort(excluded),
    workerMcp: await reserveFreePort(excluded),
    homeOpenCode: await reserveFreePort(excluded),
    workerOpenCode: await reserveFreePort(excluded),
    homeCodex: await reserveFreePort(excluded),
    workerCodex: await reserveFreePort(excluded),
  }
}

function kernelEnv({
  runtimeDir,
  ports,
  relayToken,
  daemonId,
  machineId,
  kernelPort,
  mcpPort,
  openCodePort,
  codexPort,
}) {
  const prefix = path.join(runtimeDir, daemonId)
  const homeDir = `${prefix}-home`
  return {
    ...process.env,
    HOME: homeDir,
    ARROBA_HOME: path.join(homeDir, ".arroba"),
    ARROBA_CAPABILITY_ISOLATION_ROOT: `${prefix}-capabilities`,
    ARROBA_KERNEL_PORT: String(kernelPort),
    ARROBA_MCP_PORT: String(mcpPort),
    ARROBA_OPENCODE_PORT: String(openCodePort),
    ARROBA_CODEX_PORT: String(codexPort),
    ARROBA_RELAY_URL: `ws://127.0.0.1:${ports.relay}`,
    ARROBA_RELAY_TOKEN: relayToken,
    ARROBA_DAEMON_ID: daemonId,
    ARROBA_MACHINE_ID: machineId,
    ARROBA_ACCEPT_REMOTE_LEASES: "1",
    ARROBA_DAEMON_SOCKET: `${prefix}.sock`,
    ARROBA_SESSION_HISTORY_DIR: `${prefix}-history`,
    ARROBA_PROVIDER_DEV_STUB: "1",
    XDG_CONFIG_HOME: `${prefix}-config`,
    XDG_STATE_HOME: `${prefix}-state`,
    XDG_CACHE_HOME: `${prefix}-cache`,
    ARROBA_TEST_TUI: "1",
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
    child.once("error", reject)
    child.once("close", (code, signal) => resolve({ code, signal, stdout, stderr }))
  })
}

async function ensureRustBinary(binaryPath, manifestPath, binaryName, buildRust) {
  if (buildRust) {
    const built = await run("cargo", ["build", "--manifest-path", manifestPath, "--bin", binaryName])
    if (built.code !== 0) throw new Error(`${binaryName} build failed\n${built.stdout}\n${built.stderr}`)
  }
  await access(binaryPath).catch(() => {
    throw new Error(`missing ${binaryName} binary at ${binaryPath}`)
  })
  return binaryPath
}

async function ensureJavaScriptBuild() {
  const built = await run("pnpm", ["--filter", "@arroba/cli", "run", "build"])
  if (built.code !== 0) {
    throw new Error(`kernel-client/CLI build failed\n${built.stdout}\n${built.stderr}`)
  }
}

function spawnLogged(command, args, options, log) {
  const child = spawn(command, args, { ...options, stdio: ["ignore", "pipe", "pipe"] })
  child.stdout.on("data", (chunk) => { log.stdout += chunk.toString() })
  child.stderr.on("data", (chunk) => { log.stderr += chunk.toString() })
  return child
}

async function stopChild(child) {
  if (!child || child.exitCode !== null || child.signalCode !== null) return
  child.kill("SIGTERM")
  const exited = await Promise.race([
    new Promise((resolve) => child.once("exit", () => resolve(true))),
    sleep(5_000).then(() => false),
  ])
  if (!exited && child.exitCode === null && child.signalCode === null) child.kill("SIGKILL")
}

function unwrapVariant(response, ...variants) {
  for (const variant of variants) {
    if (response?.[variant] != null) return response[variant]
  }
  return response
}

async function waitForKernel(client) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
    try {
      await client.send({ ListSessions: null })
      return
    } catch (error) {
      lastError = error
      await sleep(250)
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
}

async function waitForRemoteKernel(client, machineId, kernelId) {
  let last = []
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const response = unwrapVariant(
      await client.send({ ListRemoteMachineKernels: { machine_ref: machineId } }),
      "RemoteMachineKernelsListed",
    )
    last = response.kernels ?? []
    const kernel = last.find((candidate) => candidate.kernel_id === kernelId)
    if (kernel?.accepting_remote_leases === true) return kernel
    await sleep(500)
  }
  throw new Error(`worker kernel ${kernelId} did not become ready: ${JSON.stringify(last)}`)
}

async function waitForRemotePlacement(client, sessionId, agentId, workerKernelId) {
  let last = null
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const response = unwrapVariant(
      await client.send({ GetSessionState: { session_id: sessionId } }),
      "SessionState",
      "SessionStateLoaded",
    )
    last = response.session?.agents?.find((agent) => agent.id === agentId) ?? null
    if (last?.remote_execution?.worker_kernel_id === workerKernelId) return last
    await sleep(250)
  }
  throw new Error(`agent did not move to ${workerKernelId}: ${JSON.stringify(last)}`)
}

async function waitForSocket(socketPath) {
  let lastError = null
  for (let attempt = 0; attempt < 80; attempt += 1) {
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
  throw new Error(`TUI automation socket did not become ready: ${lastError?.message ?? lastError}`)
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
      else deferred.reject(new Error(response.error ?? "automation request failed"))
    }
  })
  return {
    send(action, fields = {}) {
      const id = nextId++
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject })
        socket.write(`${JSON.stringify({ id, action, ...fields })}\n`)
      })
    },
    close() {
      socket.destroy()
    },
  }
}

async function waitForSnapshot(automation, predicate, label, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    last = await automation.send("snapshot")
    if (predicate(last)) return last
    await sleep(100)
  }
  throw new Error(`timed out waiting for ${label}: ${JSON.stringify(last?.footer ?? last)}`)
}

function mcpConfig(name) {
  return {
    name,
    transport: {
      type: "stdio",
      command: "/usr/bin/true",
      args: [],
      env: {},
      env_vars: [],
    },
    enabled: true,
    required: false,
    tools: {},
  }
}

async function installGlobalMcp(client, name) {
  await client.send({
    InstallMcpServer: {
      workspace_id: null,
      config: mcpConfig(name),
    },
  })
}

function assertIsolatedCatalog(catalog, homeKernelId, workerKernelId) {
  const entries = catalog.entries ?? []
  const has = (source, name) => entries.some((entry) => entry.source === source && entry.kind === "mcp" && entry.name === name)
  if (!has("home", "home_browser") || has("home", "worker_browser")) {
    throw new Error(`home extension catalog is not isolated: ${JSON.stringify(entries)}`)
  }
  if (!has("worker", "worker_browser") || has("worker", "home_browser")) {
    throw new Error(`worker extension catalog is not isolated: ${JSON.stringify(entries)}`)
  }
  if (catalog.home_kernel_id !== homeKernelId || catalog.worker_kernel_id !== workerKernelId) {
    throw new Error(`extension catalog kernel identity mismatch: ${JSON.stringify(catalog)}`)
  }
}

async function launchTui({
  label,
  columns,
  rows,
  homeEnv,
  homeKernelUrl,
  sessionId,
  workspace,
  worktree,
  runtimeDir,
  timeoutMs,
}) {
  const automationSocket = path.join(os.tmpdir(), `arroba-ext-${label}-${process.pid}.sock`)
  const child = spawn(process.env.PYTHON ?? "python3", [
    path.join(scriptDir, "lib", "pty-bridge.py"),
    "--columns", String(columns),
    "--rows", String(rows),
    "--",
    "bun", path.join(repoRoot, "apps/cli/dist/index.js"),
    "--kernel-url", homeKernelUrl,
    "--automation-socket", automationSocket,
    "--session", sessionId,
    "--workspace", workspace,
    "--worktree", worktree,
    "--provider", "dev-stub",
    "--model", "worker-extension-source-drill",
    "--client-id", `worker-extension-${label}-${process.pid}`,
  ], {
    cwd: repoRoot,
    env: homeEnv,
    stdio: ["pipe", "pipe", "pipe"],
  })
  const rawChunks = []
  const stderrChunks = []
  const rawText = () => Buffer.concat(rawChunks).toString("utf8")
  const stderrText = () => Buffer.concat(stderrChunks).toString("utf8")
  child.stdout.on("data", (chunk) => { rawChunks.push(Buffer.from(chunk)) })
  child.stderr.on("data", (chunk) => { stderrChunks.push(Buffer.from(chunk)) })
  try {
    await waitForSocket(automationSocket)
  } catch (error) {
    await stopChild(child)
    await rm(automationSocket, { force: true }).catch(() => {})
    throw new Error(`${error.message}\nPTY stderr:\n${stderrText()}\nPTY tail:\n${stripPtyControlSequences(rawText()).slice(-4000)}`)
  }
  const automation = createAutomationClient(automationSocket)
  await automation.send("ping")
  await waitForSnapshot(automation, (snapshot) => snapshot.session?.id === sessionId, `${label} TUI attach`, timeoutMs)
  await sleep(1_200)
  return {
    automation,
    automationSocket,
    child,
    columns,
    rows,
    label,
    runtimeDir,
    getRaw: rawText,
    getRawBuffer: () => Buffer.concat(rawChunks),
    getStderr: stderrText,
  }
}

async function typeCommand(tui, command, footerMarker, timeoutMs, commandLog) {
  // A second CLI may restore the session's last prompt draft. Clear it with real
  // terminal delete keystrokes before typing the next command through the PTY.
  tui.child.stdin.write("\u007f".repeat(256))
  await sleep(150)
  tui.child.stdin.write(`${command}\r`)
  const snapshot = await waitForSnapshot(
    tui.automation,
    (candidate) => JSON.stringify(candidate.footer ?? "").includes(footerMarker),
    `${tui.label} footer ${footerMarker}`,
    timeoutMs,
  )
  await sleep(150)
  commandLog.push({ tui: tui.label, command, footer: snapshot.footer })
  return snapshot
}

async function captureFrame(tui, artifactDir, name) {
  tui.child.kill("SIGUSR1")
  await sleep(200)
  tui.child.kill("SIGUSR1")
  await sleep(300)
  return await writePtyFrameArtifacts({
    raw: tui.getRaw(),
    outputDir: path.join(artifactDir, "frames"),
    name,
    columns: tui.columns,
    rows: tui.rows,
    screenshotColumns: tui.columns - 1,
    screenshotRows: tui.rows - 6,
  })
}

async function closeTui(tui, artifactDir) {
  if (!tui) return
  await tui.automation.send("exit").catch(() => {})
  tui.automation.close()
  await stopChild(tui.child)
  await mkdir(path.join(artifactDir, "pty"), { recursive: true })
  await writeFile(path.join(artifactDir, "pty", `${tui.label}.raw.log`), tui.getRawBuffer())
  await writeFile(path.join(artifactDir, "pty", `${tui.label}.normalized.log`), stripPtyControlSequences(tui.getRaw()), "utf8")
  await writeFile(path.join(artifactDir, "pty", `${tui.label}.stderr.log`), tui.getStderr(), "utf8")
  await rm(tui.automationSocket, { force: true }).catch(() => {})
}

function requireText(frameArtifact, expected, label) {
  const normalized = frameArtifact.frame.text.replace(/\s+/g, " ")
  if (!normalized.includes(expected)) {
    throw new Error(`${label} missing '${expected}' in captured PTY frame\n${frameArtifact.frame.text}`)
  }
  return { check: label, expected, passed: true }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  if (options.help) {
    console.log([
      "Usage: node apps/cli/scripts/live-worker-extension-source-tui-drill.mjs [options]",
      "  --workspace <path>",
      "  --worktree <path>",
      `  --timeout-ms ${DEFAULT_TIMEOUT_MS}`,
      "  --skip-rust-build",
      "  --discard-artifacts-on-success",
    ].join("\n"))
    return
  }

  const artifactDir = path.join(repoRoot, ".artifacts", "live-worker-extension-source-tui-drill", nowStamp())
  const runtimeDir = await mkdtemp(path.join(os.tmpdir(), "arroba-worker-ext-tui-"))
  await prepareDrillArtifacts(artifactDir)
  const ports = await makePorts()
  const relayToken = `worker-extension-source-${process.pid}-${Date.now()}`
  const homeKernelId = `home-extension-${process.pid}`
  const workerKernelId = `worker-extension-${process.pid}`
  const homeMachineId = `home-extension-machine-${process.pid}`
  const workerMachineId = `worker-extension-machine-${process.pid}`
  const homeEnv = kernelEnv({
    runtimeDir,
    ports,
    relayToken,
    daemonId: homeKernelId,
    machineId: homeMachineId,
    kernelPort: ports.homeKernel,
    mcpPort: ports.homeMcp,
    openCodePort: ports.homeOpenCode,
    codexPort: ports.homeCodex,
  })
  const workerEnv = kernelEnv({
    runtimeDir,
    ports,
    relayToken,
    daemonId: workerKernelId,
    machineId: workerMachineId,
    kernelPort: ports.workerKernel,
    mcpPort: ports.workerMcp,
    openCodePort: ports.workerOpenCode,
    codexPort: ports.workerCodex,
  })
  await Promise.all([
    homeEnv.HOME,
    homeEnv.ARROBA_HOME,
    homeEnv.ARROBA_CAPABILITY_ISOLATION_ROOT,
    workerEnv.HOME,
    workerEnv.ARROBA_HOME,
    workerEnv.ARROBA_CAPABILITY_ISOLATION_ROOT,
  ].map((directory) => mkdir(directory, { recursive: true })))
  const relayEnv = {
    ...process.env,
    ARROBA_RELAY_HOST: "127.0.0.1",
    ARROBA_RELAY_PORT: String(ports.relay),
    ARROBA_RELAY_TOKEN: relayToken,
  }
  const homeKernelUrl = `ws://127.0.0.1:${ports.homeKernel}`
  const workerKernelUrl = `ws://127.0.0.1:${ports.workerKernel}`
  const logs = {
    relay: { stdout: "", stderr: "" },
    home: { stdout: "", stderr: "" },
    worker: { stdout: "", stderr: "" },
  }
  const children = { relay: null, home: null, worker: null }
  let homeClient = null
  let workerClient = null
  let wideTui = null
  let narrowTui = null
  let sessionId = null
  let passed = false
  let failure = null
  const commandLog = []
  const captures = []
  const visualReview = []

  try {
    await ensureJavaScriptBuild()
    const relayBinary = await ensureRustBinary(
      path.join(repoRoot, "target/debug/arroba-relay"),
      path.join(repoRoot, "apps/relay/Cargo.toml"),
      "arroba-relay",
      options.buildRust,
    )
    const kernelBinary = await ensureRustBinary(
      path.join(repoRoot, "target/debug/arroba-kernel"),
      path.join(repoRoot, "apps/kernel/Cargo.toml"),
      "arroba-kernel",
      options.buildRust,
    )
    children.relay = spawnLogged(relayBinary, [], { cwd: repoRoot, env: relayEnv }, logs.relay)
    children.home = spawnLogged(kernelBinary, [], { cwd: repoRoot, env: homeEnv }, logs.home)
    children.worker = spawnLogged(kernelBinary, [], { cwd: repoRoot, env: workerEnv }, logs.worker)

    const { LocalIpcClient } = await import("../../../packages/kernel-client/dist/ipc.js")
    homeClient = new LocalIpcClient(homeKernelUrl)
    workerClient = new LocalIpcClient(workerKernelUrl)
    await waitForKernel(homeClient)
    await waitForKernel(workerClient)
    await waitForRemoteKernel(homeClient, workerMachineId, workerKernelId)
    await installGlobalMcp(homeClient, "home_browser")
    await installGlobalMcp(workerClient, "worker_browser")

    const created = unwrapVariant(await homeClient.send({
      CreateSession: {
        workspace_id: options.workspace,
        worktree_id: options.worktree,
      },
    }), "SessionCreated")
    sessionId = created.session.id
    const spawned = unwrapVariant(await homeClient.send({
      SpawnAgent: {
        session_id: sessionId,
        provider: "dev-stub",
        alias: "extension-worker",
        model: "worker-extension-source-drill",
        effort: "low",
        worktree_id: options.worktree,
        kernel_ref: workerKernelId,
      },
    }), "AgentSpawned")
    const agent = await waitForRemotePlacement(homeClient, sessionId, spawned.agent.id, workerKernelId)
    const agentRef = agent.agent_ref
    const catalogResponse = unwrapVariant(await homeClient.send({
      ListAgentExtensionCatalog: { agent_ref: agentRef, source: "all" },
    }), "AgentExtensionCatalogListed")
    const catalog = catalogResponse.catalog
    assertIsolatedCatalog(catalog, homeKernelId, workerKernelId)

    wideTui = await launchTui({
      label: "wide-120x36",
      columns: 120,
      rows: 36,
      homeEnv,
      homeKernelUrl,
      sessionId,
      workspace: options.workspace,
      worktree: options.worktree,
      runtimeDir,
      timeoutMs: options.timeoutMs,
    })
    await typeCommand(wideTui, `/extension catalog ${agentRef} --from all`, `showing all extension catalog for ${agentRef}`, options.timeoutMs, commandLog)
    const catalogCapture = await captureFrame(wideTui, artifactDir, "01-wide-catalog-all")
    captures.push(catalogCapture)
    visualReview.push(requireText(catalogCapture, "home:mcp:home_browser", "catalog shows home source"))
    visualReview.push(requireText(catalogCapture, "worker:mcp:worker_browser", "catalog shows worker source"))
    visualReview.push(requireText(catalogCapture, `worker kernel: ${workerKernelId} (available)`, "catalog shows resolved worker kernel"))

    await typeCommand(wideTui, `/extension grant mcp ${agentRef} worker_browser --from worker`, `granted MCP worker_browser to ${agentRef} from worker`, options.timeoutMs, commandLog)
    await typeCommand(wideTui, `/extension grant mcp ${agentRef} home_browser --from home --confirm-home-proxy`, `granted MCP home_browser to ${agentRef} from home`, options.timeoutMs, commandLog)
    await typeCommand(wideTui, `/extension grants mcp ${agentRef} --from all`, `showing all mcp grants for ${agentRef}`, options.timeoutMs, commandLog)
    const mixedCapture = await captureFrame(wideTui, artifactDir, "02-wide-mixed-grants")
    captures.push(mixedCapture)
    visualReview.push(requireText(mixedCapture, "source=home", "mixed grants show home source"))
    visualReview.push(requireText(mixedCapture, "runtime=home-proxy", "mixed grants show home runtime"))
    visualReview.push(requireText(mixedCapture, "source=worker", "mixed grants show worker source"))
    visualReview.push(requireText(mixedCapture, "runtime=worker-local", "mixed grants show worker runtime"))

    await typeCommand(wideTui, `/extension grants mcp ${agentRef} --from worker`, `showing worker mcp grants for ${agentRef}`, options.timeoutMs, commandLog)
    const filteredCapture = await captureFrame(wideTui, artifactDir, "03-wide-worker-filter")
    captures.push(filteredCapture)
    visualReview.push(requireText(filteredCapture, "MCP grants from worker", "worker filter is explicit"))
    visualReview.push(requireText(filteredCapture, `kernel=${workerKernelId}`, "worker grant shows kernel id"))

    await typeCommand(wideTui, `/extension revoke mcp ${agentRef} worker_browser --from worker`, `revoked MCP worker_browser from ${agentRef} from worker`, options.timeoutMs, commandLog)
    await typeCommand(wideTui, `/extension grants mcp ${agentRef} --from all`, `showing all mcp grants for ${agentRef}`, options.timeoutMs, commandLog)
    const revokedCapture = await captureFrame(wideTui, artifactDir, "04-wide-worker-revoked")
    captures.push(revokedCapture)
    visualReview.push(requireText(revokedCapture, "source=home", "home grant remains after worker revoke"))

    const stateAfterWide = unwrapVariant(await homeClient.send({ GetSessionState: { session_id: sessionId } }), "SessionState", "SessionStateLoaded")
    const persistedAfterWide = stateAfterWide.session.agents.find((candidate) => candidate.id === agent.id)
    const persistedKeys = (persistedAfterWide.extension_grants ?? []).map((grant) => `${grant.source ?? "home"}:${grant.kind}:${grant.name}`).sort()
    if (JSON.stringify(persistedKeys) !== JSON.stringify(["home:mcp:home_browser"])) {
      throw new Error(`persisted grant identity mismatch after worker revoke: ${JSON.stringify(persistedKeys)}`)
    }

    await closeTui(wideTui, artifactDir)
    wideTui = null

    narrowTui = await launchTui({
      label: "narrow-80x30",
      columns: 80,
      rows: 30,
      homeEnv,
      homeKernelUrl,
      sessionId,
      workspace: options.workspace,
      worktree: options.worktree,
      runtimeDir,
      timeoutMs: options.timeoutMs,
    })
    await typeCommand(narrowTui, `/extension grant mcp ${agentRef} worker_browser --from worker`, `granted MCP worker_browser to ${agentRef} from worker`, options.timeoutMs, commandLog)
    await typeCommand(narrowTui, `/extension grants mcp ${agentRef} --from all`, `showing all mcp grants for ${agentRef}`, options.timeoutMs, commandLog)
    const narrowCapture = await captureFrame(narrowTui, artifactDir, "05-narrow-mixed-grants")
    captures.push(narrowCapture)
    visualReview.push(requireText(narrowCapture, "source=home", "narrow frame keeps home source legible"))
    visualReview.push(requireText(narrowCapture, "source=worker", "narrow frame keeps worker source legible"))
    visualReview.push({
      check: "narrow frame respects terminal width",
      expected: "all rows <= 80 cells",
      passed: narrowCapture.frame.lines.every((line) => Array.from(line).length <= 80),
    })
    await typeCommand(narrowTui, `/extension revoke mcp ${agentRef} worker_browser --from worker`, `revoked MCP worker_browser from ${agentRef} from worker`, options.timeoutMs, commandLog)

    const finalState = unwrapVariant(await homeClient.send({ GetSessionState: { session_id: sessionId } }), "SessionState", "SessionStateLoaded")
    const finalAgent = finalState.session.agents.find((candidate) => candidate.id === agent.id)
    const finalKeys = (finalAgent.extension_grants ?? []).map((grant) => `${grant.source ?? "home"}:${grant.kind}:${grant.name}`).sort()
    if (JSON.stringify(finalKeys) !== JSON.stringify(["home:mcp:home_browser"])) {
      throw new Error(`final persisted grant identity mismatch: ${JSON.stringify(finalKeys)}`)
    }

    const result = {
      status: "passed",
      sessionId,
      agentId: agent.id,
      agentRef,
      homeKernelId,
      workerKernelId,
      persistedGrantKeys: finalKeys,
      commands: commandLog,
      captures: captures.map((capture) => ({
        textPath: capture.textPath,
        pngPath: capture.pngPath,
        screenshot: capture.screenshot,
      })),
      visualReview,
      validation: {
        rawPtyCaptured: true,
        commandsTypedThroughPtyStdin: true,
        automationUsedForSynchronizationOnly: true,
        pngsRasterizedFromCapturedPtyFrames: true,
        pngCrop: "exact transcript viewport crop; one scrollbar column and six prompt/footer rows omitted",
        persistedIpcStateVerified: true,
      },
    }
    await writeFile(path.join(artifactDir, "result.json"), `${JSON.stringify(result, null, 2)}\n`, "utf8")
    passed = true
    console.log(JSON.stringify({ status: "passed", artifactDir, captures: result.captures }, null, 2))
  } catch (error) {
    failure = error
    await writeFile(path.join(artifactDir, "result.json"), `${JSON.stringify({
      status: "failed",
      error: error instanceof Error ? error.message : String(error),
      commands: commandLog,
      visualReview,
    }, null, 2)}\n`, "utf8").catch(() => {})
    throw error
  } finally {
    await closeTui(narrowTui, artifactDir).catch(() => {})
    await closeTui(wideTui, artifactDir).catch(() => {})
    if (sessionId) await homeClient?.send({ EndSession: { session_id: sessionId } }).catch(() => {})
    await homeClient?.close().catch(() => {})
    await workerClient?.close().catch(() => {})
    await stopChild(children.home)
    await stopChild(children.worker)
    await stopChild(children.relay)
    await mkdir(path.join(artifactDir, "runtime-logs"), { recursive: true }).catch(() => {})
    for (const [name, log] of Object.entries(logs)) {
      await writeFile(path.join(artifactDir, "runtime-logs", `${name}.stdout.log`), log.stdout, "utf8").catch(() => {})
      await writeFile(path.join(artifactDir, "runtime-logs", `${name}.stderr.log`), log.stderr, "utf8").catch(() => {})
    }
    await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
    await finalizeDrillArtifacts({
      rootDir: artifactDir,
      passed,
      preserveOnFailure: true,
      preserveOnSuccess: options.preserveOnSuccess,
      failure,
      metadata: {
        drill: "live-worker-extension-source-tui",
        sessionId,
        homeKernelId,
        workerKernelId,
        commands: commandLog,
        visualReview,
      },
    })
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
