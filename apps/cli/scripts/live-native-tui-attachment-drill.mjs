import { spawn, execFile } from "node:child_process"
import net from "node:net"
import path from "node:path"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { setTimeout as sleep } from "node:timers/promises"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import { historyOutlineText } from "./lib/drill-history-outline.mjs"

import WebSocket from "ws"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  endSessionRequest,
  getSessionHistoryOutlineRequest,
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
const validationPng = Buffer.from("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", "base64")

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
      console.log("Usage: node apps/cli/scripts/live-native-tui-attachment-drill.mjs [--providers codex,opencode,claude] [--keep-artifacts-on-failure]")
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  for (const provider of options.providers) {
    if (provider !== "codex" && provider !== "opencode" && provider !== "claude") throw new Error(`unsupported provider: ${provider}`)
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

async function screenStuff(name, text) {
  await screen(name, ["-p", "0", "-X", "stuff", text])
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
    preserveOnFailure: options.keepArtifactsOnFailure || process.env.ARROBA_KEEP_NATIVE_TUI_ATTACHMENT_ARTIFACTS === "1",
    failure,
    metadata: {
      drill: "native-tui-attachment",
      provider,
      kernelUrl,
      logs,
    },
    log: (name, details) => console.log(`[native-tui-attachment-drill] ${name}`, JSON.stringify(details)),
  })
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
  const alias = `${provider === "codex" ? "cdx" : provider === "opencode" ? "oc" : "cc"}-attachment`
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
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(root)
    await mkdir(logs.nativeDir, { recursive: true })
    await writeFile(imagePath, validationPng)
    await writeFile(textPath, `${nativeMarker}\n`)
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
      provider === "claude" ? "required" : "yolo",
      ...(provider === "codex" ? ["--model", "gpt-5.4-mini", "--effort", "high"] : []),
      ...(provider === "claude" ? ["--detached-screen"] : []),
    ], {
      ...process.env,
      ARROBA_CODEX_NATIVE_DEBUG: provider === "codex" ? "1" : undefined,
      ARROBA_CODEX_NATIVE_DEBUG_FILE: provider === "codex" ? logs.proxy : undefined,
      ARROBA_OPENCODE_NATIVE_DEBUG: provider === "opencode" ? "1" : undefined,
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: provider === "opencode" ? logs.proxy : undefined,
      ARROBA_CLAUDE_NATIVE_DEBUG: provider === "claude" ? "1" : undefined,
      ARROBA_CLAUDE_NATIVE_DEBUG_FILE: provider === "claude" ? logs.proxy : undefined,
    })
    const sessionId = (await waitForFileMatch(logs.native, /arroba session:\s+([^\s(]+)/)).match[1]
    const proxyUrl = provider === "codex"
      ? (await waitForFileMatch(logs.native, /proxy:\s+(ws:\/\/127\.0\.0\.1:\d+)/)).match[1]
      : provider === "opencode"
        ? (await waitForFileMatch(logs.native, /proxy:\s+(http:\/\/127\.0\.0\.1:\d+)/)).match[1]
        : null
    const providerSessionId = provider === "opencode"
      ? (await waitForFileMatch(logs.native, /opencode sess:\s+([^\s]+)/)).match[1]
      : null
    const threadId = provider === "codex"
      ? (await waitForFileMatch(logs.proxy, /thread_observed:\s+\{"threadId":"([^"]+)"/)).match[1]
      : null
    const claudeScreen = provider === "claude"
      ? (await waitForFileMatch(logs.native, /screen:\s+(arroba-claude-[^\s]+)/)).match[1]
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
    } else if (provider === "opencode") {
      await runNativeOpenCodePromptWithFile(
        proxyUrl,
        providerSessionId,
        worktree,
        textPath,
        `Reply with exactly ${nativeMarker} and nothing else.`,
      )
    } else if (provider === "claude") {
      await screenStuff(claudeScreen, `@${imagePath} Reply with exactly ${nativeMarker} and nothing else.`)
      await sleep(250)
      await screenStuff(claudeScreen, "\r")
    }
    await waitForLogOccurrences(logs.proxy, provider === "codex" ? "attachmentCount\":1" : "native_prompt_attachments_observed", 1)
    await waitForHistoryOutput(client, sessionId, attachment.id, agent.id, nativeMarker)
    if (provider !== "claude") {
      await waitForLogOccurrences(logs.proxy, "attachments_forwarded", 1)
    }

    const arrobaAttachmentPath = path.join(root, `${provider}-arroba-note.txt`)
    await writeFile(arrobaAttachmentPath, `${arrobaMarker}\n`)
    const arrobaPrompt = provider === "claude"
      ? `Reply with exactly ${arrobaMarker} and nothing else.`
      : `Reply with exactly ${arrobaMarker} and nothing else.`
    await client.send(submitPromptRequest(sessionId, attachment.id, agent.id, arrobaPrompt, [
      provider === "codex" || provider === "claude"
        ? { url: imagePath, mime: "image/png", filename: path.basename(imagePath) }
        : { url: `file://${arrobaAttachmentPath}`, mime: "text/plain", filename: path.basename(arrobaAttachmentPath) },
    ]))
    await waitForLogOccurrences(logs.proxy, "attachments_forwarded", provider === "claude" ? 1 : 2)
    await waitForHistoryOutput(client, sessionId, attachment.id, agent.id, arrobaMarker)

    succeeded = true
    return { provider, status: "ok", sessionId, alias, nativeMarker, arrobaMarker, logs }
  } catch (error) {
    failure = error
    throw error
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
    results.push(await runProvider(provider, options))
  }
  console.log(JSON.stringify({ status: "ok", results }, null, 2))
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
