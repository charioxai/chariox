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
  submitPromptRequest,
} from "../dist/ipc-requests.js"

const execFileAsync = promisify(execFile)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cliPath = path.join(cliRoot, "dist/index.js")
const kernelBinary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
const marker = `NTATT_${process.pid.toString(36)}_${Date.now().toString(36)}`
const tinyPng = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/p9sAAAAASUVORK5CYII=", "base64")

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
      console.log("Usage: node apps/cli/scripts/live-native-tui-attachment-drill.mjs [--providers codex,opencode] [--keep-artifacts-on-failure]")
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
  return 53500 + Math.floor(Math.random() * 4000)
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

async function waitForLogOccurrences(logFile, needle, count, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs
  let text = ""
  while (Date.now() < deadline) {
    text = await readFile(logFile, "utf8").catch(() => "")
    const occurrences = text.split(needle).length - 1
    if (occurrences >= count) return text
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${count} occurrences of ${needle} in ${logFile}\n${text.slice(-4000)}`)
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
      for (const message of messages) ws.send(JSON.stringify(message))
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

async function runNativeOpenCodePromptWithFile(proxyUrl, providerSessionId, worktree, filePath, prompt) {
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
      prompt,
      `--file=${filePath}`,
    ], {
      cwd: worktree,
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    const timer = setTimeout(() => {
      child.kill("SIGTERM")
      reject(new Error(`opencode attachment prompt timed out\n${stdout}\n${stderr}`))
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
      else reject(new Error(`opencode run --file exited with ${signal ?? code}\n${stdout}\n${stderr}`))
    })
  })
}

async function runProvider(provider, options) {
  const root = path.join("/tmp", `arb-native-attachment-${provider}-${process.pid}-${Date.now()}`)
  const kernelPort = makePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const workspace = repoRoot
  const worktree = repoRoot
  const alias = `${provider === "codex" ? "cdx" : "oc"}-attachment`
  const screenNative = `arroba-${provider}-attachment-${process.pid}`
  const logs = {
    nativeDir: path.join(root, "native-screen"),
    native: path.join(root, "native-screen", "screenlog.0"),
    proxy: path.join(root, "native.proxy.log"),
  }
  const nativeMarker = `${marker}_${provider}_NATIVE_ATTACHMENT`
  const arrobaMarker = `${marker}_${provider}_ARROBA_ATTACHMENT`
  const imagePath = path.join(root, `${provider}-image.png`)
  const textPath = path.join(root, `${provider}-note.txt`)
  let daemon = null
  let client = null
  try {
    await mkdir(logs.nativeDir, { recursive: true })
    await writeFile(imagePath, tinyPng)
    await writeFile(textPath, `attachment drill ${provider}\n`)
    daemon = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_KERNEL_PORT: String(kernelPort),
        ARROBA_MCP_PORT: String(kernelPort + 1000),
        ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
        ARROBA_CODEX_PORT: String(kernelPort + 2001),
        ARROBA_DAEMON_ID: `native-tui-attachment-${provider}-${process.pid}`,
        ARROBA_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        ARROBA_SESSION_HISTORY_DIR: path.join(root, "history"),
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForDaemon(kernelUrl, workspace, worktree)

    await startScreen(screenNative, logs.nativeDir, "bun", [
      cliPath,
      provider,
      "--kernel-url",
      kernelUrl,
      "--alias",
      `native-attachment-${provider}-${marker}`,
      "--agent-alias",
      alias,
      "--workspace",
      workspace,
      "--worktree",
      worktree,
      "--permissions",
      "yolo",
      ...(provider === "codex" ? ["--model", "gpt-5.4-mini", "--effort", "high"] : []),
    ], {
      ...process.env,
      ARROBA_CODEX_NATIVE_DEBUG: provider === "codex" ? "1" : undefined,
      ARROBA_CODEX_NATIVE_DEBUG_FILE: provider === "codex" ? logs.proxy : undefined,
      ARROBA_OPENCODE_NATIVE_DEBUG: provider === "opencode" ? "1" : undefined,
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: provider === "opencode" ? logs.proxy : undefined,
    })
    const sessionId = (await waitForFileMatch(logs.native, /arroba session:\s+([^\s(]+)/)).match[1]
    const proxyUrl = provider === "codex"
      ? (await waitForFileMatch(logs.native, /proxy:\s+(ws:\/\/127\.0\.0\.1:\d+)/)).match[1]
      : (await waitForFileMatch(logs.native, /proxy:\s+(http:\/\/127\.0\.0\.1:\d+)/)).match[1]
    const providerSessionId = provider === "opencode"
      ? (await waitForFileMatch(logs.native, /opencode sess:\s+([^\s]+)/)).match[1]
      : null
    const threadId = provider === "codex"
      ? (await waitForFileMatch(logs.proxy, /thread_observed:\s+\{"threadId":"([^"]+)"/)).match[1]
      : null

    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(
      await client.send(attachToSessionRequest(sessionId, `native-tui-attachment-${provider}-${process.pid}`)),
      "SessionAttached",
    ).attachment
    const agent = await waitForNamedAgent(client, sessionId, alias)

    if (provider === "codex") {
      const responses = await codexRpc(proxyUrl, [
        { id: 1, method: "initialize", params: { clientInfo: { name: "native-attachment-drill", version: "0.0.0" } } },
        {
          id: 2,
          method: "turn/start",
          params: {
            threadId,
            input: [
              { type: "text", text: `Reply with exactly ${nativeMarker} and nothing else.`, text_elements: [] },
              { type: "localImage", path: imagePath },
            ],
          },
        },
      ])
      const turnResponse = responses.find((response) => response.id === 2)
      if (!turnResponse || turnResponse.error) throw new Error(`codex native attachment turn failed: ${JSON.stringify(turnResponse)}`)
    } else {
      await runNativeOpenCodePromptWithFile(
        proxyUrl,
        providerSessionId,
        worktree,
        textPath,
        `Reply with exactly ${nativeMarker} and nothing else.`,
      )
    }
    await waitForLogOccurrences(logs.proxy, provider === "codex" ? "attachmentCount\":1" : "native_prompt_attachments_observed", 1)
    await waitForHistoryOutput(client, sessionId, attachment.id, agent.id, nativeMarker)
    await waitForLogOccurrences(logs.proxy, "attachments_forwarded", 1)

    await client.send(submitPromptRequest(sessionId, attachment.id, agent.id, `Reply with exactly ${arrobaMarker} and nothing else.`, [
      provider === "codex"
        ? { url: imagePath, mime: "image/png", filename: path.basename(imagePath) }
        : { url: `file://${textPath}`, mime: "text/plain", filename: path.basename(textPath) },
    ]))
    await waitForLogOccurrences(logs.proxy, "attachments_forwarded", 2)

    return { provider, status: "ok", sessionId, alias, nativeMarker, arrobaMarker, logs }
  } finally {
    if (client) await client.close().catch(() => {})
    await screenQuit(screenNative)
    if (daemon && daemon.exitCode == null) {
      daemon.kill("SIGTERM")
      await Promise.race([new Promise((resolve) => daemon.once("exit", resolve)), sleep(2_000)])
      if (daemon.exitCode == null) daemon.kill("SIGKILL")
    }
    if (process.env.ARROBA_KEEP_NATIVE_TUI_ATTACHMENT_ARTIFACTS === "1" || (options.keepArtifactsOnFailure && process.exitCode)) {
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
    results.push(await runProvider(provider, options))
  }
  console.log(JSON.stringify({ status: "ok", results }, null, 2))
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
