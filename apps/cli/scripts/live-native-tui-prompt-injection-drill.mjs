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
const hiddenMarker = "ARROBA_NATIVE_TUI_HIDDEN_INSTRUCTIONS"
const marker = `NTINJ_${process.pid.toString(36)}_${Date.now().toString(36)}`

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
      console.log("Usage: node apps/cli/scripts/live-native-tui-prompt-injection-drill.mjs [--providers codex,opencode,claude] [--keep-artifacts-on-failure]")
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

async function makePort() {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    const port = 54500 + Math.floor(Math.random() * 3000)
    if (
      await portAvailable(port)
      && await portAvailable(port + 1000)
      && await portAvailable(port + 2000)
      && await portAvailable(port + 2001)
    ) {
      return port
    }
  }
  throw new Error("could not find free native prompt injection drill ports")
}

async function portAvailable(port) {
  return await new Promise((resolve) => {
    const server = net.createServer()
    server.once("error", () => resolve(false))
    server.once("listening", () => {
      server.close(() => resolve(true))
    })
    server.listen(port, "127.0.0.1")
  })
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

function nativeTempRootForClaudeScreen(screenName) {
  const suffix = screenName.replace(/^arroba-claude-/, "")
  return path.join(os.tmpdir(), `arroba-claude-native-${suffix}`)
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
  let lastOutput = ""
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const outline = unwrap(await client.send(getSessionHistoryOutlineRequest(sessionId, [agentId], 6)), "SessionHistoryOutline")
    const agent = outline.agents?.find((entry) => entry.agent_id === agentId) ?? null
    const chunks = []
    for (const turn of agent?.turns ?? []) {
      for (const row of [...(turn.entries ?? []), ...(turn.summary ? [turn.summary] : [])]) {
        const entry = row?.entry
        if (entry && entry.kind !== "user_prompt") {
          chunks.push(entry.text ?? "")
        }
      }
      for (const blob of turn.blobs ?? []) {
        const blobContent = unwrap(await client.send(getSessionHistoryBlobContentRequest(sessionId, agentId, blob.blob_id)), "SessionHistoryBlobContent")
        for (const row of blobContent.entries ?? []) {
          const entry = row?.entry
          if (entry && entry.kind !== "user_prompt") {
            chunks.push(entry.text ?? "")
          }
        }
      }
    }
    const output = chunks.join("")
    lastOutput = output
    if (output.includes(expected)) return output
    await sleep(1_000)
  }
  throw new Error(`timed out waiting for history output ${expected}; last=${lastOutput.slice(-4000)}`)
}

async function waitForProxyHiddenForward(logFile, timeoutMs = 90_000) {
  const deadline = Date.now() + timeoutMs
  let text = ""
  while (Date.now() < deadline) {
    text = await readFile(logFile, "utf8").catch(() => "")
    if (
      text.includes("hidden_instructions_forwarded")
      || text.includes("collaboration_mode_forwarded")
      || text.includes("system_context_forwarded")
    ) return text
    await sleep(250)
  }
  throw new Error(`timed out waiting for hidden instruction forwarding in ${logFile}\n${text.slice(-4000)}`)
}

async function waitForClaudeHookPrompt(eventsFile, visiblePrompt, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs
  let raw = ""
  while (Date.now() < deadline) {
    raw = await readFile(eventsFile, "utf8").catch(() => "")
    const prompts = raw
      .split("\n")
      .filter((line) => line.trim())
      .map((line) => {
        try {
          return JSON.parse(line)
        } catch {
          return null
        }
      })
      .filter((event) => event?.hook_event_name === "UserPromptSubmit")
      .map((event) => event.prompt ?? "")
    const prompt = prompts.find((entry) => entry.includes(visiblePrompt))
    if (prompt) {
      if (prompt.includes(hiddenMarker) || prompt.includes("list_extensions") || prompt.includes("native approval request")) {
        throw new Error(`Claude hook prompt showed hidden Arroba instructions in ${eventsFile}`)
      }
      return prompt
    }
    await sleep(500)
  }
  throw new Error(`timed out waiting for Claude hook prompt ${visiblePrompt} in ${eventsFile}\n${raw}`)
}

async function assertNativeTuiDidNotShowHiddenInstructions(logFile) {
  const text = await readFile(logFile, "utf8").catch(() => "")
  if (text.includes(hiddenMarker) || text.includes("list_extensions") || text.includes("native approval request")) {
    throw new Error(`native TUI log showed hidden Arroba instructions in ${logFile}`)
  }
}

async function runNativeOpenCodePrompt(proxyUrl, providerSessionId, worktree, prompt) {
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
    ], {
      cwd: worktree,
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    })
    let stdout = ""
    let stderr = ""
    const timer = setTimeout(() => {
      child.kill("SIGTERM")
      reject(new Error(`opencode native prompt timed out\n${stdout}\n${stderr}`))
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
      else reject(new Error(`opencode run --attach exited with ${signal ?? code}\n${stdout}\n${stderr}`))
    })
  })
}

async function runProvider(provider, options) {
  const root = path.join("/tmp", `arb-native-injection-${provider}-${process.pid}-${Date.now()}`)
  const kernelPort = await makePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const workspace = repoRoot
  const worktree = repoRoot
  const alias = `${provider === "codex" ? "cdx" : provider === "opencode" ? "oc" : "cc"}-injection`
  const screenNative = `arroba-${provider}-injection-${process.pid}`
  const logs = {
    nativeDir: path.join(root, "native-screen"),
    native: path.join(root, "native-screen", "screenlog.0"),
    proxy: path.join(root, "native.proxy.log"),
  }
  const outputMarker = `${marker}_${provider}_VISIBLE_PROMPT`
  let daemon = null
  let client = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(root)
    await mkdir(logs.nativeDir, { recursive: true })
    daemon = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_KERNEL_PORT: String(kernelPort),
        ARROBA_MCP_PORT: String(kernelPort + 1000),
        ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
        ARROBA_CODEX_PORT: String(kernelPort + 2001),
        ARROBA_DAEMON_ID: `native-tui-injection-${provider}-${process.pid}`,
        ARROBA_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        ARROBA_SESSION_HISTORY_DIR: path.join(root, "history"),
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForDaemon(kernelUrl, workspace, worktree)

    const nativeArgs = [
      cliPath,
      provider,
      "--kernel-url",
      kernelUrl,
      "--alias",
      `native-injection-${provider}-${marker}`,
      "--agent-alias",
      alias,
      "--workspace",
      workspace,
      "--worktree",
      worktree,
      "--permissions",
      provider === "claude" ? "required" : "yolo",
    ]
    if (provider === "codex") {
      nativeArgs.push(
        "--model",
        "gpt-5.4-mini",
        "--effort",
        "high",
        "--initial-prompt",
        `Reply with exactly ${outputMarker} and nothing else.`,
      )
    } else if (provider === "claude") {
      nativeArgs.push(
        "--detached-screen",
        "--model",
        "sonnet",
        "--effort",
        "low",
      )
    }
    await startScreen(screenNative, logs.nativeDir, "bun", nativeArgs, {
      ...process.env,
      ARROBA_CODEX_NATIVE_DEBUG: provider === "codex" ? "1" : undefined,
      ARROBA_CODEX_NATIVE_DEBUG_FILE: provider === "codex" ? logs.proxy : undefined,
      ARROBA_OPENCODE_NATIVE_DEBUG: provider === "opencode" ? "1" : undefined,
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: provider === "opencode" ? logs.proxy : undefined,
      ARROBA_CLAUDE_NATIVE_DEBUG: provider === "claude" ? "1" : undefined,
      ARROBA_CLAUDE_NATIVE_DEBUG_FILE: provider === "claude" ? logs.proxy : undefined,
    })
    const sessionId = (await waitForFileMatch(logs.native, /arroba session:\s+([^\s(]+)/)).match[1]
    const proxyUrl = provider === "opencode"
      ? (await waitForFileMatch(logs.native, /proxy:\s+(http:\/\/127\.0\.0\.1:\d+)/)).match[1]
      : null
    const providerSessionId = provider === "opencode"
      ? (await waitForFileMatch(logs.native, /opencode sess:\s+([^\s]+)/)).match[1]
      : null
    const claudeScreen = provider === "claude"
      ? (await waitForFileMatch(logs.native, /screen:\s+(arroba-claude-[^\s]+)/)).match[1]
      : null
    if (claudeScreen) {
      logs.claudeScreen = path.join(nativeTempRootForClaudeScreen(claudeScreen), "screen", "screenlog.0")
      logs.claudeEvents = path.join(nativeTempRootForClaudeScreen(claudeScreen), "events.jsonl")
    }

    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(
      await client.send(attachToSessionRequest(sessionId, `native-tui-injection-${provider}-${process.pid}`)),
      "SessionAttached",
    ).attachment
    const agent = await waitForNamedAgent(client, sessionId, alias)

    if (provider === "opencode") {
      await runNativeOpenCodePrompt(proxyUrl, providerSessionId, worktree, `Reply with exactly ${outputMarker} and nothing else.`)
    } else if (provider === "claude") {
      await client.send(submitPromptRequest(
        sessionId,
        attachment.id,
        agent.id,
        `Reply with exactly ${outputMarker} and nothing else.`,
        [],
      ))
    }
    await waitForHistoryOutput(client, sessionId, attachment.id, agent.id, outputMarker)
    if (provider !== "claude") {
      await waitForProxyHiddenForward(logs.proxy)
    }
    if (provider === "claude") {
      await waitForClaudeHookPrompt(logs.claudeEvents, outputMarker)
    }
    await sleep(1_000)
    await assertNativeTuiDidNotShowHiddenInstructions(logs.native)
    if (logs.claudeScreen) {
      await assertNativeTuiDidNotShowHiddenInstructions(logs.claudeScreen)
    }
    const result = { provider, status: "ok", sessionId, alias, outputMarker, logs }
    succeeded = true
    return result
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
    const preserveOnFailure = options.keepArtifactsOnFailure || process.env.ARROBA_KEEP_NATIVE_TUI_INJECTION_ARTIFACTS === "1"
    const finalized = await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded,
      preserveOnFailure,
      failure,
      metadata: {
        drill: "native-tui-prompt-injection",
        provider,
        kernelUrl,
        logs,
      },
      log: (name, details) => console.log(`[native-tui-prompt-injection-drill] ${name}`, JSON.stringify(details)),
    })
    if (!finalized.preserved && logs.claudeScreen) {
      await rm(path.dirname(path.dirname(logs.claudeScreen)), { recursive: true, force: true }).catch(() => {})
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
