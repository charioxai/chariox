import { spawn, execFile } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { setTimeout as sleep } from "node:timers/promises"

import WebSocket from "ws"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  endSessionRequest,
  getSessionHistoryRequest,
  listAgentsRequest,
  pumpTerminalOutputRequest,
} from "../dist/ipc-requests.js"

const execFileAsync = promisify(execFile)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cliPath = path.join(cliRoot, "dist/index.js")
const kernelBinary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
const marker = `NTCMD_${process.pid.toString(36)}_${Date.now().toString(36)}`

function parseArgs(argv) {
  const options = {
    providers: ["codex", "opencode"],
    keepArtifactsOnFailure: false,
  }
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--") continue
    if (arg === "--provider") options.providers = [argv[++index]]
    else if (arg === "--providers") options.providers = argv[++index].split(",").map((value) => value.trim()).filter(Boolean)
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

async function waitForNamedAgent(client, sessionId, alias) {
  for (let attempt = 0; attempt < 120; attempt += 1) {
    const agents = unwrap(await client.send(listAgentsRequest(sessionId)), "AgentsListed").agents ?? []
    const agent = agents.find((entry) => entry.alias === alias)
    if (agent) return agent
    await sleep(500)
  }
  throw new Error(`timed out waiting for agent ${alias}`)
}

async function waitForHistoryOutput(client, sessionId, attachmentId, agentId, expected, timeoutMs = 240_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const page = unwrap(await client.send(getSessionHistoryRequest(sessionId, 300, 100_000, null, agentId)), "SessionHistory")
    const output = page.entries
      .map((row) => row.entry)
      .filter((entry) => entry && entry.kind !== "user_prompt")
      .map((entry) => entry.text ?? "")
      .join("")
    if (output.includes(expected)) return output
    await sleep(1_000)
  }
  throw new Error(`timed out waiting for history output ${expected}`)
}

function sendJsonRpc(ws, message) {
  ws.send(JSON.stringify(message))
}

async function codexRpc(proxyUrl, messages, timeoutMs = 30_000) {
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

async function runNativeOpenCodeCommand(proxyUrl, providerSessionId, worktree, command, args) {
  const executable = process.env.ARROBA_OPENCODE_BIN?.trim() || "opencode"
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
      env: process.env,
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
  const root = path.join("/tmp", `arb-native-command-${provider}-${process.pid}-${Date.now()}`)
  const kernelPort = makePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const workspace = repoRoot
  const worktree = repoRoot
  const alias = "cdx-command"
  const screenNative = `arroba-${provider}-command-${process.pid}`
  const logs = {
    nativeDir: path.join(root, "native-screen"),
    native: path.join(root, "native-screen", "screenlog.0"),
    proxy: path.join(root, "native.proxy.log"),
  }
  const commandFile = path.join(root, "codex-command.txt")
  let daemon = null
  try {
    await mkdir(logs.nativeDir, { recursive: true })
    daemon = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_KERNEL_PORT: String(kernelPort),
        ARROBA_MCP_PORT: String(kernelPort + 1000),
        ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
        ARROBA_CODEX_PORT: String(kernelPort + 2001),
        ARROBA_DAEMON_ID: `native-tui-command-${provider}-${process.pid}`,
        ARROBA_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        ARROBA_SESSION_HISTORY_DIR: path.join(root, "history"),
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForDaemon(kernelUrl, workspace, worktree)

    await startScreen(screenNative, logs.nativeDir, "bun", [
      cliPath,
      "codex",
      "--kernel-url",
      kernelUrl,
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
    ], {
      ...process.env,
      ARROBA_CODEX_NATIVE_DEBUG: "1",
      ARROBA_CODEX_NATIVE_DEBUG_FILE: logs.proxy,
    })
    const sessionId = (await waitForFileMatch(logs.native, /arroba session:\s+([^\s(]+)/)).match[1]
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
    return { provider, status: "ok", sessionId, threadId, proxyUrl, logs }
  } finally {
    await screenQuit(screenNative)
    if (daemon && daemon.exitCode == null) {
      daemon.kill("SIGTERM")
      await Promise.race([new Promise((resolve) => daemon.once("exit", resolve)), sleep(2_000)])
      if (daemon.exitCode == null) daemon.kill("SIGKILL")
    }
    if (process.env.ARROBA_KEEP_NATIVE_TUI_COMMAND_ARTIFACTS === "1" || (options.keepArtifactsOnFailure && process.exitCode)) {
      console.log(JSON.stringify({ provider, artifactsKept: root }))
    } else {
      await rm(root, { recursive: true, force: true }).catch(() => {})
    }
  }
}

async function runOpenCode(options) {
  const provider = "opencode"
  const root = path.join("/tmp", `arb-native-command-${provider}-${process.pid}-${Date.now()}`)
  const kernelPort = makePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const workspace = path.join(root, "workspace")
  const worktree = workspace
  const alias = "oc-command"
  const screenNative = `arroba-${provider}-command-${process.pid}`
  const logs = {
    nativeDir: path.join(root, "native-screen"),
    native: path.join(root, "native-screen", "screenlog.0"),
    proxy: path.join(root, "native.proxy.log"),
  }
  const commandName = "arroba_native_command"
  const commandMarker = `${marker}_OPENCODE_PROVIDER_COMMAND`
  let daemon = null
  let client = null
  try {
    await mkdir(logs.nativeDir, { recursive: true })
    await mkdir(worktree, { recursive: true })
    await writeFile(path.join(worktree, "opencode.json"), JSON.stringify({
      command: {
        [commandName]: {
          template: `Reply with exactly ${commandMarker} and nothing else. Arguments: $ARGUMENTS`,
          description: "Arroba native TUI provider command drill",
        },
      },
    }, null, 2))
    daemon = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_KERNEL_PORT: String(kernelPort),
        ARROBA_MCP_PORT: String(kernelPort + 1000),
        ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
        ARROBA_CODEX_PORT: String(kernelPort + 2001),
        ARROBA_DAEMON_ID: `native-tui-command-${provider}-${process.pid}`,
        ARROBA_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        ARROBA_SESSION_HISTORY_DIR: path.join(root, "history"),
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForDaemon(kernelUrl, workspace, worktree)

    await startScreen(screenNative, logs.nativeDir, "bun", [
      cliPath,
      "opencode",
      "--kernel-url",
      kernelUrl,
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
    ], {
      ...process.env,
      ARROBA_OPENCODE_NATIVE_DEBUG: "1",
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: logs.proxy,
    })
    const sessionId = (await waitForFileMatch(logs.native, /arroba session:\s+([^\s(]+)/)).match[1]
    const proxyUrl = (await waitForFileMatch(logs.native, /proxy:\s+(http:\/\/127\.0\.0\.1:\d+)/)).match[1]
    const providerSessionId = (await waitForFileMatch(logs.native, /opencode sess:\s+([^\s]+)/)).match[1]

    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(
      await client.send(attachToSessionRequest(sessionId, `native-tui-command-${provider}-${process.pid}`)),
      "SessionAttached",
    ).attachment
    const agent = await waitForNamedAgent(client, sessionId, alias)

    await runNativeOpenCodeCommand(proxyUrl, providerSessionId, worktree, commandName, "from-native-provider-command-drill")
    const proxyLog = await readFile(logs.proxy, "utf8")
    if (!proxyLog.includes(`/session/${providerSessionId}/command`)) {
      throw new Error("OpenCode provider command did not pass through native proxy")
    }
    await waitForHistoryOutput(client, sessionId, attachment.id, agent.id, commandMarker)
    return { provider, status: "ok", sessionId, providerSessionId, proxyUrl, commandName, logs }
  } finally {
    if (client) await client.close().catch(() => {})
    await screenQuit(screenNative)
    if (daemon && daemon.exitCode == null) {
      daemon.kill("SIGTERM")
      await Promise.race([new Promise((resolve) => daemon.once("exit", resolve)), sleep(2_000)])
      if (daemon.exitCode == null) daemon.kill("SIGKILL")
    }
    if (process.env.ARROBA_KEEP_NATIVE_TUI_COMMAND_ARTIFACTS === "1" || (options.keepArtifactsOnFailure && process.exitCode)) {
      console.log(JSON.stringify({ provider, artifactsKept: root }))
    } else {
      await rm(root, { recursive: true, force: true }).catch(() => {})
    }
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const results = []
  for (const provider of options.providers) {
    if (provider === "codex") results.push(await runCodex(options))
    else results.push(await runOpenCode(options))
  }
  console.log(JSON.stringify({ status: "ok", results }, null, 2))
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
