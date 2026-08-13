import { createServer } from "node:http"

import { WebSocket } from "ws"

let protocolTimeoutMs = 300_000
let runtimeStartTimeoutMs = 90_000

export function createPublicationTraceProtocol(options = {}) {
  protocolTimeoutMs = options.protocolTimeoutMs ?? protocolTimeoutMs
  runtimeStartTimeoutMs = options.runtimeStartTimeoutMs ?? runtimeStartTimeoutMs
  return {
    freePort,
    invokeTransport,
    waitForGatewayReady,
    waitForPublicationStatus,
  }
}

async function waitForGatewayReady(localUrl) {
  const statusUrl = new URL("/.well-known/chariox/publication/status", localUrl).toString()
  const deadline = Date.now() + runtimeStartTimeoutMs
  let lastError = null
  while (Date.now() < deadline) {
    try {
      const response = await fetch(statusUrl, { signal: AbortSignal.timeout(1_000) })
      if (response.ok) return
      lastError = new Error(`status endpoint returned HTTP ${response.status}`)
    } catch (error) {
      lastError = error
    }
    await sleep(500)
  }
  const message = lastError instanceof Error ? lastError.message : String(lastError)
  throw new Error(`publication runtime at ${localUrl} did not become ready: ${message}`)
}

async function invokeTransport(localUrl, transport, provider, policy) {
  if (transport.id === "schedule_only") return { transport: transport.id, message: "no ingress; status endpoint only" }
  const prompt = promptFor(provider, transport, policy)
  const base = new URL(localUrl)
  if (transport.id === "human_http") {
    const url = new URL(`/prompt/${encodeURIComponent(prompt)}`, base)
    return await invokeHumanHttp(url.toString())
  }
  if (transport.id === "api_sse_json") {
    const url = new URL("/invoke", base)
    return await invokeApiSse(url.toString(), {
      method: "POST",
      headers: { accept: "text/event-stream", "content-type": "application/json" },
      body: JSON.stringify({ prompt }),
    })
  }
  if (transport.id === "websocket_json") {
    const url = new URL("/socket", base)
    url.protocol = url.protocol === "https:" ? "wss:" : "ws:"
    return await invokeWebSocket(url.toString(), { type: "invoke", input: { prompt } })
  }
  if (transport.id === "mcp") {
    const url = new URL("/mcp", base)
    const initialize = await postJson(url, { jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2025-03-26" } })
    const tools = await postJson(url, { jsonrpc: "2.0", id: 2, method: "tools/list" })
    const toolName = tools?.result?.tools?.[0]?.name
    const calledRaw = await postJsonCapture(url, { jsonrpc: "2.0", id: 3, method: "tools/call", params: { name: toolName, arguments: { prompt } } })
    return { transport: transport.id, url: url.toString(), initialize, tools, called: calledRaw.json, called_raw: calledRaw }
  }
  throw new Error(`unsupported transport ${transport.id}`)
}

async function invokeHumanHttp(url) {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(new Error(`human_http invocation timed out for ${url}`)), protocolTimeoutMs)
  let status = null
  let body = ""
  try {
    const response = await fetch(url, { headers: { accept: "application/json" }, signal: controller.signal })
    status = response.status
    body = await response.text()
    return { transport: "human_http", url, status, body }
  } catch (error) {
    return {
      transport: "human_http",
      url,
      status,
      body,
      timed_out: error?.name === "AbortError" || /timed out/i.test(String(error?.message ?? error)),
      error: error instanceof Error ? error.message : String(error),
    }
  } finally {
    clearTimeout(timeout)
  }
}

async function waitForPublicationStatus(localUrl, transport) {
  const statusUrl = new URL("/.well-known/chariox/publication/status", localUrl).toString()
  const deadline = Date.now() + (transport.id === "schedule_only" ? 180_000 : 30_000)
  let last = null
  while (Date.now() < deadline) {
    last = await fetchJson(statusUrl)
    if (transport.id !== "schedule_only") return last
    if (last?.latest_run?.id || last?.last_run?.id || last?.runs?.length > 0) return last
    await sleep(1_000)
  }
  return last
}

function promptFor(provider, transport, policy) {
  const suffix = `${provider.provider}_${transport.id}_${policy.id}`.replace(/[^a-z0-9_]/gi, "_")
  return [
    `Run live publication trace validation for ${suffix}.`,
    `Follow the workflow node instructions exactly and keep channel-specific marker text in the channel requested by each node.`,
    `The final workflow output should only contain the final-output marker requested by the finalizer node.`,
  ].join("\n")
}

async function fetchJson(url) {
  const response = await fetch(url)
  const text = await response.text()
  try {
    return JSON.parse(text)
  } catch {
    return { status: response.status, text }
  }
}

async function postJson(url, payload) {
  const captured = await postJsonCapture(url, payload)
  if (captured.json) return captured.json
  throw new Error(captured.error ?? `invalid JSON response from ${url}`)
}

async function postJsonCapture(url, payload) {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(new Error(`JSON request timed out for ${url}`)), protocolTimeoutMs)
  let status = null
  let text = ""
  try {
    const response = await fetch(url, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
      signal: controller.signal,
    })
    status = response.status
    const reader = response.body?.getReader()
    if (!reader) {
      text = await response.text()
    } else {
      const decoder = new TextDecoder()
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        text += decoder.decode(value, { stream: true })
      }
      text += decoder.decode()
    }
    return parseCapturedJson(status, text)
  } catch (error) {
    const timedOut = error?.name === "AbortError" || /timed out/i.test(String(error?.message ?? error))
    return {
      status,
      text,
      json: null,
      timed_out: timedOut,
      error: error instanceof Error ? error.message : String(error),
    }
  } finally {
    clearTimeout(timeout)
  }
}

function parseCapturedJson(status, text) {
  try {
    return { status, text, json: JSON.parse(text.trim()), timed_out: false, error: null }
  } catch (error) {
    return {
      status,
      text,
      json: null,
      timed_out: false,
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

async function invokeApiSse(url, init) {
  const controller = new AbortController()
  const timeout = setTimeout(() => controller.abort(new Error(`api_sse_json invocation timed out for ${url}`)), protocolTimeoutMs)
  const events = []
  let body = ""
  try {
    const response = await fetch(url, { ...init, signal: controller.signal })
    const reader = response.body?.getReader()
    if (!reader) return { transport: "api_sse_json", url, status: response.status, body, events }
    const decoder = new TextDecoder()
    while (true) {
      const { done, value } = await reader.read()
      if (done) break
      body += decoder.decode(value, { stream: true })
      parseSseEvents(body, events)
      if (events.some((event) => event.event === "final" || event.event === "error" || event.event === "timeout")) break
    }
    body += decoder.decode()
    parseSseEvents(body, events)
    await reader.cancel().catch(() => {})
    return { transport: "api_sse_json", url, status: response.status, body, events }
  } catch (error) {
    return {
      transport: "api_sse_json",
      url,
      status: null,
      body,
      events,
      timed_out: error?.name === "AbortError" || /timed out/i.test(String(error?.message ?? error)),
      error: error instanceof Error ? error.message : String(error),
    }
  } finally {
    clearTimeout(timeout)
  }
}

function parseSseEvents(body, events) {
  const parsedCount = events.reduce((count, event) => count + (event.raw?.length ?? 0), 0)
  const remaining = body.slice(parsedCount)
  const blocks = remaining.split(/\n\n/)
  for (let index = 0; index < blocks.length - 1; index += 1) {
    const raw = `${blocks[index]}\n\n`
    let eventName = "message"
    let data = ""
    for (const line of blocks[index].split(/\n/)) {
      if (line.startsWith("event:")) eventName = line.slice("event:".length).trim()
      if (line.startsWith("data:")) data += line.slice("data:".length).trim()
    }
    let parsed = data
    try {
      parsed = JSON.parse(data)
    } catch {
      // Keep raw string data for malformed or non-JSON event payloads.
    }
    events.push({ event: eventName, data: parsed, raw })
  }
}

async function invokeWebSocket(url, payload) {
  return await new Promise((resolve, reject) => {
    const messages = []
    const socket = new WebSocket(url)
    const timeout = setTimeout(() => {
      socket.close()
      resolve({ transport: "websocket_json", url, messages, timed_out: true })
    }, protocolTimeoutMs)
    socket.on("message", (data) => {
      const text = String(data)
      messages.push(text)
      if (text.includes('"type":"ready"')) socket.send(JSON.stringify(payload))
      if (text.includes('"type":"final"') || text.includes('"type":"error"')) {
        clearTimeout(timeout)
        socket.close()
        resolve({ transport: "websocket_json", url, messages })
      }
    })
    socket.on("error", (error) => {
      clearTimeout(timeout)
      reject(error)
    })
  })
}

async function freePort() {
  return await new Promise((resolve, reject) => {
    const server = createServer()
    server.listen(0, "127.0.0.1", () => {
      const address = server.address()
      const port = typeof address === "object" && address ? address.port : null
      server.close(() => port ? resolve(port) : reject(new Error("failed to allocate free port")))
    })
    server.on("error", reject)
  })
}

async function sleep(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}
