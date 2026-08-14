import { createServer } from "node:http"

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
