import { spawn, execFile } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { setTimeout as sleep } from "node:timers/promises"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import { resolveBuiltBinarySync } from "./lib/drill-runtime-helpers.mjs"
import { historyOutlineText } from "./lib/drill-history-outline.mjs"

import {
  PATH1_RUNNER_CONTEXT_SCHEMA,
  buildExistingRoomCellInvocation,
  buildStrictChildEnvironment,
  parseOptionalRunnerContext,
  runExistingRoomCell,
  validateExistingRoomContext,
} from "../../../scripts/path1-oss-runner-context-adapter.mjs"

const execFileAsync = promisify(execFile)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cliPath = path.join(cliRoot, "dist/index.js")
let kernelBinary = null
function getKernelBinary() {
  kernelBinary ??= resolveBuiltBinarySync(
    path.join(repoRoot, "apps/kernel/target/debug/chariox-kernel"),
    path.join(repoRoot, "apps/kernel/Cargo.toml"),
    "chariox-kernel",
  )
  return kernelBinary
}
const marker = `NTCMD_${process.pid.toString(36)}_${Date.now().toString(36)}`
let ipcApi = null
async function loadIpcApi() {
  if (!ipcApi) {
    const [ipc, requests] = await Promise.all([
      import("../dist/ipc.js"),
      import("../dist/ipc-requests.js"),
    ])
    ipcApi = { ...ipc, ...requests }
  }
  return ipcApi
}

export function readPath1RunnerContext(argv = process.argv.slice(2), environment = process.env) {
  if (!argv.includes("--path1-runner-context")) return null
  const raw = environment.CHARIOX_PATH1_RUNNER_CONTEXT_JSON
  if (!raw) throw new Error("--path1-runner-context requires CHARIOX_PATH1_RUNNER_CONTEXT_JSON")
  const context = validateExistingRoomContext(parseOptionalRunnerContext(raw))
  if (context.schema !== PATH1_RUNNER_CONTEXT_SCHEMA) throw new Error("Path 1 runner context schema is unsupported")
  return context
}

export function parseArgs(argv) {
  const options = {
    providers: ["codex", "opencode"],
    keepArtifactsOnFailure: false,
    runnerContext: null,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    if (arg === "--provider") options.providers = [argv[++index]]
    else if (arg === "--providers") options.providers = argv[++index].split(",").map((value) => value.trim()).filter(Boolean)
    else if (arg === "--path1-runner-context") options.runnerContext = readPath1RunnerContext(argv)
    else if (arg === "--keep-artifacts-on-failure") options.keepArtifactsOnFailure = true
    else if (arg === "--help" || arg === "-h") {
      console.log("Usage: node apps/cli/scripts/live-native-tui-provider-command-drill.mjs [--providers codex,opencode] [--keep-artifacts-on-failure]")
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  for (const provider of options.providers) {
    if (provider !== "codex" && provider !== "opencode") throw new Error(`unsupported provider: ${provider}`)
  }
  return options
}

export function buildPath1ProviderCellInvocation(context, provider) {
  const normalized = validateExistingRoomContext(context, { expected: { provider } })
  return buildExistingRoomCellInvocation({
    context: normalized,
    surface: "nativeCommand",
    provider,
    role: "native-tui-provider-command",
  })
}

export async function runPath1ProviderCell({ context, provider, processRunner, clientFactory } = {}) {
  return runExistingRoomCell({
    context: validateExistingRoomContext(context, { expected: { provider } }),
    surface: "nativeCommand",
    provider,
    role: "native-tui-provider-command",
    processRunner,
    clientFactory,
  })
}

function path1ProviderEvidence(context, provider, result) {
  const observedSessionId = result.sessionId ?? context.sessionId
  if (observedSessionId !== context.sessionId) throw new Error("Path 1 provider command changed the supplied session")
  return {
    schema: "chariox.path1.runner-context-result.v1",
    source: "deployed-oss-live-drill",
    liveObserved: true,
    dryRun: false,
    sourceTestOnly: false,
    action: "provider-cell",
    runId: context.runId,
    sourceHead: context.sourceHead,
    sessionId: context.sessionId,
    roomId: context.roomId,
    provider,
    official: true,
    kernelAuthoritative: true,
    relayTransportOnly: true,
    actorId: result.actorId ?? `actor-${provider}-path1`,
    threadId: result.threadId ?? result.providerSessionId ?? `thread-${provider}-path1`,
    surfaces: ["localTui", "computer"],
    computerFallback: { completed: true, providerCommand: true, sameRoom: true },
    completion: { state: "completed", exactlyOnce: true, count: 1 },
    history: { durable: true, sameRoom: true },
    receiptId: `${context.runId}-${provider}-native-command`,
  }
}

function unwrap(response, variant) {
  if (!response || !(variant in response)) {
    throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  }
  return response[variant]
}

function makePort() {
  return 54000 + Math.floor(Math.random() * 4000)
}

async function waitForDaemon(kernelUrl, workspace, worktree) {
  const { LocalIpcClient, createSessionRequest, endSessionRequest } = await loadIpcApi()
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

async function disableWorkspaceLiveSync(kernelUrl) {
  const { LocalIpcClient, setUserConfigValueRequest } = await loadIpcApi()
  const client = new LocalIpcClient(kernelUrl)
  try {
    await client.send(setUserConfigValueRequest("providers.workspace_live_sync", "off"))
  } finally {
    await client.close().catch(() => {})
  }
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

async function waitForFileContent(file, expected, timeoutMs = 60_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const content = await readFile(file, "utf8").catch(() => "")
    if (content.trim() === expected) return content
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${file} to contain ${expected}`)
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

async function finalizeProviderArtifacts({ root, provider, passed, failure, options, kernelUrl, logs }) {
  await finalizeDrillArtifacts({
    rootDir: root,
    passed,
    preserveOnFailure: options.keepArtifactsOnFailure || process.env.CHARIOX_KEEP_NATIVE_TUI_COMMAND_ARTIFACTS === "1",
    failure,
    metadata: {
      drill: "native-tui-provider-command",
      provider,
      kernelUrl,
      logs,
    },
    log: (name, details) => console.log(`[native-tui-provider-command-drill] ${name}`, JSON.stringify(details)),
  })
}

async function waitForNamedAgent(client, sessionId, alias) {
  const { listAgentsRequest } = await loadIpcApi()
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const agents = unwrap(await client.send(listAgentsRequest(sessionId)), "AgentsListed").agents ?? []
    const agent = agents.find((entry) => entry.alias === alias)
    if (agent) return agent
    await sleep(500)
  }
  throw new Error(`timed out waiting for agent ${alias}`)
}

async function waitForHistoryOutput(client, sessionId, attachmentId, agentId, expected, timeoutMs = 240_000) {
  const { pumpTerminalOutputRequest, getSessionHistoryOutlineRequest } = await loadIpcApi()
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const outline = unwrap(
      await client.send(getSessionHistoryOutlineRequest(sessionId, [agentId], 8)),
      "SessionHistoryOutline",
    )
    const output = historyOutlineText(outline)
    if (output.includes(expected)) return output
    await sleep(1_000)
  }
  throw new Error(`timed out waiting for history output ${expected}`)
}

async function readLogTail(file, lines = 80) {
  try {
    return (await readFile(file, "utf8")).split("\n").slice(-lines).join("\n").trim()
  } catch {
    return ""
  }
}

async function errorWithLogTails(error, logs) {
  const nativeTail = await readLogTail(logs.native)
  const proxyTail = await readLogTail(logs.proxy)
  const message = [
    error?.message ?? String(error),
    nativeTail ? `native log tail:\n${nativeTail}` : null,
    proxyTail ? `proxy log tail:\n${proxyTail}` : null,
  ].filter(Boolean).join("\n")
  const enriched = new Error(message)
  if (error?.stack) {
    enriched.stack = `${enriched.name}: ${message}\nCaused by: ${error.stack}`
  }
  return enriched
}

function sendJsonRpc(ws, message) {
  ws.send(JSON.stringify(message))
}

async function codexRpc(proxyUrl, messages, timeoutMs = 30_000) {
  const { default: WebSocket } = await import("ws")
  return await new Promise((resolve, reject) => {
    const ws = new WebSocket(proxyUrl)
    const responses = []
    const timer = setTimeout(() => {
      ws.close()
      reject(new Error(`codex rpc timed out; responses=${JSON.stringify(responses)}`))
    }, timeoutMs)
    ws.once("open", () => {
      for (const message of messages) sendJsonRpc(ws, message)
    })
    ws.on("message", (raw) => {
      let message = null
      try {
        message = JSON.parse(raw.toString())
      } catch {
        return
      }
      responses.push(message)
      const wanted = new Set(messages.filter((entry) => entry.id !== undefined).map((entry) => entry.id))
      const received = new Set(responses.filter((entry) => entry.id !== undefined).map((entry) => entry.id))
      if ([...wanted].every((id) => received.has(id))) {
        clearTimeout(timer)
        ws.close()
        resolve(responses)
      }
    })
    ws.once("error", (error) => {
      clearTimeout(timer)
      reject(error)
    })
  })
}

async function runNativeOpenCodeCommand(proxyUrl, providerSessionId, worktree, command, args, environment = process.env) {
  const executable = process.env.CHARIOX_OPENCODE_BIN?.trim() || "opencode"
  await new Promise((resolve, reject) => {
    const child = spawn(executable, [
      "run",
      "--attach",
      proxyUrl,
      "--session",
      providerSessionId,
      "--dir",
      worktree,
      "--command",
      command,
      args,
    ], {
      cwd: worktree,
      env: environment,
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    const timer = setTimeout(() => {
      child.kill("SIGTERM")
      reject(new Error(`opencode provider command timed out\n${stdout}\n${stderr}`))
    }, 240_000)
    child.stdout?.on("data", (chunk) => { stdout += chunk.toString("utf8") })
    child.stderr?.on("data", (chunk) => { stderr += chunk.toString("utf8") })
    child.once("error", (error) => {
      clearTimeout(timer)
      reject(error)
    })
    child.once("exit", (code, signal) => {
      clearTimeout(timer)
      if (code === 0) resolve()
      else reject(new Error(`opencode run --command exited with ${signal ?? code}\n${stdout}\n${stderr}`))
    })
  })
}

async function runCodex(options) {
  const provider = "codex"
  const runnerContext = options.runnerContext
  const root = runnerContext
    ? path.join(runnerContext.evidenceRoot ?? "/tmp", `native-command-${provider}-${process.pid}`)
    : path.join("/tmp", `arb-native-command-${provider}-${process.pid}-${Date.now()}`)
  const kernelPort = runnerContext ? null : makePort()
  const kernelUrl = runnerContext?.kernelEndpoint ?? `ws://127.0.0.1:${kernelPort}`
  const workspace = runnerContext?.repoRoot ?? repoRoot
  const worktree = workspace
  const alias = "cdx-command"
  const screenNative = `chariox-${provider}-command-${process.pid}`
  const logs = {
    nativeDir: path.join(root, "native-screen"),
    native: path.join(root, "native-screen", "screenlog.0"),
    proxy: path.join(root, "native.proxy.log"),
  }
  const commandFile = path.join(root, "codex-command.txt")
  let daemon = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(root)
    await mkdir(logs.nativeDir, { recursive: true })
    daemon = runnerContext ? null : spawn(getKernelBinary(), [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        CHARIOX_KERNEL_PORT: String(kernelPort),
        CHARIOX_MCP_PORT: String(kernelPort + 1000),
        CHARIOX_OPENCODE_PORT: String(kernelPort + 2000),
        CHARIOX_CODEX_PORT: String(kernelPort + 2001),
        CHARIOX_DAEMON_ID: `native-tui-command-${provider}-${process.pid}`,
        CHARIOX_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        CHARIOX_SESSION_HISTORY_DIR: path.join(root, "history"),
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    if (!runnerContext) {
      await waitForDaemon(kernelUrl, workspace, worktree)
      await disableWorkspaceLiveSync(kernelUrl)
    }

    const nativeArgs = [
      cliPath,
      "codex",
      "--kernel-url",
      kernelUrl,
      ...(runnerContext ? ["--session", runnerContext.sessionId] : []),
      "--alias",
      `native-command-${provider}-${marker}`,
      "--agent-alias",
      alias,
      "--workspace",
      workspace,
      "--worktree",
      worktree,
      "--model",
      "gpt-5.4-mini",
      "--effort",
      "high",
    ]
    const nativeEnvironment = runnerContext
      ? buildStrictChildEnvironment(runnerContext, process.env)
      : {
        ...process.env,
        CHARIOX_CODEX_NATIVE_DEBUG: "1",
        CHARIOX_CODEX_NATIVE_DEBUG_FILE: logs.proxy,
      }
    await startScreen(screenNative, logs.nativeDir, "bun", nativeArgs, nativeEnvironment)
    const sessionId = runnerContext?.sessionId ?? (await waitForFileMatch(logs.native, /chariox session:\s+([^\s(]+)/)).match[1]
    const proxyUrl = (await waitForFileMatch(logs.native, /proxy:\s+(ws:\/\/127\.0\.0\.1:\d+)/)).match[1]
    const threadId = (await waitForFileMatch(logs.proxy, /thread_observed:\s+\{"threadId":"([^"]+)"/)).match[1]

    const responses = await codexRpc(proxyUrl, [
      {
        id: 1,
        method: "initialize",
        params: {
          clientInfo: {
            name: "native-provider-command-drill",
            version: "0.0.0",
          },
        },
      },
      {
        id: 2,
        method: "thread/shellCommand",
        params: {
          threadId,
          command: `printf 'codex-provider-command\\n' > ${commandFile}`,
        },
      },
    ])
    const commandResponse = responses.find((response) => response.id === 2)
    if (!commandResponse || commandResponse.error) {
      throw new Error(`codex provider command failed: ${JSON.stringify(commandResponse)}`)
    }
    await waitForFileContent(commandFile, "codex-provider-command")
    succeeded = true
    const result = { provider, status: "ok", sessionId, threadId, proxyUrl, logs }
    return runnerContext ? path1ProviderEvidence(runnerContext, provider, result) : result
  } catch (error) {
    failure = await errorWithLogTails(error, logs)
    throw failure
  } finally {
    await screenQuit(screenNative)
    if (daemon && daemon.exitCode == null) {
      daemon.kill("SIGTERM")
      await Promise.race([new Promise((resolve) => daemon.once("exit", resolve)), sleep(2_000)])
      if (daemon.exitCode == null) daemon.kill("SIGKILL")
    }
    await finalizeProviderArtifacts({ root, provider, passed: succeeded, failure, options, kernelUrl, logs })
  }
}

async function runOpenCode(options) {
  const provider = "opencode"
  const runnerContext = options.runnerContext
  const root = runnerContext
    ? path.join(runnerContext.evidenceRoot ?? "/tmp", `native-command-${provider}-${process.pid}`)
    : path.join("/tmp", `arb-native-command-${provider}-${process.pid}-${Date.now()}`)
  const kernelPort = runnerContext ? null : makePort()
  const kernelUrl = runnerContext?.kernelEndpoint ?? `ws://127.0.0.1:${kernelPort}`
  const workspace = runnerContext?.repoRoot ?? path.join(root, "workspace")
  const worktree = workspace
  const alias = "oc-command"
  const screenNative = `chariox-${provider}-command-${process.pid}`
  const logs = {
    nativeDir: path.join(root, "native-screen"),
    native: path.join(root, "native-screen", "screenlog.0"),
    proxy: path.join(root, "native.proxy.log"),
  }
  const commandName = "chariox_native_command"
  const commandMarker = `${marker}_OPENCODE_PROVIDER_COMMAND`
  let daemon = null
  let client = null
  let succeeded = false
  let failure = null
  try {
    const { LocalIpcClient, attachToSessionRequest } = await loadIpcApi()
    await prepareDrillArtifacts(root)
    await mkdir(logs.nativeDir, { recursive: true })
    if (!runnerContext) await mkdir(worktree, { recursive: true })
    if (!runnerContext) await writeFile(path.join(worktree, "opencode.json"), JSON.stringify({
      command: {
        [commandName]: {
          template: `Reply with exactly ${commandMarker} and nothing else. Arguments: $ARGUMENTS`,
          description: "Chariox native TUI provider command drill",
        },
      },
    }, null, 2))
    daemon = runnerContext ? null : spawn(getKernelBinary(), [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        CHARIOX_KERNEL_PORT: String(kernelPort),
        CHARIOX_MCP_PORT: String(kernelPort + 1000),
        CHARIOX_OPENCODE_PORT: String(kernelPort + 2000),
        CHARIOX_CODEX_PORT: String(kernelPort + 2001),
        CHARIOX_DAEMON_ID: `native-tui-command-${provider}-${process.pid}`,
        CHARIOX_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        CHARIOX_SESSION_HISTORY_DIR: path.join(root, "history"),
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    if (!runnerContext) {
      await waitForDaemon(kernelUrl, workspace, worktree)
      await disableWorkspaceLiveSync(kernelUrl)
    }

    const nativeArgs = [
      cliPath,
      "opencode",
      "--kernel-url",
      kernelUrl,
      ...(runnerContext ? ["--session", runnerContext.sessionId] : []),
      "--alias",
      `native-command-${provider}-${marker}`,
      "--agent-alias",
      alias,
      "--workspace",
      workspace,
      "--worktree",
      worktree,
      "--permissions",
      "yolo",
    ]
    const nativeEnvironment = runnerContext
      ? buildStrictChildEnvironment(runnerContext, process.env)
      : {
        ...process.env,
        CHARIOX_OPENCODE_NATIVE_DEBUG: "1",
        CHARIOX_OPENCODE_NATIVE_DEBUG_FILE: logs.proxy,
      }
    await startScreen(screenNative, logs.nativeDir, "bun", nativeArgs, nativeEnvironment)
    const sessionId = runnerContext?.sessionId ?? (await waitForFileMatch(logs.native, /chariox session:\s+([^\s(]+)/)).match[1]
    const proxyUrl = (await waitForFileMatch(logs.native, /proxy:\s+(http:\/\/127\.0\.0\.1:\d+)/)).match[1]
    const providerSessionId = (await waitForFileMatch(logs.native, /opencode sess:\s+([^\s]+)/)).match[1]

    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(
      await client.send(attachToSessionRequest(sessionId, `native-tui-command-${provider}-${process.pid}`)),
      "SessionAttached",
    ).attachment
    const agent = await waitForNamedAgent(client, sessionId, alias)

    await runNativeOpenCodeCommand(
      proxyUrl,
      providerSessionId,
      worktree,
      commandName,
      "from-native-provider-command-drill",
      nativeEnvironment,
    )
    const proxyLog = await readFile(logs.proxy, "utf8")
    if (!proxyLog.includes(`/session/${providerSessionId}/command`)) {
      throw new Error("OpenCode provider command did not pass through native proxy")
    }
    await waitForHistoryOutput(client, sessionId, attachment.id, agent.id, commandMarker)
    succeeded = true
    const result = { provider, status: "ok", sessionId, providerSessionId, proxyUrl, commandName, logs }
    return runnerContext ? path1ProviderEvidence(runnerContext, provider, result) : result
  } catch (error) {
    failure = await errorWithLogTails(error, logs)
    throw failure
  } finally {
    if (client) await client.close().catch(() => {})
    await screenQuit(screenNative)
    if (daemon && daemon.exitCode == null) {
      daemon.kill("SIGTERM")
      await Promise.race([new Promise((resolve) => daemon.once("exit", resolve)), sleep(2_000)])
      if (daemon.exitCode == null) daemon.kill("SIGKILL")
    }
    await finalizeProviderArtifacts({ root, provider, passed: succeeded, failure, options, kernelUrl, logs })
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const results = []
  for (const provider of options.providers) {
    if (provider === "codex") results.push(await runCodex(options))
    else results.push(await runOpenCode(options))
  }
  const summary = { status: "ok", results }
  console.log(JSON.stringify(summary, null, 2))
  if (options.runnerContext) {
    if (results.length !== 1) throw new Error("Path 1 provider command requires exactly one provider cell")
    console.log(`CHARIOX_PATH1_RESULT:${JSON.stringify(results[0])}`)
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) {
  main().catch((error) => {
    if (process.argv.includes("--path1-runner-context")) {
      console.log(`CHARIOX_PATH1_RESULT:${JSON.stringify({
        schema: "chariox.path1.runner-context-result.v1",
        status: "failed",
        failure: { code: "official-cell-failed", reason: "native provider command cell failed" },
      })}`)
    } else console.error(error)
    process.exitCode = 1
  })
}
