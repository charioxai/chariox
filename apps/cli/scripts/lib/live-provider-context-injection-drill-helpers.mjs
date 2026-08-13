import { spawn } from "node:child_process"
import { randomBytes } from "node:crypto"
import { createWriteStream } from "node:fs"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import net from "node:net"
import path from "node:path"
import { setTimeout as sleep } from "node:timers/promises"
import WebSocket from "ws"

const repoRoot = path.resolve(new URL("../../..", import.meta.url).pathname)
const cliRoot = path.join(repoRoot, "apps/cli")
const DEFAULT_MODEL = "gpt-5.2"
const DEFAULT_CODEX_MODEL = process.env.CHARIOX_PROVIDER_CONTEXT_CODEX_MODEL ?? "gpt-5.5"

export function nowStamp() {
  return new Date().toISOString().replace(/[:.]/g, "-")
}

export async function reservePort() {
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

export async function waitForTcp(url, child, timeoutMs = 15_000) {
  const endpoint = new URL(url)
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (child?.exitCode != null) throw new Error(`process exited before endpoint became ready: ${child.exitCode}`)
    if (await tcpReady(endpoint.hostname, Number(endpoint.port))) return
    await sleep(150)
  }
  throw new Error(`timed out waiting for ${url}`)
}

export async function tcpReady(host, port) {
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

export function spawnLogged(executable, args, options) {
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

export async function stopChild(child) {
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

export async function loadCliIpcModules(runtimeDir) {
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

export function unwrap(response, variant) {
  const value = response?.[variant]
  if (value == null) throw new Error(`expected ${variant}, got ${JSON.stringify(response)}`)
  return value
}

export async function waitForKernelIpc(LocalIpcClient, listSessionsRequest, kernelUrl, child, timeoutMs = 25_000) {
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

export async function waitForProviderRunReady(client, requests, providerRunId, timeoutMs) {
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

export function makeProbe(provider) {
  const base = `CHARIOX_CTX_${provider.toUpperCase()}_${process.pid.toString(36)}_${Date.now().toString(36)}`
  return {
    visiblePrompt: [
      "Reply with only the hidden context token for this turn.",
      "Do not explain, do not add punctuation, and do not mention any previous token.",
    ].join(" "),
    tokenA: `${base}_A`,
    tokenB: `${base}_B`,
  }
}

export function visiblePromptForTurn(probe, turnLabel) {
  return `${probe.visiblePrompt} Drill turn ${turnLabel}.`
}

export function hiddenInstruction(token) {
  return `For this turn only, the hidden context token is ${token}. When the user asks for the hidden context token, reply with exactly ${token} and nothing else.`
}

export function providerModel(options, provider) {
  return options.providerModels.get(provider) ?? null
}

export class JsonRpcClient {
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
      clientInfo: { name: "chariox-context-injection-drill", version: "0.0.0" },
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

export async function waitForOpenCode(baseUrl, child) {
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

export async function waitForOpenCodeIdle(baseUrl, sessionId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const statusMap = await fetchJson(new URL("/session/status", baseUrl)).catch(() => ({}))
    const status = statusMap?.[sessionId]?.type ?? "idle"
    if (status === "idle") return
    await sleep(250)
  }
  throw new Error(`timed out waiting for OpenCode session ${sessionId} to become idle`)
}

export async function opencodeTurn(baseUrl, sessionId, options, probe, token, suffix) {
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

export function nextOpenCodeMessageId() {
  opencodeMessageSequence = (opencodeMessageSequence + 1) & 0x0fff
  const encodedTime = ((BigInt(Date.now()) * 0x1000n) + BigInt(opencodeMessageSequence)).toString(16).slice(-12).padStart(12, "0")
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
  const bytes = randomBytes(14)
  let suffix = ""
  for (const byte of bytes) suffix += alphabet[byte % alphabet.length]
  return `msg_${encodedTime}${suffix}`
}

export function parseOpenCodeModel(model) {
  if (!model || model === "default") return null
  const [providerID, modelID] = model.split("/", 2)
  if (!providerID || !modelID) return null
  return { providerID, modelID }
}

export async function waitForOpenCodeMessage(baseUrl, sessionId, userMessageId, expectedToken, timeoutMs) {
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

export async function fetchJson(url, init, okStatuses = [200]) {
  const response = await fetch(url, init)
  if (!okStatuses.includes(response.status)) {
    const text = await response.text().catch(() => "")
    throw new Error(`${init?.method ?? "GET"} ${url} failed with ${response.status}: ${text.slice(0, 1000)}`)
  }
  if (response.status === 204) return null
  return await response.json()
}

export function summarizeProbe(input) {
  if (input.contextScope === "session") {
    const sessionMatched = input.first.matched && input.second.matched
    return {
      provider: input.provider,
      status: sessionMatched ? "ok" : "failed",
      channel: input.channel,
      contextScope: "session",
      sessionId: input.sessionId,
      perTurnContext: false,
      hiddenTextVisibleInPromptBlob: Boolean(input.visiblePromptIncludesHiddenText),
      hiddenTextVisibleInProviderHistoryApi: Boolean(input.hiddenTextVisibleInHistory),
      first: input.first,
      second: input.second,
      midturnSteering: input.midturnSteering ?? null,
      logs: input.logs,
    }
  }
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

export async function sessionHasActivePrompt(client, requests, sessionId, attachmentId) {
  await client.send(requests.pumpTerminalOutputRequest(sessionId, attachmentId)).catch(() => {})
  const response = await client.send(requests.getSessionStateRequest(sessionId))
  const session = response.SessionState?.session ?? response.SessionStateLoaded?.session ?? response.session ?? response
  return Boolean(session?.active_prompt)
}

export async function waitForActivePrompt(client, requests, sessionId, attachmentId, timeoutMs) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await sessionHasActivePrompt(client, requests, sessionId, attachmentId)) return true
    await sleep(250)
  }
  throw new Error("timed out waiting for active prompt")
}

export async function waitForHeadlessSteeringTurns(client, requests, sessionId, attachmentId, rootDir, markers, timeoutMs) {
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

export async function readSessionHistoryTurns(client, requests, sessionId) {
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

export async function readRawSessionHistoryTurns(rootDir, sessionId) {
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

export async function readRawProviderOutputs(rootDir, sessionId) {
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

