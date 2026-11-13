import Fastify from "fastify"
import type { FastifyReply, FastifyRequest } from "fastify"
import { readFile, readdir } from "node:fs/promises"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"
import type { ServerResponse } from "node:http"
import { AgentTriggerClient } from "./agent-trigger-client.js"
import { installRawBodyParsers } from "./publication-raw-body-parsers.js"
import { agentTriggerPage, agentTriggerErrorPage } from "./agent-trigger-page.js"
import { foreignRequestReason } from "./agent-trigger-origin-guard.js"
import { invocationPath, invokeHttpTrigger, sendPageMessage, type HttpTriggerKernel, type HttpTriggerResponse } from "./agent-trigger-http.js"

export function loopbackHost(value: string): string {
  if (value !== "127.0.0.1" && value !== "::1") throw Error("agent HTTP triggers bind to loopback only")
  return value
}

export async function buildAgentTriggerServer(client: Pick<AgentTriggerClient, "metadata" | "options" | "subscribe" | "invoke" | "reply" | "resend" | "conversation" | "workspaceImage" | "onChange" | "close">, host: string, options: { base?: string; localOriginGuard?: boolean; csrfToken?: string } = {}) {
  loopbackHost(host)
  const meta = await client.metadata()
  if (meta.trigger.source.kind !== "http") throw Error("only an HTTP trigger has a gateway")
  const ingress = meta.trigger.source.ingress_id
  const base = options.base ?? `/${ingress}/`
  const app = Fastify({ bodyLimit: 65_536, logger: false })
  installRawBodyParsers(app)
  const assets = new Map<string, Buffer>()
  const assetRoot = join(dirname(fileURLToPath(import.meta.url)), "agent-trigger-assets")
  async function readAssets(path: string, prefix = "") {
    for (const file of await readdir(path, { withFileTypes: true })) {
      const name = prefix + file.name
      if (file.isDirectory()) await readAssets(join(path, file.name), name + "/")
      else assets.set(name, await readFile(join(path, file.name)))
    }
  }
  await readAssets(assetRoot)
  type Viewer = { response: ServerResponse; blocked: boolean; stale: boolean }
  const viewers = new Set<Viewer>()
  const sendEvent = (viewer: Viewer, event: string) => {
    if (viewer.response.destroyed || viewer.response.writableEnded) return
    if (viewer.blocked) { viewer.stale = true; return }
    if (!viewer.response.write(event)) {
      viewer.blocked = true
      viewer.response.once("drain", () => {
        viewer.blocked = false
        if (viewer.stale) { viewer.stale = false; void refresh() }
      })
    }
  }
  let refreshing = false, refreshPending = false
  const refresh = async () => {
    refreshPending = true
    if (refreshing || !viewers.size) return
    refreshing = true
    try {
      while (refreshPending && viewers.size) {
        refreshPending = false
        const conversation = await client.conversation()
        const event = `event: conversation\ndata: ${JSON.stringify(conversation)}\n\n`
        for (const viewer of viewers) sendEvent(viewer, event)
      }
    } catch {
      for (const viewer of viewers) sendEvent(viewer, 'event: offline\ndata: {}\n\n')
    } finally { refreshing = false }
  }
  const unsubscribe = client.onChange(records => {
    if (records) {
      const event = `event: output\ndata: ${JSON.stringify(records)}\n\n`
      for (const viewer of viewers) sendEvent(viewer, event)
    } else { void refresh() }
  })
  let tokens = 3, lastRefill = performance.now()
  const admit = () => {
    const now = performance.now()
    tokens = Math.min(3, tokens + (now - lastRefill) * 6 / 60_000)
    lastRefill = now
    if (tokens < 1) return false
    tokens--
    return true
  }
  const limit = { admit, refund: () => { tokens = Math.min(3, tokens + 1) } }
  const failure = (request: FastifyRequest, reply: FastifyReply, status: number, error: string, hint?: string) => {
    reply.code(status).header("Cache-Control", "no-store")
    if (String(request.headers.accept).includes("text/html")) return reply.type("text/html; charset=utf-8").send(agentTriggerErrorPage(error, base, hint))
    return { error }
  }
  const kernel: HttpTriggerKernel = { options: () => client.options(), invoke: (input, http) => client.invoke(input, http), reply: id => client.reply(id) }
  const respond = (request: FastifyRequest, reply: FastifyReply, result: HttpTriggerResponse) => {
    if (result.status >= 400) {
      for (const [name, value] of Object.entries(result.headers)) reply.header(name, value)
      return failure(request, reply, result.status, (result.body as { error: string }).error)
    }
    reply.code(result.status)
    for (const [name, value] of Object.entries(result.headers)) reply.header(name, value)
    return reply.send(result.body)
  }
  // The route, methods and parser are the trigger's; the rate limit is this gateway's.
  const invoke = async (request: FastifyRequest, reply: FastifyReply) => {
    const url = new URL(request.url, "http://127.0.0.1")
    const raw = Buffer.from((request.raw as { charioxRawBody?: string }).charioxRawBody ?? "", "utf8")
    return respond(request, reply, await invokeHttpTrigger({ method: request.method, path: url.pathname.slice(base.length - 1), search: url.search,
      query: request.query as Record<string, unknown>, headers: request.headers, body: request.body ?? null, raw,
      contentType: String(request.headers["content-type"] ?? "text/plain") }, kernel, base, limit))
  }
  app.addHook("onRequest", async (request, reply) => {
    const reason = options.localOriginGuard === false ? null : foreignRequestReason(request, base)
    if (!reason) return
    const refusal = failure(request, reply, 403, reason, "Another website sent this request. Open this address in your browser on this computer.")
    return refusal === reply ? reply : reply.send(refusal)
  })
  app.get(base + "health", async () => ({ ingress_id: ingress }))
  app.get(base + "assets/*", async (request, reply) => {
    const name = (request.params as { "*": string })["*"]
    const bytes = assets.get(name)
    if (!bytes) return reply.code(404).send({ error: "not found" })
    const type = name.endsWith(".css") ? "text/css" : name.endsWith(".woff2") ? "font/woff2" : name.endsWith(".woff") ? "font/woff" : name.endsWith(".ttf") ? "font/ttf" : "text/javascript"
    return reply.type(type).send(bytes)
  })
  const viewer = async (request: FastifyRequest, reply: FastifyReply) => {
    const conversation = await client.conversation()
    if (!conversation.online) reply.header("Retry-After", "5")
    reply.code(!conversation.enabled ? 403 : !conversation.online ? 503 : 200).header("Cache-Control", "no-store").type("text/html; charset=utf-8")
    return agentTriggerPage(conversation, base, options.csrfToken)
  }
  const every = ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
  const path = invocationPath((await client.options()).options)
  // The page lives at the base; a request with a query there, or any request at another route, is a firing.
  app.route({ method: every, url: base, exposeHeadRoute: false, handler: async (request, reply) => {
    const prefetch = String(request.headers["sec-purpose"] ?? request.headers.purpose ?? "").toLowerCase().includes("prefetch")
    const read = request.method === "HEAD" || (request.method === "GET" && (prefetch || !request.url.includes("?")))
    if (read || path !== "/") return read ? viewer(request, reply) : failure(request, reply, 404, "trigger not found")
    return invoke(request, reply)
  } })
  if (path !== "/") app.route({ method: every, url: base + path.slice(1), exposeHeadRoute: false, handler: invoke })
  app.post(base + "send", async (request, reply) => {
    const prompt = (request.body as { prompt?: unknown } | null)?.prompt
    if (typeof prompt !== "string" || !prompt.trim()) return failure(request, reply, 400, "invalid request body")
    if (!admit()) { reply.header("Retry-After", "10"); return failure(request, reply, 429, "too many requests; retry shortly") }
    const result = await sendPageMessage(prompt, kernel)
    if (result.status === 503) limit.refund()
    return respond(request, reply, result)
  })
  app.post(base + "resend", async (request, reply) => {
    const promptId = (request.body as { prompt_id?: unknown } | null)?.prompt_id
    if (typeof promptId !== "string" || !promptId) return failure(request, reply, 400, "invalid request body")
    if (!admit()) { reply.header("Retry-After", "10"); return failure(request, reply, 429, "too many requests; retry shortly") }
    // The kernel sends a firing again once; a second click or an old page finds it already handled.
    try { await client.resend(promptId) } catch { return failure(request, reply, 409, "this message is no longer waiting to be sent again") }
    reply.header("Cache-Control", "no-store")
    return reply.code(202).send({ prompt_id: promptId })
  })
  // Pictures a reply references in the agent's workspace, so this page shows them as the web terminal does.
  app.get(base + "workspace-image", async (request, reply) => {
    const path = (request.query as { path?: unknown }).path
    if (typeof path !== "string" || !path) return reply.code(400).send({ error: "invalid image path" })
    const image = await client.workspaceImage(path).catch(() => null)
    if (!image) return reply.code(404).header("Cache-Control", "no-store").send({ error: "image unavailable" })
    return reply.header("Cache-Control", "no-store").header("X-Content-Type-Options", "nosniff").type(image.mime).send(image.bytes)
  })
  app.get(base + "history", async (request, reply) => {
    const cursor = (request.query as { before?: string }).before
    const before = cursor === undefined ? null : Number(cursor)
    if (before !== null && (!Number.isSafeInteger(before) || before < 0)) return reply.code(400).send({ error: "invalid history cursor" })
    return client.conversation(before === null ? null : { before_sequence: before })
  })
  app.get(base + "events", (request, reply) => {
    reply.hijack()
    reply.raw.writeHead(200, { "Content-Type": "text/event-stream", "Cache-Control": "no-cache, no-store", Connection: "keep-alive" })
    reply.raw.write(": connected\n\n")
    const viewer = { response: reply.raw, blocked: false, stale: false }
    viewers.add(viewer)
    const heartbeat = setInterval(() => { if (!reply.raw.destroyed) reply.raw.write(": keepalive\n\n") }, 15_000)
    request.raw.once("close", () => { clearInterval(heartbeat); viewers.delete(viewer) })
    void refresh()
  })
  app.setErrorHandler((error, request, reply) => {
    const code = error && typeof error === "object" && "statusCode" in error ? Number(error.statusCode) : 503
    const message = error instanceof Error ? error.message : "agent offline"
    const status = code === 413 ? 413 : [400, 401, 403, 404, 409, 415, 502].includes(code) ? code : /not found|no longer exists|agent_not_found/.test(message) ? 404 : 503
    return failure(request, reply, status, status === 413 ? "payload too large" : status === 404 ? "trigger not found" : status === 502 ? "delivery unconfirmed; check agent history before sending again" : status === 401 || status === 403 ? "agent access denied" : status === 409 ? "agent request refused" : status === 400 || status === 415 ? "invalid request body" : "agent offline")
  })
  app.addHook("onClose", async () => {
    unsubscribe()
    for (const viewer of viewers) viewer.response.end()
    viewers.clear()
    await client.close()
  })
  return app
}
