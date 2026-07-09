import { spawn, execFile } from "node:child_process"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { mkdir, readFile, readdir, rm } from "node:fs/promises"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"
import { setTimeout as sleep } from "node:timers/promises"
import { finalizeDrillArtifacts, prepareDrillArtifacts } from "./lib/drill-artifacts.mjs"
import { resolveBuiltBinarySync } from "./lib/drill-runtime-helpers.mjs"

import { LocalIpcClient } from "../dist/ipc.js"
import {
  attachToSessionRequest,
  createSessionRequest,
  endSessionRequest,
  getSessionHistoryBlobContentRequest,
  getSessionHistoryOutlineRequest,
  listAgentsRequest,
  pumpTerminalOutputRequest,
  setUserConfigValueRequest,
} from "../dist/ipc-requests.js"

const execFileAsync = promisify(execFile)
const scriptDir = path.dirname(fileURLToPath(import.meta.url))
const cliRoot = path.resolve(scriptDir, "..")
const repoRoot = path.resolve(cliRoot, "..", "..")
const cliPath = path.join(cliRoot, "dist/index.js")
const kernelBinary = resolveBuiltBinarySync(
  path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel"),
  path.join(repoRoot, "apps/kernel/Cargo.toml"),
  "arroba-kernel",
)
const marker = `NTPERM_${process.pid.toString(36)}_${Date.now().toString(36)}`

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
      console.log("Usage: node apps/cli/scripts/live-native-tui-permission-drill.mjs [--providers codex,opencode,claude] [--keep-artifacts-on-failure]")
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
  return 55000 + Math.floor(Math.random() * 5000)
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

async function disableWorkspaceLiveSync(kernelUrl) {
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

async function waitForAutomation(socketPath) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      await automationRequest(socketPath, { action: "ping" }, 5_000)
      return
    } catch (error) {
      if (attempt === 99) throw error
      await sleep(250)
    }
  }
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

async function readHistoryEntriesFromDisk(historyDir, agentId) {
  const files = await readdir(historyDir).catch(() => [])
  const entries = []
  for (const file of files.filter((entry) => entry.endsWith(".jsonl"))) {
    const text = await readFile(path.join(historyDir, file), "utf8").catch(() => "")
    for (const line of text.split("\n")) {
      if (!line.trim()) continue
      try {
        const entry = JSON.parse(line)
        if (!agentId || entry?.agent_id === agentId) entries.push(entry)
      } catch {
        // Ignore partially written history lines while the provider is still streaming.
      }
    }
  }
  return entries
}

function historyOutputText(entries, agentId) {
  return entries
    .filter((entry) =>
      entry?.agent_id === agentId
        && (entry.kind === "provider_output" || entry.kind === "assistant")
    )
    .map((entry) => entry.text ?? "")
    .join("")
}

async function waitForHistoryMarker(client, sessionId, attachmentId, agentId, markerText, historyDir, timeoutMs = 240_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const entries = await loadAgentHistoryEntries(client, sessionId, agentId)
    const text = historyOutputText(entries, agentId)
    if (text.includes(markerText)) return text
    const diskText = historyOutputText(await readHistoryEntriesFromDisk(historyDir, agentId), agentId)
    if (diskText.includes(markerText)) return diskText
    await sleep(1_000)
  }
  throw new Error(`timed out waiting for history marker ${markerText}`)
}

function findCompletedProviderTool(entries, agentId, filePath) {
  let lastMatch = null
  for (const entry of entries) {
    if (!entry || entry.kind !== "provider_tool" || entry.agent_id !== agentId || typeof entry.text !== "string") continue
    let update = null
    try {
      update = JSON.parse(entry.text)
    } catch {
      continue
    }
    const command = String(update.input?.command ?? update.input?.cmd ?? "")
    const output = typeof update.output === "string" ? update.output : JSON.stringify(update.output ?? {})
    if (!entry.text.includes(filePath) && !command.includes(filePath) && !output.includes(filePath)) continue
    lastMatch = update
    if (update.status === "completed") return { completed: update, lastMatch }
  }
  return { completed: null, lastMatch }
}

async function waitForProviderToolCompletion(client, sessionId, attachmentId, agentId, filePath, historyDir, timeoutMs = 240_000) {
  const deadline = Date.now() + timeoutMs
  let lastMatch = null
  while (Date.now() < deadline) {
    await client.send(pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    const projected = findCompletedProviderTool(await loadAgentHistoryEntries(client, sessionId, agentId), agentId, filePath)
    lastMatch = projected.lastMatch ?? lastMatch
    if (projected.completed) return projected.completed
    const fromDisk = findCompletedProviderTool(await readHistoryEntriesFromDisk(historyDir, agentId), agentId, filePath)
    lastMatch = fromDisk.lastMatch ?? lastMatch
    if (fromDisk.completed) return fromDisk.completed
    await sleep(1_000)
  }
  throw new Error(`timed out waiting for completed provider tool touching ${filePath}; last=${JSON.stringify(lastMatch)}`)
}

async function loadAgentHistoryEntries(client, sessionId, agentId) {
  const outline = unwrap(
    await client.send(getSessionHistoryOutlineRequest(sessionId, [agentId], 20)),
    "SessionHistoryOutline",
  )
  const entries = []
  const agent = outline.agents?.find((entry) => entry.agent_id === agentId)
  for (const turn of agent?.turns ?? []) {
    for (const row of turn.entries ?? []) {
      if (row?.entry) entries.push(row.entry)
    }
    for (const blob of turn.blobs ?? []) {
      const content = unwrap(
        await client.send(getSessionHistoryBlobContentRequest(sessionId, agentId, blob.blob_id)),
        "SessionHistoryBlobContent",
      )
      for (const row of content.entries ?? []) {
        if (row?.entry) entries.push(row.entry)
      }
    }
  }
  return entries
}

async function waitForFileContent(filePath, expected, timeoutMs = 240_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const content = await readFile(filePath, "utf8").catch(() => "")
    if (content.trim() === expected) return content
    await sleep(500)
  }
  throw new Error(`timed out waiting for ${filePath} to contain ${expected}`)
}

async function waitForInteraction(socketPath, alias, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const snapshot = await automationRequest(socketPath, { action: "snapshot" })
    last = snapshot
    const agent = snapshot.session?.agents?.find((entry) => entry.alias === alias)
    const interaction = snapshot.interactions?.find((entry) => entry.agentId === agent?.id && entry.kind === "permission")
    if (agent && interaction) return { snapshot, agent, interaction }
    await sleep(250)
  }
  throw new Error(`timed out waiting for permission interaction for ${alias}; last=${JSON.stringify(last)}`)
}

async function waitForInteractionFocused(socketPath, interactionId, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const snapshot = await automationRequest(socketPath, { action: "snapshot" })
    last = snapshot
    const interaction = snapshot.interactions?.find((entry) => entry.id === interactionId)
    if (interaction?.focused) return snapshot
    await sleep(250)
  }
  throw new Error(`timed out waiting for focused interaction ${interactionId}; last=${JSON.stringify(last)}`)
}

async function waitForInteractionCleared(socketPath, interactionId, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const snapshot = await automationRequest(socketPath, { action: "snapshot" })
    last = snapshot
    const interaction = snapshot.interactions?.find((entry) => entry.id === interactionId)
    if (!interaction) return snapshot
    await sleep(250)
  }
  throw new Error(`timed out waiting for interaction ${interactionId} to clear; last=${JSON.stringify(last)}`)
}

async function answerPermissionFromCli(socketPath, alias) {
  const pending = await waitForInteraction(socketPath, alias)
  if (!pending.interaction.focused) {
    await automationRequest(socketPath, {
      action: "workspace_shell_exec",
      command: `agent focus ${pending.agent.id}`,
    })
    await waitForInteractionFocused(socketPath, pending.interaction.id)
  }
  const allowIndex = pending.interaction.choices.findIndex((choice) =>
    choice.id === "allow_once"
      || choice.id === "allow"
      || /allow|yes|proceed/i.test(choice.label ?? "")
  )
  const response = await automationRequest(socketPath, {
    action: "interaction_submit",
    choiceIndex: allowIndex >= 0 ? allowIndex : 0,
  })
  const stillPending = response.interactions?.some((entry) => entry.id === pending.interaction.id)
  if (stillPending) {
    throw new Error(`permission interaction did not clear after submit: ${JSON.stringify(response.interactions)}`)
  }
  await waitForInteractionCleared(socketPath, pending.interaction.id)
  return pending.interaction
}

async function waitForAgentIdle(socketPath, alias, timeoutMs = 120_000) {
  const deadline = Date.now() + timeoutMs
  let last = null
  while (Date.now() < deadline) {
    const snapshot = await automationRequest(socketPath, { action: "snapshot" })
    last = snapshot
    const agent = snapshot.session?.agents?.find((entry) => entry.alias === alias)
    const badge = String(agent?.badge?.label ?? "").toLowerCase()
    if (agent && agent.isProcessing === false && badge === "idle") return snapshot
    await sleep(250)
  }
  throw new Error(`timed out waiting for ${alias} to become idle; last=${JSON.stringify(last)}`)
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
      reject(new Error(`opencode native permission prompt timed out\n${stdout}\n${stderr}`))
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

function permissionPrompt(provider, markerText, filePath, content) {
  const shellCommand = `printf %s ${content} > ${filePath}`
  if (provider === "claude") {
    return `Please create the file ${filePath} with this content: ${content}. You can use Bash if convenient.`
  }
  return provider === "codex"
    ? `Use the shell to run \`${shellCommand}\`. After the command succeeds, reply with exactly ${markerText}.`
    : `Use the shell to run \`${shellCommand}\`. After the command succeeds, reply with exactly ${markerText}.`
}

async function verifyProviderPrerequisites(provider) {
  if (provider !== "claude") return
  let stdout = ""
  let stderr = ""
  try {
    const result = await execFileAsync("claude", ["auth", "status"], {
      maxBuffer: 1024 * 1024,
    })
    stdout = result.stdout
    stderr = result.stderr
  } catch (error) {
    stdout = error?.stdout ?? ""
    stderr = error?.stderr ?? ""
  }
  let status = null
  try {
    status = JSON.parse(stdout)
  } catch {
    throw new Error(`Claude auth status was not JSON: ${(stdout || stderr).slice(0, 4000)}`)
  }
  if (status?.loggedIn !== true) {
    throw new Error(`Claude provider account is not logged in: authMethod=${status?.authMethod ?? "unknown"} apiProvider=${status?.apiProvider ?? "unknown"}`)
  }
}

async function runProvider(provider, options) {
  const root = path.join("/tmp", `arb-native-perm-${provider}-${process.pid}-${Date.now()}`)
  const kernelPort = makePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const workspace = repoRoot
  const worktree = repoRoot
  const alias = `${provider === "codex" ? "cdx" : provider === "opencode" ? "oc" : "cc"}-perm`
  const screenNative = `arroba-${provider}-perm-${process.pid}`
  const screenCli = `arroba-${provider}-perm-cli-${process.pid}`
  const logs = {
    nativeDir: path.join(root, "native-screen"),
    cliDir: path.join(root, "arroba-cli-screen"),
    historyDir: path.join(root, "history"),
    native: path.join(root, "native-screen", "screenlog.0"),
    cli: path.join(root, "arroba-cli-screen", "screenlog.0"),
    proxy: path.join(root, "native.proxy.log"),
  }
  const markers = {
    nativePrompt: `${marker}_${provider}_NATIVE_PERMISSION`,
    arrobaPrompt: `${marker}_${provider}_ARROBA_PERMISSION`,
  }
  const files = {
    nativePrompt: `/tmp/arroba-${provider}-native-permission-${process.pid}.txt`,
    arrobaPrompt: `/tmp/arroba-${provider}-arroba-permission-${process.pid}.txt`,
  }
  const automationSocket = path.join("/tmp", `arb-${provider}-perm-cli-${process.pid}.sock`)
  let daemon = null
  let client = null
  let succeeded = false
  let failure = null
  try {
    await prepareDrillArtifacts(root)
    await mkdir(logs.nativeDir, { recursive: true })
    await mkdir(logs.cliDir, { recursive: true })
    await verifyProviderPrerequisites(provider)
    daemon = spawn(kernelBinary, [], {
      cwd: repoRoot,
      env: {
        ...process.env,
        ARROBA_KERNEL_PORT: String(kernelPort),
        ARROBA_MCP_PORT: String(kernelPort + 1000),
        ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
        ARROBA_CODEX_PORT: String(kernelPort + 2001),
        ARROBA_DAEMON_ID: `native-tui-permission-${provider}-${process.pid}`,
        ARROBA_DAEMON_SOCKET: path.join(root, "daemon.sock"),
        ARROBA_SESSION_HISTORY_DIR: logs.historyDir,
      },
      stdio: ["ignore", "ignore", "inherit"],
    })
    await waitForDaemon(kernelUrl, workspace, worktree)
    await disableWorkspaceLiveSync(kernelUrl)

    const nativeArgs = [
      cliPath,
      provider,
      "--kernel-url",
      kernelUrl,
      "--alias",
      `native-permission-${provider}-${marker}`,
      "--agent-alias",
      alias,
      "--workspace",
      workspace,
      "--worktree",
      worktree,
      "--mode",
      "build",
      "--permissions",
      "required",
    ]
    if (provider === "codex") {
      nativeArgs.push(
        "--model",
        "gpt-5.4-mini",
        "--effort",
        "high",
        "--initial-prompt",
        permissionPrompt(provider, markers.nativePrompt, files.nativePrompt, `native-${provider}`),
      )
    } else if (provider === "claude") {
      nativeArgs.push(
        "--detached-screen",
        "--model",
        "sonnet",
        "--effort",
        "low",
        "--initial-prompt",
        permissionPrompt(provider, markers.nativePrompt, files.nativePrompt, `native-${provider}`),
      )
    }
    await startScreen(screenNative, logs.nativeDir, "bun", nativeArgs, {
      ...process.env,
      ARROBA_CODEX_NATIVE_DEBUG: provider === "codex" ? "1" : undefined,
      ARROBA_CODEX_NATIVE_DEBUG_FILE: provider === "codex" ? logs.proxy : undefined,
      ARROBA_OPENCODE_NATIVE_DEBUG: provider === "opencode" ? "1" : undefined,
      ARROBA_OPENCODE_NATIVE_DEBUG_FILE: provider === "opencode" ? logs.proxy : undefined,
    })

    const sessionId = (await waitForFileMatch(logs.native, /arroba session:\s+([^\s(]+)/)).match[1]
    const proxyUrl = provider === "opencode"
      ? (await waitForFileMatch(logs.native, /proxy:\s+(http:\/\/127\.0\.0\.1:\d+)/)).match[1]
      : null
    const providerSessionId = provider === "opencode"
      ? (await waitForFileMatch(logs.native, /opencode sess:\s+([^\s]+)/)).match[1]
      : null
    if (provider === "claude") {
      await waitForFileMatch(logs.native, /screen:\s+(arroba-claude-[^\s]+)/)
    }

    client = new LocalIpcClient(kernelUrl)
    const attachment = unwrap(
      await client.send(attachToSessionRequest(sessionId, `native-tui-permission-${provider}-${process.pid}`)),
      "SessionAttached",
    ).attachment
    const agent = await waitForNamedAgent(client, sessionId, alias)

    await startScreen(screenCli, logs.cliDir, "bun", [
      cliPath,
      "--kernel-url",
      kernelUrl,
      "--session",
      sessionId,
      "--client-id",
      `native-tui-permission-observer-${provider}-${process.pid}`,
      "--automation-socket",
      automationSocket,
      "--provider",
      provider,
      "--model",
      provider === "codex" ? "gpt-5.4-mini" : provider === "claude" ? "sonnet" : "default",
      "--effort",
      provider === "claude" ? "low" : "high",
    ], process.env)
    await waitForAutomation(automationSocket)

    if (provider === "opencode") {
      const nativePrompt = permissionPrompt(provider, markers.nativePrompt, files.nativePrompt, `native-${provider}`)
      const nativeRun = runNativeOpenCodePrompt(proxyUrl, providerSessionId, worktree, nativePrompt)
      const nativeInteraction = await answerPermissionFromCli(automationSocket, alias)
      await nativeRun
      await waitForHistoryMarker(client, sessionId, attachment.id, agent.id, markers.nativePrompt, logs.historyDir)
      await waitForProviderToolCompletion(client, sessionId, attachment.id, agent.id, files.nativePrompt, logs.historyDir)
      await waitForFileContent(files.nativePrompt, `native-${provider}`, 5_000).catch(() => {})
      await waitForAgentIdle(automationSocket, alias)
      console.log(JSON.stringify({ provider, direction: "native_tui_to_arroba", interaction: nativeInteraction.title ?? nativeInteraction.message }))
    } else {
      const nativeInteraction = await answerPermissionFromCli(automationSocket, alias)
      if (provider !== "claude") await waitForHistoryMarker(client, sessionId, attachment.id, agent.id, markers.nativePrompt, logs.historyDir)
      if (provider !== "claude") await waitForProviderToolCompletion(client, sessionId, attachment.id, agent.id, files.nativePrompt, logs.historyDir)
      if (provider === "claude") await waitForFileContent(files.nativePrompt, `native-${provider}`)
      else await waitForFileContent(files.nativePrompt, `native-${provider}`, 5_000).catch(() => {})
      await waitForAgentIdle(automationSocket, alias)
      console.log(JSON.stringify({ provider, direction: "native_tui_to_arroba", interaction: nativeInteraction.title ?? nativeInteraction.message }))
    }

    await automationRequest(automationSocket, {
      action: "workspace_shell_exec",
      command: `prompt ${alias} ${permissionPrompt(provider, markers.arrobaPrompt, files.arrobaPrompt, `arroba-${provider}`)}`,
    })
    const arrobaInteraction = await answerPermissionFromCli(automationSocket, alias)
    if (provider !== "claude") await waitForHistoryMarker(client, sessionId, attachment.id, agent.id, markers.arrobaPrompt, logs.historyDir)
    if (provider !== "claude") await waitForProviderToolCompletion(client, sessionId, attachment.id, agent.id, files.arrobaPrompt, logs.historyDir)
    if (provider === "claude") await waitForFileContent(files.arrobaPrompt, `arroba-${provider}`)
    else await waitForFileContent(files.arrobaPrompt, `arroba-${provider}`, 5_000).catch(() => {})
    console.log(JSON.stringify({ provider, direction: "arroba_cli_to_provider", interaction: arrobaInteraction.title ?? arrobaInteraction.message }))

    succeeded = true
    return { provider, status: "ok", sessionId, alias, markers, logs }
  } catch (error) {
    failure = error
    throw error
  } finally {
    if (client) await client.close().catch(() => {})
    await screenQuit(screenNative)
    await screenQuit(screenCli)
    if (daemon && daemon.exitCode == null) {
      daemon.kill("SIGTERM")
      await Promise.race([new Promise((resolve) => daemon.once("exit", resolve)), sleep(2_000)])
      if (daemon.exitCode == null) daemon.kill("SIGKILL")
    }
    const preserveOnFailure = options.keepArtifactsOnFailure || process.env.ARROBA_KEEP_NATIVE_TUI_PERMISSION_ARTIFACTS === "1"
    const finalized = await finalizeDrillArtifacts({
      rootDir: root,
      passed: succeeded,
      preserveOnFailure,
      failure,
      metadata: {
        drill: "native-tui-permission",
        provider,
        kernelUrl,
        logs,
      },
      log: (name, details) => console.log(`[native-tui-permission-drill] ${name}`, JSON.stringify(details)),
    })
    if (!finalized.preserved) {
      await rm(automationSocket, { force: true }).catch(() => {})
    }
    await rm(files.nativePrompt, { force: true }).catch(() => {})
    await rm(files.arrobaPrompt, { force: true }).catch(() => {})
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const results = []
  for (const provider of options.providers) {
    try {
      results.push(await runProvider(provider, options))
    } catch (error) {
      results.push({
        provider,
        status: "failed",
        error: error?.message ?? String(error),
      })
    }
  }
  const failures = results.filter((entry) => entry.status !== "ok")
  console.log(JSON.stringify({ status: failures.length === 0 ? "ok" : "failed", results }, null, 2))
  if (failures.length > 0) {
    process.exitCode = 1
  }
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
