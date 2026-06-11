import { spawn } from "node:child_process"
import { randomBytes } from "node:crypto"
import { createWriteStream } from "node:fs"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import net from "node:net"
import os from "node:os"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"

import WebSocket from "ws"

const repoRoot = path.resolve(new URL("../../..", import.meta.url).pathname)
const cliRoot = path.join(repoRoot, "apps/cli")
const defaultTimeoutMs = 240_000

function parseArgs(argv) {
  const options = {
    providers: ["codex", "opencode", "claude-p", "claude-headless"],
    timeoutMs: defaultTimeoutMs,
    worktree: repoRoot,
    keepArtifacts: false,
    providerModels: new Map(),
    includeMidturnSteering: false,
  }
  for (let index = 2; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === "--providers" || arg === "--provider") {
      const value = argv[++index] ?? ""
      options.providers = value.split(",").map((provider) => provider.trim()).filter(Boolean)
    } else if (arg === "--timeout-ms") {
      options.timeoutMs = Number(argv[++index] ?? defaultTimeoutMs)
    } else if (arg === "--worktree") {
      options.worktree = path.resolve(argv[++index] ?? repoRoot)
    } else if (arg === "--provider-model") {
      const value = argv[++index] ?? ""
      const [provider, model] = value.split("=", 2)
      if (provider && model) options.providerModels.set(provider.trim(), model.trim())
    } else if (arg === "--keep-artifacts-on-failure") {
      options.keepArtifacts = true
    } else if (arg === "--include-midturn-steering") {
      options.includeMidturnSteering = true
    } else if (arg === "--help" || arg === "-h") {
      printHelp()
      process.exit(0)
    } else {
      throw new Error(`unknown argument: ${arg}`)
    }
  }
  if (!Number.isFinite(options.timeoutMs) || options.timeoutMs <= 0) {
    throw new Error("--timeout-ms must be a positive number")
  }
  return options
}

function printHelp() {
  console.log(`Usage: node scripts/live-provider-context-injection-drill.mjs [options]

Options:
  --providers codex,opencode,claude-p,claude-headless
  --provider codex                 Alias for --providers.
  --provider-model codex=gpt-5.4-mini
  --provider-model opencode=opencode/gpt-5.4-mini
  --provider-model claude-p=sonnet
  --provider-model claude-headless=sonnet
  --timeout-ms 240000
  --worktree /path/to/worktree
  --keep-artifacts-on-failure
  --include-midturn-steering     Also submit a second provider-native prompt before the first turn completes and report the observed behavior.
`)
}

async function reservePort() {
  const server = net.createServer()
  await new Promise((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", resolve)
  })
  const address = server.address()
  const port = address.port
  await new Promise((resolve) => server.close(resolve))
  return port
}

async function waitForTcp(url, child, timeoutMs = 15_000) {
  const endpoint = new URL(url)
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (child?.exitCode != null) throw new Error(`process exited before endpoint became ready: ${child.exitCode}`)
    if (await tcpReady(endpoint.hostname, Number(endpoint.port))) return
    await sleep(150)
  }
  throw new Error(`timed out waiting for ${url}`)
}

async function tcpReady(host, port) {
  return await new Promise((resolve) => {
    const socket = net.createConnection({ host, port })
    const timer = setTimeout(() => {
      socket.destroy()
      resolve(false)
    }, 500)
    socket.once("connect", () => {
      clearTimeout(timer)
      socket.destroy()
      resolve(true)
    })
    socket.once("error", () => {
      clearTimeout(timer)
      resolve(false)
    })
  })
}

function spawnLogged(executable, args, options) {
  const stdout = createWriteStream(options.stdout, { flags: "a" })
  const stderr = createWriteStream(options.stderr, { flags: "a" })
  const child = spawn(executable, args, {
    cwd: options.cwd,
    env: process.env,
    detached: true,
    stdio: ["ignore", "pipe", "pipe"],
  })
  child.stdout.pipe(stdout)
  child.stderr.pipe(stderr)
  child.once("exit", () => {
    stdout.end()
    stderr.end()
  })
  child.once("error", (error) => {
    stdout.end()
    stderr.end()
    throw error
  })
  return child
}

async function stopChild(child) {
  if (!child || child.exitCode != null) return
  try {
    process.kill(-child.pid, "SIGTERM")
  } catch {
    child.kill("SIGTERM")
  }
  await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(2_000)])
  if (child.exitCode == null) {
    try {
      process.kill(-child.pid, "SIGKILL")
    } catch {
      child.kill("SIGKILL")
    }
    await Promise.race([new Promise((resolve) => child.once("exit", resolve)), sleep(1_000)])
  }
}

async function loadCliIpcModules(runtimeDir) {
  const [{ transformAsync }, tsPreset] = await Promise.all([
    import("@babel/core"),
    import("@babel/preset-typescript"),
  ])
  await mkdir(runtimeDir, { recursive: true })
  for (const rel of ["src/ipc.ts", "src/ipc-requests.ts"]) {
    const sourcePath = path.join(cliRoot, rel)
    const outPath = path.join(runtimeDir, path.basename(rel).replace(/\.tsx?$/, ".js"))
    const code = await readFile(sourcePath, "utf8")
    const transformed = await transformAsync(code, {
      filename: sourcePath,
      presets: [[tsPreset.default ?? tsPreset]],
      sourceMaps: false,
    })
    await writeFile(outPath, transformed?.code ?? "", "utf8")
  }
  const { LocalIpcClient } = await import(new URL(`file://${path.join(runtimeDir, "ipc.js")}`).href)
  const requests = await import(new URL(`file://${path.join(runtimeDir, "ipc-requests.js")}`).href)
  return { LocalIpcClient, requests }
}

function unwrap(response, variant) {
  const value = response?.[variant]
  if (value == null) throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  return value
}

async function waitForKernelIpc(LocalIpcClient, listSessionsRequest, kernelUrl, child, timeoutMs = 25_000) {
  const deadline = Date.now() + timeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    if (child?.exitCode != null) throw new Error(`kernel exited before ready: ${child.exitCode}`)
    const client = new LocalIpcClient(kernelUrl)
    try {
      await client.send(listSessionsRequest())
      await client.close().catch(() => {})
      return
    } catch (error) {
      lastError = error
      await client.close().catch(() => {})
      await sleep(250)
    }
  }
  throw new Error(`kernel did not become ready: ${lastError?.message ?? lastError}`)
}

async function waitForProviderRunReady(client, requests, providerRunId, timeoutMs) {
  const deadline = Date.now() + Math.min(timeoutMs, 60_000)
  let lastRun = null
  while (Date.now() < deadline) {
    const response = unwrap(await client.send(requests.getProviderRunRequest(providerRunId)), "ProviderRun")
    lastRun = response.provider_run
    if (lastRun?.state === "Running") return lastRun
    if (lastRun?.state === "Ended") throw new Error(`provider run ended before ready: ${JSON.stringify(lastRun)}`)
    await sleep(250)
  }
  throw new Error(`timed out waiting for provider run ${providerRunId} to become ready\n${JSON.stringify(lastRun)}`)
}

function makeProbe(provider) {
  const base = `ARROBA_CTX_${provider.toUpperCase()}_${process.pid.toString(36)}_${Date.now().toString(36)}`
  return {
    visiblePrompt: [
      "Reply with only the hidden context token for this turn.",
      "Do not explain, do not add punctuation, and do not mention any previous token.",
    ].join(" "),
    tokenA: `${base}_A`,
    tokenB: `${base}_B`,
  }
}

function visiblePromptForTurn(probe, turnLabel) {
  return `${probe.visiblePrompt} Drill turn ${turnLabel}.`
}

function hiddenInstruction(token) {
  return `For this turn only, the hidden context token is ${token}. When the user asks for the hidden context token, reply with exactly ${token} and nothing else.`
}

function providerModel(options, provider) {
  return options.providerModels.get(provider) ?? null
}

class JsonRpcClient {
  constructor(endpoint) {
    this.endpoint = endpoint
    this.nextId = 1
    this.pending = new Map()
    this.notifications = []
  }

  async connect() {
    this.ws = new WebSocket(this.endpoint)
    await new Promise((resolve, reject) => {
      this.ws.once("open", resolve)
      this.ws.once("error", reject)
    })
    this.ws.on("message", (data) => this.handleMessage(data.toString("utf8")))
    await this.request("initialize", {
      clientInfo: { name: "arroba-context-injection-drill", version: "0.0.0" },
      capabilities: { experimentalApi: true },
    })
    this.ws.send(JSON.stringify({ jsonrpc: "2.0", method: "initialized", params: {} }))
  }

  close() {
    this.ws?.close()
  }

  handleMessage(raw) {
    let message
    try {
      message = JSON.parse(raw)
    } catch {
      this.notifications.push({ method: "parse-error", raw })
      return
    }
    if (message.id != null && message.method) {
      this.respondToServerRequest(message)
      return
    }
    if (message.id != null && this.pending.has(message.id)) {
      const pending = this.pending.get(message.id)
      this.pending.delete(message.id)
      if (message.error) pending.reject(new Error(message.error.message ?? JSON.stringify(message.error)))
      else pending.resolve(message.result)
      return
    }
    this.notifications.push(message)
  }

  respondToServerRequest(message) {
    const result = message.method === "item/permissions/requestApproval"
      ? { permissions: {}, scope: "turn" }
      : message.method === "mcpServer/elicitation/request"
        ? { action: "decline", content: null, _meta: null }
        : { decision: "decline" }
    this.ws.send(JSON.stringify({ jsonrpc: "2.0", id: message.id, result }))
  }

  request(method, params, timeoutMs = 120_000) {
    const id = this.nextId
    this.nextId += 1
    const payload = { jsonrpc: "2.0", id, method, params }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id)
        reject(new Error(`${method} timed out after ${timeoutMs}ms`))
      }, timeoutMs)
      this.pending.set(id, {
        resolve: (value) => {
          clearTimeout(timer)
          resolve(value)
        },
        reject: (error) => {
          clearTimeout(timer)
          reject(error)
        },
      })
      this.ws.send(JSON.stringify(payload))
    })
  }

  drainNotifications() {
    const notifications = this.notifications
    this.notifications = []
    return notifications
  }
}

async function runCodexDrill(options, root) {
  const provider = "codex"
  const port = await reservePort()
  const endpoint = `ws://127.0.0.1:${port}`
  const logs = {
    stdout: path.join(root, "codex.stdout.log"),
    stderr: path.join(root, "codex.stderr.log"),
  }
  const executable = process.env.ARROBA_CODEX_BIN?.trim() || "codex"
  const child = spawnLogged(executable, ["app-server", "--listen", endpoint], {
    cwd: options.worktree,
    stdout: logs.stdout,
    stderr: logs.stderr,
  })
  let client = null
  try {
    await waitForTcp(endpoint, child)
    client = new JsonRpcClient(endpoint)
    await client.connect()
    const model = providerModel(options, provider)
    const threadParams = {
      approvalPolicy: "never",
      approvalsReviewer: "user",
      sandbox: "read-only",
      sandboxPolicy: { type: "readOnly" },
      personality: "pragmatic",
      persistExtendedHistory: true,
      serviceName: "arroba-context-injection-drill",
      cwd: options.worktree,
    }
    if (model) threadParams.model = model
    const threadStart = await client.request("thread/start", threadParams, options.timeoutMs)
    const threadId = threadStart?.thread?.id
    if (!threadId) throw new Error(`Codex thread/start returned no thread id: ${JSON.stringify(threadStart)}`)
    const turnModel = model ?? threadStart?.model
    if (!turnModel) throw new Error(`Codex thread/start returned no model: ${JSON.stringify(threadStart)}`)

    const probe = makeProbe(provider)
    const first = await codexTurn(client, { options, threadId, model: turnModel, probe, token: probe.tokenA })
    const second = await codexTurn(client, { options, threadId, model: turnModel, probe, token: probe.tokenB })
    const midturnSteering = options.includeMidturnSteering
      ? await codexMidturnSteeringProbe(client, { options, threadId, model: turnModel, provider })
      : null
    const turns = await client.request("thread/turns/list", { threadId }, 30_000).catch((error) => ({ error: error.message }))
    const visibleHistory = JSON.stringify(turns)
    return summarizeProbe({
      provider,
      channel: "turn/start collaborationMode.settings.developer_instructions",
      sessionId: threadId,
      tokenA: probe.tokenA,
      tokenB: probe.tokenB,
      first,
      second,
      midturnSteering,
      hiddenTextVisibleInHistory: visibleHistory.includes(probe.tokenA) || visibleHistory.includes(probe.tokenB),
      visiblePromptIncludesHiddenText: probe.visiblePrompt.includes(probe.tokenA) || probe.visiblePrompt.includes(probe.tokenB),
      logs,
    })
  } finally {
    client?.close()
    await stopChild(child)
  }
}

async function codexMidturnSteeringProbe(client, { options, threadId, model, provider }) {
  const marker = `ARROBA_STEER_${provider.toUpperCase()}_${process.pid.toString(36)}_${Date.now().toString(36)}`
  const firstMarker = `${marker}_FIRST`
  const secondMarker = `${marker}_SECOND`
  client.drainNotifications()
  const common = {
    threadId,
    approvalPolicy: "never",
    approvalsReviewer: "user",
    personality: "pragmatic",
    sandbox: "read-only",
    sandboxPolicy: { type: "readOnly" },
    summary: "detailed",
    cwd: options.worktree,
  }
  if (model) common.model = model
  const firstStart = await client.request("turn/start", {
    ...common,
    input: [{ type: "text", text: [
      `Begin your answer with ${firstMarker}.`,
      "Then continue with a deliberately long numbered list from 1 to 120.",
      "Do not mention any second prompt unless one is delivered.",
    ].join(" ") }],
  }, options.timeoutMs)
  await sleep(750)
  let secondSubmit = "accepted"
  let secondError = null
  try {
    await client.request("turn/start", {
      ...common,
      input: [{ type: "text", text: `Midturn steering probe: include ${secondMarker} in your current answer if you can see this before finishing.` }],
    }, 20_000)
  } catch (error) {
    secondSubmit = "rejected"
    secondError = error.message
  }
  const observation = await collectCodexMidturnNotifications(client, { firstMarker, secondMarker, timeoutMs: Math.min(options.timeoutMs, 90_000) })
  return {
    channel: "codex turn/start while previous turn is active",
    firstStartTurnId: firstStart?.turn?.id ?? firstStart?.id ?? null,
    secondSubmit,
    secondError,
    firstMarker,
    secondMarker,
    ...observation,
  }
}

async function collectCodexMidturnNotifications(client, { firstMarker, secondMarker, timeoutMs }) {
  const deadline = Date.now() + timeoutMs
  let output = ""
  let completedTurns = 0
  const methods = []
  while (Date.now() < deadline) {
    for (const notification of client.drainNotifications()) {
      methods.push(notification.method)
      if (notification.method === "item/agentMessage/delta") {
        output += notification.params?.delta ?? ""
      } else if (notification.method === "item/completed") {
        const item = notification.params?.item
        if (item?.type === "agent_message" || item?.type === "message") output += item.text ?? item.content ?? ""
      } else if (notification.method === "turn/completed") {
        completedTurns += 1
      }
    }
    if (completedTurns > 0 && output.includes(firstMarker)) break
    if (output.includes(firstMarker) && output.includes(secondMarker)) break
    await sleep(250)
  }
  return {
    completedTurns,
    sawFirstMarker: output.includes(firstMarker),
    sawSecondMarker: output.includes(secondMarker),
    outputPreview: output.trim().slice(0, 2000),
    methods: [...new Set(methods)].filter(Boolean),
  }
}

async function codexTurn(client, { options, threadId, model, probe, token }) {
  client.drainNotifications()
  const params = {
    threadId,
    input: [{ type: "text", text: probe.visiblePrompt }],
    approvalPolicy: "never",
    approvalsReviewer: "user",
    personality: "pragmatic",
    sandbox: "read-only",
    sandboxPolicy: { type: "readOnly" },
    summary: "detailed",
    cwd: options.worktree,
    collaborationMode: {
      mode: "default",
      settings: {
        reasoning_effort: null,
        developer_instructions: hiddenInstruction(token),
      },
    },
  }
  if (model) {
    params.model = model
    params.collaborationMode.settings.model = model
  }
  params.input[0].text = visiblePromptForTurn(probe, token.endsWith("_A") ? "A" : "B")
  await client.request("turn/start", params, options.timeoutMs)
  return await waitForCodexTurn(client, token, options.timeoutMs)
}

async function waitForCodexTurn(client, expectedToken, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let output = ""
  let completed = false
  const rawMethods = []
  while (Date.now() < deadline) {
    for (const notification of client.drainNotifications()) {
      rawMethods.push(notification.method)
      if (notification.method === "item/agentMessage/delta") {
        output += notification.params?.delta ?? ""
      } else if (notification.method === "item/completed") {
        const item = notification.params?.item
        if (item?.type === "agent_message" || item?.type === "message") output += item.text ?? item.content ?? ""
      } else if (notification.method === "turn/completed") {
        completed = true
        const status = notification.params?.turn?.status
        const errorMessage = notification.params?.turn?.error?.message
        if (status && status !== "completed") throw new Error(`Codex turn completed with ${status}: ${errorMessage ?? ""}`)
      } else if (notification.method === "error") {
        throw new Error(`Codex error notification: ${JSON.stringify(notification.params)}`)
      }
    }
    if (completed && output.trim()) break
    if (output.includes(expectedToken) && completed) break
    await sleep(200)
  }
  return {
    expectedToken,
    output: output.trim(),
    matched: output.includes(expectedToken),
    completed,
    methods: [...new Set(rawMethods)].filter(Boolean),
  }
}

async function runOpenCodeDrill(options, root) {
  const provider = "opencode"
  const port = await reservePort()
  const baseUrl = `http://127.0.0.1:${port}`
  const logs = {
    stdout: path.join(root, "opencode.stdout.log"),
    stderr: path.join(root, "opencode.stderr.log"),
  }
  const executable = process.env.ARROBA_OPENCODE_BIN?.trim() || "opencode"
  const child = spawnLogged(executable, ["serve", "--hostname", "127.0.0.1", "--port", String(port)], {
    cwd: options.worktree,
    stdout: logs.stdout,
    stderr: logs.stderr,
  })
  try {
    await waitForOpenCode(baseUrl, child)
    const session = await fetchJson(new URL("/session", baseUrl), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({}),
    })
    const sessionId = session?.id
    if (!sessionId) throw new Error(`OpenCode session create returned no id: ${JSON.stringify(session)}`)
    const probe = makeProbe(provider)
    const first = await opencodeTurn(baseUrl, sessionId, options, probe, probe.tokenA, "a")
    await waitForOpenCodeIdle(baseUrl, sessionId, options.timeoutMs)
    await sleep(1_500)
    const second = await opencodeTurn(baseUrl, sessionId, options, probe, probe.tokenB, "b")
    await waitForOpenCodeIdle(baseUrl, sessionId, options.timeoutMs)
    const midturnSteering = options.includeMidturnSteering
      ? await opencodeMidturnSteeringProbe(baseUrl, sessionId, options, provider)
      : null
    const messages = await fetchJson(new URL(`/session/${sessionId}/message`, baseUrl))
    const visibleHistory = JSON.stringify(messages)
    return summarizeProbe({
      provider,
      channel: "POST /session/{id}/prompt_async body.system",
      sessionId,
      tokenA: probe.tokenA,
      tokenB: probe.tokenB,
      first,
      second,
      midturnSteering,
      hiddenTextVisibleInHistory: visibleHistory.includes(probe.tokenA) || visibleHistory.includes(probe.tokenB),
      visiblePromptIncludesHiddenText: probe.visiblePrompt.includes(probe.tokenA) || probe.visiblePrompt.includes(probe.tokenB),
      logs,
    })
  } finally {
    await stopChild(child)
  }
}

async function opencodeMidturnSteeringProbe(baseUrl, sessionId, options, provider) {
  const marker = `ARROBA_STEER_${provider.toUpperCase()}_${process.pid.toString(36)}_${Date.now().toString(36)}`
  const firstMarker = `${marker}_FIRST`
  const secondMarker = `${marker}_SECOND`
  const firstMessageId = nextOpenCodeMessageId()
  const secondMessageId = nextOpenCodeMessageId()
  const firstBody = opencodePromptBody(options, firstMessageId, [
    `Begin your answer with ${firstMarker}.`,
    "Then continue with a deliberately long numbered list from 1 to 120.",
    "Do not mention any second prompt unless one is delivered.",
  ].join(" "))
  await fetchJson(new URL(`/session/${sessionId}/prompt_async`, baseUrl), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(firstBody),
  }, [200, 204])
  await sleep(750)
  let secondSubmit = "accepted"
  let secondError = null
  try {
    const secondBody = opencodePromptBody(options, secondMessageId, `Midturn steering probe: include ${secondMarker} in your current answer if you can see this before finishing.`)
    await fetchJson(new URL(`/session/${sessionId}/prompt_async`, baseUrl), {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(secondBody),
    }, [200, 204])
  } catch (error) {
    secondSubmit = "rejected"
    secondError = error.message
  }
  const observation = await collectOpenCodeMidturnMessages(baseUrl, sessionId, {
    firstMessageId,
    secondMessageId,
    firstMarker,
    secondMarker,
    timeoutMs: Math.min(options.timeoutMs, 90_000),
  })
  return {
    channel: "opencode prompt_async while session status is busy",
    firstMessageId,
    secondMessageId,
    secondSubmit,
    secondError,
    firstMarker,
    secondMarker,
    ...observation,
  }
}

function opencodePromptBody(options, messageId, text) {
  const body = {
    messageID: messageId,
    parts: [{ type: "text", text }],
    agent: "build",
    tools: {
      bash: false,
      edit: false,
      write: false,
      apply_patch: false,
      multiedit: false,
      task: false,
    },
  }
  const model = providerModel(options, "opencode")
  const parsedModel = parseOpenCodeModel(model)
  if (parsedModel) body.model = parsedModel
  return body
}

async function collectOpenCodeMidturnMessages(baseUrl, sessionId, { firstMessageId, secondMessageId, firstMarker, secondMarker, timeoutMs }) {
  const deadline = Date.now() + timeoutMs
  let lastMessages = []
  while (Date.now() < deadline) {
    lastMessages = await fetchJson(new URL(`/session/${sessionId}/message`, baseUrl)).catch(() => [])
    const text = JSON.stringify(lastMessages)
    if (text.includes(firstMarker) && (text.includes(secondMarker) || text.includes(secondMessageId))) break
    const statusMap = await fetchJson(new URL("/session/status", baseUrl)).catch(() => ({}))
    if ((statusMap?.[sessionId]?.type ?? "idle") === "idle" && text.includes(firstMarker)) break
    await sleep(500)
  }
  const messages = Array.isArray(lastMessages) ? lastMessages : []
  const firstChildren = messages.filter((message) => message?.info?.parentID === firstMessageId)
  const secondChildren = messages.filter((message) => message?.info?.parentID === secondMessageId)
  const output = messages.map((message) =>
    (message.parts ?? []).map((part) => part.text ?? part.state?.output ?? "").join("")
  ).join("\n")
  return {
    firstAssistantChildren: firstChildren.length,
    secondAssistantChildren: secondChildren.length,
    sawFirstMarker: output.includes(firstMarker),
    sawSecondMarker: output.includes(secondMarker),
    outputPreview: output.trim().slice(0, 2000),
  }
}

async function runClaudeDrill(options, root, provider = "claude") {
  const dir = path.join(root, provider)
  await mkdir(dir, { recursive: true })
  const logs = {
    stdout: path.join(dir, "claude.stdout.jsonl"),
    stderr: path.join(dir, "claude.stderr.log"),
    events: path.join(dir, "events.jsonl"),
  }
  const contextFile = path.join(dir, "context.txt")
  const hookHandler = path.join(dir, "hook-handler.mjs")
  const settingsPath = path.join(dir, "settings.json")
  await writeFile(hookHandler, claudeHookHandlerSource(), "utf8")
  await writeFile(settingsPath, JSON.stringify({
    hooks: {
      UserPromptSubmit: [{ hooks: [{ type: "command", command: `node ${JSON.stringify(hookHandler)}` }] }],
    },
  }), "utf8")

  const executable = process.env.ARROBA_CLAUDE_BIN?.trim() || "claude"
  const model = providerModel(options, provider) ?? "sonnet"
  const child = spawn(executable, [
    "-p",
    "--input-format",
    "stream-json",
    "--output-format",
    "stream-json",
    "--verbose",
    "--settings",
    settingsPath,
    "--model",
    model,
    "--effort",
    "low",
    "--permission-mode",
    "dontAsk",
    "--replay-user-messages",
  ], {
    cwd: options.worktree,
    env: {
      ...process.env,
      ARROBA_CLAUDE_CONTEXT_DRILL_EVENTS: logs.events,
      ARROBA_CLAUDE_CONTEXT_DRILL_CONTEXT: contextFile,
    },
    detached: true,
    stdio: ["pipe", "pipe", "pipe"],
  })

  let stdoutBuffer = ""
  let stderr = ""
  let sessionId = null
  const stdoutLog = createWriteStream(logs.stdout, { flags: "a" })
  const stderrLog = createWriteStream(logs.stderr, { flags: "a" })
  const resultQueue = []
  const waiters = []
  child.stdout.on("data", (chunk) => {
    const text = chunk.toString("utf8")
    stdoutLog.write(text)
    stdoutBuffer += text
    let index
    while ((index = stdoutBuffer.indexOf("\n")) >= 0) {
      const line = stdoutBuffer.slice(0, index)
      stdoutBuffer = stdoutBuffer.slice(index + 1)
      if (!line.trim()) continue
      let message
      try {
        message = JSON.parse(line)
      } catch {
        continue
      }
      sessionId = message.session_id ?? sessionId
      if (message.type === "result") {
        resultQueue.push(message)
        while (waiters.length > 0) waiters.shift()?.()
      }
    }
  })
  child.stderr.on("data", (chunk) => {
    const text = chunk.toString("utf8")
    stderr += text
    stderrLog.write(text)
  })

  try {
    const probe = makeProbe(provider)
    const first = await claudeTurn(child, resultQueue, waiters, {
      options,
      contextFile,
      visiblePrompt: visiblePromptForTurn(probe, "A"),
      token: probe.tokenA,
    })
    const second = await claudeTurn(child, resultQueue, waiters, {
      options,
      contextFile,
      visiblePrompt: visiblePromptForTurn(probe, "B"),
      token: probe.tokenB,
    })
    const midturnSteering = options.includeMidturnSteering
      ? await claudeMidturnSteeringProbe(child, resultQueue, waiters, { options, contextFile, provider })
      : null
    const eventText = await readFile(logs.events, "utf8").catch(() => "")
    const transcriptPaths = eventText
      .split("\n")
      .filter(Boolean)
      .map((line) => {
        try {
          return JSON.parse(line).transcript_path
        } catch {
          return null
        }
      })
      .filter(Boolean)
    const transcriptText = (await Promise.all([...new Set(transcriptPaths)].map((file) =>
      readFile(file, "utf8").catch(() => ""),
    ))).join("\n")
    return summarizeProbe({
      provider,
      channel: "UserPromptSubmit hook hookSpecificOutput.additionalContext",
      sessionId,
      tokenA: probe.tokenA,
      tokenB: probe.tokenB,
      first,
      second,
      midturnSteering,
      hiddenTextVisibleInHistory: transcriptText.includes(probe.tokenA) || transcriptText.includes(probe.tokenB),
      visiblePromptIncludesHiddenText: probe.visiblePrompt.includes(probe.tokenA) || probe.visiblePrompt.includes(probe.tokenB),
      logs,
    })
  } finally {
    child.stdin.end()
    await stopChild(child)
    stdoutLog.end()
    stderrLog.end()
    if (stderr.trim()) {
      // Preserve stderr in artifacts; Claude may emit warnings even on successful turns.
    }
  }
}

async function runClaudeHeadlessDrill(options, root) {
  const provider = "claude-headless"
  const logs = {
    stdout: path.join(root, "claude-headless-prompt-assembly.stdout.log"),
    stderr: path.join(root, "claude-headless-prompt-assembly.stderr.log"),
  }
  const child = spawn("node", [
    path.join(repoRoot, "apps/cli/scripts/live-prompt-assembly-drill.mjs"),
    "--provider",
    provider,
    "--provider-model",
    "claude-headless=sonnet",
    "--timeout-ms",
    String(options.timeoutMs),
    "--poll-ms",
    "1000",
    "--keep-artifacts-on-failure",
  ], {
    cwd: repoRoot,
    env: process.env,
    stdio: ["ignore", "pipe", "pipe"],
  })
  let stdout = ""
  let stderr = ""
  child.stdout.on("data", (chunk) => {
    const text = chunk.toString("utf8")
    stdout += text
  })
  child.stderr.on("data", (chunk) => {
    const text = chunk.toString("utf8")
    stderr += text
  })
  const exit = await new Promise((resolve) => {
    child.once("close", (code, signal) => resolve({ code, signal }))
    child.once("error", (error) => resolve({ code: null, signal: null, error }))
  })
  await writeFile(logs.stdout, stdout, "utf8")
  await writeFile(logs.stderr, stderr, "utf8")
  if (exit.error || exit.code !== 0) {
    return {
      provider,
      status: "failed",
      reason: exit.error?.message ?? `prompt assembly drill exited with ${exit.code ?? exit.signal}`,
      stdout: stdout.slice(-2000),
      stderr: stderr.slice(-2000),
      logs,
    }
  }
  let parsed = null
  const jsonStart = stdout.lastIndexOf("\n{")
  const rawJson = jsonStart >= 0 ? stdout.slice(jsonStart + 1) : stdout.slice(stdout.indexOf("{"))
  try {
    parsed = JSON.parse(rawJson)
  } catch {}
  const result = parsed?.results?.find((item) => item.provider === provider)
  const midturnSteering = options.includeMidturnSteering
    ? await claudeHeadlessMidturnSteeringProbe(options, root)
    : null
  const midturnOk = !midturnSteering
    || (midturnSteering.firstCompleted && midturnSteering.classification !== "unobserved")
  return {
    provider,
    status: result?.status === "ok" && midturnOk ? "ok" : "failed",
    channel: "Arroba kernel Claude headless UserPromptSubmit additionalContext",
    sessionId: result?.providerRunId ?? null,
    perTurnContext: Boolean(result?.tokenSeenByModel),
    hiddenTextVisibleInPromptBlob: Boolean(result?.hiddenTokenVisibleInUserPromptHistory),
    hiddenTextVisibleInProviderHistoryApi: Boolean(result?.hiddenTokenVisibleInUserPromptHistory),
    first: { matched: Boolean(result?.tokenSeenByModel), completed: result?.status === "ok" },
    second: { matched: true, completed: true, note: "covered by prompt-assembly drill single-turn validation" },
    midturnSteering,
    logs,
  }
}

async function claudeHeadlessMidturnSteeringProbe(options, root) {
  const provider = "claude-headless"
  const runId = `${process.pid}-${Date.now()}`
  const runtimeDir = path.join(cliRoot, `.tmp-provider-context-claude-headless-steering-${runId}`)
  const rootDir = path.join(root, `claude-headless-steering-${runId}`)
  const workspace = path.join(rootDir, "workspace")
  const arrobaHome = path.join(rootDir, "arroba-home")
  const logs = {
    kernel: path.join(root, `claude-headless-steering-kernel-${runId}.log`),
  }
  await mkdir(workspace, { recursive: true })
  await mkdir(arrobaHome, { recursive: true })
  const { LocalIpcClient, requests } = await loadCliIpcModules(runtimeDir)
  const kernelPort = await reservePort()
  const kernelUrl = `ws://127.0.0.1:${kernelPort}`
  const kernelLog = createWriteStream(logs.kernel, { flags: "a" })
  const kernelBinary = path.join(repoRoot, "apps/kernel/target/debug/arroba-kernel")
  const daemon = spawn(kernelBinary, [], {
    cwd: repoRoot,
    env: {
      ...process.env,
      ARROBA_HOME: arrobaHome,
      XDG_CONFIG_HOME: path.join(rootDir, "xdg-config"),
      XDG_STATE_HOME: path.join(rootDir, "xdg-state"),
      ARROBA_LOG_DIR: path.join(rootDir, "logs"),
      ARROBA_LOG_LEVEL: "debug",
      ARROBA_CLAUDE_HEADLESS_DEBUG: "1",
      ARROBA_KERNEL_PORT: String(kernelPort),
      ARROBA_MCP_PORT: String(kernelPort + 1000),
      ARROBA_OPENCODE_PORT: String(kernelPort + 2000),
      ARROBA_CODEX_PORT: String(kernelPort + 2001),
      ARROBA_DAEMON_ID: `claude-headless-steering-${runId}`,
      ARROBA_DAEMON_SOCKET: path.join(rootDir, "daemon.sock"),
      ARROBA_SESSION_HISTORY_DIR: path.join(rootDir, "history"),
    },
    detached: true,
    stdio: ["ignore", "ignore", "pipe"],
  })
  daemon.stderr.pipe(kernelLog)
  let client = null
  try {
    await waitForKernelIpc(LocalIpcClient, requests.listSessionsRequest, kernelUrl, daemon)
    client = new LocalIpcClient(kernelUrl)
    const session = unwrap(
      await client.send(requests.createSessionRequest(workspace, workspace, "claude-headless-steering")),
      "SessionCreated",
    ).session
    await client.send(requests.setWorkspaceLiveSyncModeRequest(session.id, "unrestricted"))
    const attachment = unwrap(
      await client.send(requests.attachToSessionRequest(session.id, `claude-headless-steering-${runId}`)),
      "SessionAttached",
    ).attachment
    const launchResponse = await client.send(requests.launchProviderRunRequest(
      session.id,
      provider,
      "default",
      providerModel(options, provider) ?? "sonnet",
      "low",
      null,
      null,
    ))
    const launchPayload = launchResponse.ProviderRunLaunched ?? launchResponse.ProviderRunLaunchAccepted
    if (!launchPayload?.provider_run) throw new Error(`unexpected launch response: ${JSON.stringify(launchResponse)}`)
    await waitForProviderRunReady(client, requests, launchPayload.provider_run.id, options.timeoutMs)

    const firstMarker = `ARROBA_STEER_CLAUDE_HEADLESS_${randomBytes(4).toString("hex")}_A`
    const secondMarker = `ARROBA_STEER_CLAUDE_HEADLESS_${randomBytes(4).toString("hex")}_B`
    const firstPrompt = [
      "Midturn steering probe.",
      "Write twenty short numbered lines, then finish with this marker:",
      firstMarker,
      "If another user message arrives before you finish and contains another marker, include that other marker before the final line.",
    ].join(" ")
    const secondPrompt = `Midturn steering follow-up. Reply with exactly ${secondMarker}.`

    await client.send(requests.submitPromptRequest(session.id, attachment.id, null, firstPrompt, []))
    await waitForActivePrompt(client, requests, session.id, attachment.id, 15_000).catch(() => null)
    await sleep(750)
    const activeAtSecondSubmit = await sessionHasActivePrompt(client, requests, session.id, attachment.id)
    let secondSubmit = "accepted"
    let secondError = null
    try {
      await client.send(requests.submitPromptRequest(session.id, attachment.id, null, secondPrompt, []))
    } catch (error) {
      secondSubmit = "rejected"
      secondError = error.message
    }

    const turns = await waitForHeadlessSteeringTurns(
      client,
      requests,
      session.id,
      attachment.id,
      rootDir,
      { firstMarker, secondMarker },
      options.timeoutMs,
    )
    const firstTurn = turns.find((turn) => turn.userText.includes(firstMarker)) ?? null
    const secondTurn = turns.find((turn) => turn.userText.includes(secondMarker)) ?? null
    const rawProviderOutputs = await readRawProviderOutputs(rootDir, session.id)
    const firstOutputIndex = rawProviderOutputs.findIndex((text) => text.includes(firstMarker))
    const secondOutputIndex = rawProviderOutputs.findIndex((text) => text.includes(secondMarker))
    const firstCompleted = Boolean(firstTurn?.providerText.includes(firstMarker)) || firstOutputIndex >= 0
    const observedInFirst = Boolean(firstTurn?.providerText.includes(secondMarker))
      || (firstOutputIndex >= 0 && rawProviderOutputs[firstOutputIndex]?.includes(secondMarker))
    const observedInSecond = Boolean(secondTurn?.providerText.includes(secondMarker))
      || (secondOutputIndex >= 0 && (firstOutputIndex < 0 || secondOutputIndex > firstOutputIndex))
    const classification = observedInFirst
      ? "active-turn-fold"
      : observedInSecond
        ? (activeAtSecondSubmit ? "queued-next-turn" : "idle-next-turn")
        : secondSubmit === "rejected"
          ? "rejected"
          : "unobserved"
    await client.send(requests.endSessionRequest(session.id)).catch(() => {})
    return {
      firstMarker,
      secondMarker,
      secondSubmit,
      secondError,
      activeAtSecondSubmit,
      observedInFirst,
      observedInSecond,
      classification,
      firstCompleted,
      secondCompleted: observedInSecond,
      logs,
    }
  } finally {
    await client?.close?.().catch(() => {})
    await stopChild(daemon)
    kernelLog.end()
    await rm(runtimeDir, { recursive: true, force: true }).catch(() => {})
  }
}

async function sessionHasActivePrompt(client, requests, sessionId, attachmentId) {
  await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
  const response = await client.send(requests.getSessionStateRequest(sessionId))
  const session = response.SessionState?.session ?? response.SessionStateLoaded?.session ?? response.session ?? response
  return Boolean(session?.active_prompt)
}

async function waitForActivePrompt(client, requests, sessionId, attachmentId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await sessionHasActivePrompt(client, requests, sessionId, attachmentId)) return true
    await sleep(250)
  }
  throw new Error("timed out waiting for active prompt")
}

async function waitForHeadlessSteeringTurns(client, requests, sessionId, attachmentId, rootDir, markers, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastTurns = []
  while (Date.now() < deadline) {
    await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
    lastTurns = [
      ...await readSessionHistoryTurns(client, requests, sessionId),
      ...await readRawSessionHistoryTurns(rootDir, sessionId),
    ]
    const firstTurn = lastTurns.find((turn) => turn.userText.includes(markers.firstMarker))
    const secondTurn = lastTurns.find((turn) => turn.userText.includes(markers.secondMarker))
    if (firstTurn?.providerText.includes(markers.firstMarker)
      && (firstTurn.providerText.includes(markers.secondMarker) || secondTurn?.providerText.includes(markers.secondMarker))) {
      return lastTurns
    }
    const active = await sessionHasActivePrompt(client, requests, sessionId, attachmentId)
    if (!active && firstTurn?.providerText.includes(markers.firstMarker) && secondTurn) return lastTurns
    await sleep(1000)
  }
  return lastTurns
}

async function readSessionHistoryTurns(client, requests, sessionId) {
  const outline = unwrap(await client.send(requests.getSessionHistoryOutlineRequest(sessionId, null, 12)), "SessionHistoryOutline")
  const turns = []
  for (const agent of outline.agents ?? []) {
    for (const turn of agent.turns ?? []) {
      const userParts = []
      const providerParts = []
      if (turn.user_prompt?.entry?.text) userParts.push(turn.user_prompt.entry.text)
      for (const row of turn.entries ?? []) {
        const entry = row?.entry
        if (!entry?.text) continue
        if (entry.kind === "user_prompt") userParts.push(entry.text)
        else providerParts.push(entry.text)
      }
      if (turn.summary?.entry?.text) providerParts.push(turn.summary.entry.text)
      for (const blob of turn.blobs ?? []) {
        const blobContent = unwrap(
          await client.send(requests.getSessionHistoryBlobContentRequest(sessionId, agent.agent_id, blob.blob_id)),
          "SessionHistoryBlobContent",
        )
        for (const row of blobContent.entries ?? []) {
          const entry = row?.entry
          if (!entry?.text) continue
          if (entry.kind === "user_prompt") userParts.push(entry.text)
          else providerParts.push(entry.text)
        }
      }
      turns.push({ userText: userParts.join("\n"), providerText: providerParts.join("\n") })
    }
  }
  return turns
}

async function readRawSessionHistoryTurns(rootDir, sessionId) {
  const historyDir = path.join(rootDir, "history")
  const { readdir } = await import("node:fs/promises")
  const files = await readdir(historyDir).catch(() => [])
  const turns = []
  let current = null
  for (const file of files.filter((name) => name.startsWith(`${sessionId}-`) && name.endsWith(".jsonl"))) {
    const raw = await readFile(path.join(historyDir, file), "utf8").catch(() => "")
    for (const line of raw.split("\n").filter(Boolean)) {
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if (entry.kind === "user_prompt") {
        current = { userText: String(entry.text ?? ""), providerText: "" }
        turns.push(current)
      } else if (current && entry.kind !== "notice") {
        current.providerText += `\n${entry.text ?? ""}`
      }
    }
  }
  return turns
}

async function readRawProviderOutputs(rootDir, sessionId) {
  const historyDir = path.join(rootDir, "history")
  const { readdir } = await import("node:fs/promises")
  const files = await readdir(historyDir).catch(() => [])
  const outputs = []
  for (const file of files.filter((name) => name.startsWith(`${sessionId}-`) && name.endsWith(".jsonl"))) {
    const raw = await readFile(path.join(historyDir, file), "utf8").catch(() => "")
    for (const line of raw.split("\n").filter(Boolean)) {
      let entry
      try {
        entry = JSON.parse(line)
      } catch {
        continue
      }
      if (entry.kind === "provider_output") outputs.push(String(entry.text ?? ""))
    }
  }
  return outputs
}

function claudeHookHandlerSource() {
  return `import { appendFileSync, readFileSync } from "node:fs"

const chunks = []
for await (const chunk of process.stdin) chunks.push(chunk)
const raw = Buffer.concat(chunks).toString("utf8")
const input = raw.trim() ? JSON.parse(raw) : {}
appendFileSync(process.env.ARROBA_CLAUDE_CONTEXT_DRILL_EVENTS, JSON.stringify(input) + "\\n")
if (input.hook_event_name === "UserPromptSubmit") {
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: "UserPromptSubmit",
      additionalContext: readFileSync(process.env.ARROBA_CLAUDE_CONTEXT_DRILL_CONTEXT, "utf8"),
    },
  }))
}
`
}

async function claudeTurn(child, resultQueue, waiters, { options, contextFile, visiblePrompt, token }) {
  await writeFile(contextFile, hiddenInstruction(token), "utf8")
  child.stdin.write(JSON.stringify({
    type: "user",
    message: {
      role: "user",
      content: [{ type: "text", text: visiblePrompt }],
    },
  }) + "\n")
  const result = await waitForClaudeResult(resultQueue, waiters, options.timeoutMs)
  return {
    expectedToken: token,
    output: String(result.result ?? "").trim(),
    matched: String(result.result ?? "").includes(token),
    completed: result.subtype === "success" && result.is_error === false,
    stopReason: result.stop_reason ?? null,
  }
}

async function claudeMidturnSteeringProbe(child, resultQueue, waiters, { options, contextFile, provider }) {
  const firstMarker = `ARROBA_STEER_${provider.toUpperCase()}_${randomBytes(4).toString("hex")}_A`
  const secondMarker = `ARROBA_STEER_${provider.toUpperCase()}_${randomBytes(4).toString("hex")}_B`
  await writeFile(contextFile, "No hidden context is required for this steering probe.", "utf8")
  child.stdin.write(JSON.stringify({
    type: "user",
    message: {
      role: "user",
      content: [{
        type: "text",
        text: `Midturn steering probe. Think briefly, then write five short numbered lines, then finish with ${firstMarker}. If another user message arrives before you finish, include its marker too.`,
      }],
    },
  }) + "\n")
  await sleep(750)
  let secondSubmit = "accepted"
  let secondError = null
  try {
    child.stdin.write(JSON.stringify({
      type: "user",
      message: {
        role: "user",
        content: [{ type: "text", text: `Midturn steering follow-up marker: ${secondMarker}` }],
      },
    }) + "\n")
  } catch (error) {
    secondSubmit = "rejected"
    secondError = error.message
  }

  const firstResult = await waitForClaudeResult(resultQueue, waiters, options.timeoutMs)
  const firstOutput = String(firstResult.result ?? "")
  let secondResult = null
  if (!firstOutput.includes(secondMarker) && secondSubmit === "accepted") {
    secondResult = await waitForClaudeResult(resultQueue, waiters, Math.min(options.timeoutMs, 60_000)).catch((error) => ({ error: error.message }))
  }
  const secondOutput = secondResult && !secondResult.error ? String(secondResult.result ?? "") : ""
  return {
    firstMarker,
    secondMarker,
    secondSubmit,
    secondError,
    firstCompleted: firstResult.subtype === "success" && firstResult.is_error === false,
    firstStopReason: firstResult.stop_reason ?? null,
    observedInFirst: firstOutput.includes(secondMarker),
    observedInSecond: secondOutput.includes(secondMarker),
    secondCompleted: secondResult && !secondResult.error
      ? secondResult.subtype === "success" && secondResult.is_error === false
      : false,
    secondStopReason: secondResult && !secondResult.error ? secondResult.stop_reason ?? null : null,
    secondResultError: secondResult?.error ?? null,
  }
}

async function waitForClaudeResult(resultQueue, waiters, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const result = resultQueue.shift()
    if (result) return result
    await new Promise((resolve) => {
      const timer = setTimeout(resolve, Math.min(500, Math.max(1, deadline - Date.now())))
      waiters.push(() => {
        clearTimeout(timer)
        resolve()
      })
    })
  }
  throw new Error(`timed out waiting for Claude result after ${timeoutMs}ms`)
}

async function waitForOpenCode(baseUrl, child) {
  const deadline = Date.now() + 15_000
  while (Date.now() < deadline) {
    if (child.exitCode != null) throw new Error(`opencode serve exited before becoming ready: ${child.exitCode}`)
    try {
      const response = await fetch(new URL("/global/health", baseUrl))
      if (response.ok) return
    } catch {
      // keep polling
    }
    await sleep(150)
  }
  throw new Error(`timed out waiting for ${baseUrl}`)
}

async function waitForOpenCodeIdle(baseUrl, sessionId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const statusMap = await fetchJson(new URL("/session/status", baseUrl)).catch(() => ({}))
    const status = statusMap?.[sessionId]?.type ?? "idle"
    if (status === "idle") return
    await sleep(250)
  }
  throw new Error(`timed out waiting for OpenCode session ${sessionId} to become idle`)
}

async function opencodeTurn(baseUrl, sessionId, options, probe, token, suffix) {
  const messageId = nextOpenCodeMessageId()
  const body = {
    messageID: messageId,
    system: hiddenInstruction(token),
    parts: [{ type: "text", text: visiblePromptForTurn(probe, suffix.toUpperCase()) }],
    agent: "build",
    tools: {
      bash: false,
      edit: false,
      write: false,
      apply_patch: false,
      multiedit: false,
      task: false,
    },
  }
  const model = providerModel(options, "opencode")
  const parsedModel = parseOpenCodeModel(model)
  if (parsedModel) body.model = parsedModel
  await fetchJson(new URL(`/session/${sessionId}/prompt_async`, baseUrl), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  }, [200, 204])
  return await waitForOpenCodeMessage(baseUrl, sessionId, messageId, token, options.timeoutMs)
}

let opencodeMessageSequence = 0

function nextOpenCodeMessageId() {
  opencodeMessageSequence = (opencodeMessageSequence + 1) & 0x0fff
  const encodedTime = ((BigInt(Date.now()) * 0x1000n) + BigInt(opencodeMessageSequence)).toString(16).slice(-12).padStart(12, "0")
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
  const bytes = randomBytes(14)
  let suffix = ""
  for (const byte of bytes) suffix += alphabet[byte % alphabet.length]
  return `msg_${encodedTime}${suffix}`
}

function parseOpenCodeModel(model) {
  if (!model || model === "default") return null
  const [providerID, modelID] = model.split("/", 2)
  if (!providerID || !modelID) return null
  return { providerID, modelID }
}

async function waitForOpenCodeMessage(baseUrl, sessionId, userMessageId, expectedToken, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  let lastMessages = []
  while (Date.now() < deadline) {
    lastMessages = await fetchJson(new URL(`/session/${sessionId}/message`, baseUrl)).catch(() => [])
    const messages = Array.isArray(lastMessages) ? lastMessages : []
    const assistantMessages = messages.filter((message) =>
      message?.info?.role === "assistant" && message?.info?.parentID === userMessageId
    )
    for (const message of assistantMessages) {
      const output = (message.parts ?? []).map((part) => part.text ?? part.state?.output ?? "").join("")
      const completed = Boolean(message.info?.time?.completed || message.info?.finish)
      if (completed && output.trim()) {
        return {
          expectedToken,
          output: output.trim(),
          matched: output.includes(expectedToken),
          completed,
          finish: message.info?.finish ?? null,
        }
      }
    }
    await sleep(500)
  }
  return {
    expectedToken,
    output: JSON.stringify(lastMessages).slice(-2000),
    matched: false,
    completed: false,
  }
}

async function fetchJson(url, init, okStatuses = [200]) {
  const response = await fetch(url, init)
  if (!okStatuses.includes(response.status)) {
    const text = await response.text().catch(() => "")
    throw new Error(`${init?.method ?? "GET"} ${url} failed with ${response.status}: ${text.slice(0, 1000)}`)
  }
  if (response.status === 204) return null
  return await response.json()
}

function summarizeProbe(input) {
  const firstMatchedOnlyA = input.first.matched && !input.first.output.includes(input.tokenB)
  const secondMatchedOnlyB = input.second.matched && !input.second.output.includes(input.tokenA)
  return {
    provider: input.provider,
    status: firstMatchedOnlyA && secondMatchedOnlyB ? "ok" : "failed",
    channel: input.channel,
    sessionId: input.sessionId,
    perTurnContext: secondMatchedOnlyB,
    hiddenTextVisibleInPromptBlob: Boolean(input.visiblePromptIncludesHiddenText),
    hiddenTextVisibleInProviderHistoryApi: Boolean(input.hiddenTextVisibleInHistory),
    first: input.first,
    second: input.second,
    midturnSteering: input.midturnSteering ?? null,
    logs: input.logs,
  }
}

async function main() {
  const options = parseArgs(process.argv)
  const root = path.join(os.tmpdir(), `arroba-provider-context-drill-${process.pid}-${Date.now()}`)
  await mkdir(root, { recursive: true })
  const results = []
  let failed = false
  try {
    for (const provider of options.providers) {
      if (provider === "codex") {
        results.push(await runCodexDrill(options, root))
      } else if (provider === "opencode") {
        results.push(await runOpenCodeDrill(options, root))
      } else if (provider === "claude" || provider === "claude-p") {
        results.push(await runClaudeDrill(options, root, provider))
      } else if (provider === "claude-headless") {
        results.push(await runClaudeHeadlessDrill(options, root))
      } else {
        results.push({ provider, status: "skipped", reason: "no drill implemented for this provider" })
      }
    }
    failed = results.some((result) => result.status === "failed")
    console.log(JSON.stringify({ status: failed ? "failed" : "ok", artifacts: root, results }, null, 2))
  } catch (error) {
    failed = true
    console.error(error)
    console.error(JSON.stringify({ status: "failed", artifacts: root, results }, null, 2))
  } finally {
    if (!failed || !options.keepArtifacts) {
      await rm(root, { recursive: true, force: true }).catch(() => {})
    }
  }
  if (failed) process.exitCode = 1
}

main()
