import Fastify, {type FastifyRequest, type FastifyReply} from "fastify"
import {AgentTriggerIngressClient, type AgentTriggerRelayBootstrap} from "@chariox/kernel-client/agent-trigger-ingress-client"
import {agentTriggerPage} from "@chariox/agent-trigger-gateway/page"
import {invokeAgentTrigger, type AgentTriggerBinding, type TriggerIngressConnection} from "./hosted-agent-trigger-invoke.js"

type Deployment = AgentTriggerBinding & {id: string}
type Admission = {deployment: Deployment; bootstrap?: AgentTriggerRelayBootstrap}
type Options = {
  publicOrigin: string
  controlOrigin?: string
  connect?: typeof AgentTriggerIngressClient.connect
}
const forwardedHeaders = ["cookie", "x-chariox-browser-session", "csrf-token", "x-csrf-token", "origin", "sec-fetch-site", "sec-fetch-mode", "accept"] as const

/** MP-08 / MP-09 / MP-10 / MP-11: this is runtime ingress, separate from Cloud and the opaque relay. */
export function buildHostedAgentTriggerServer(options: Options) {
  const publicOrigin = new URL(options.publicOrigin)
  if (publicOrigin.protocol !== "https:" || publicOrigin.origin !== options.publicOrigin) throw Error("Set the exact HTTPS public origin for agent trigger ingress")
  const controlOrigin = new URL(options.controlOrigin ?? options.publicOrigin)
  if (controlOrigin.protocol !== "https:" && !(controlOrigin.protocol === "http:" && ["127.0.0.1", "localhost", "[::1]"].includes(controlOrigin.hostname))) throw Error("Cloud admission requires HTTPS or a local control endpoint")
  const app = Fastify({logger: false, bodyLimit: 65_536})
  app.removeAllContentTypeParsers()
  app.addContentTypeParser("*", {parseAs: "buffer"}, (_request, body, done) => done(null, body))
  const admitted = new WeakMap<FastifyRequest, {row: Deployment; headers: Record<string, string>}>()
  const limits = new Map<string, {tokens: number; at: number}>()
  const access = (request: FastifyRequest) => request.method === "POST" || (request.method === "GET" && !String(request.headers["sec-purpose"] ?? request.headers.purpose ?? "").toLowerCase().includes("prefetch") && Object.hasOwn(request.query as object, "prompt")) ? "operate" : "read"
  const metadata = async (request: FastifyRequest, reply: FastifyReply, thumbprint?: string): Promise<Admission | undefined> => {
    const saved = admitted.get(request)
    const headers: Record<string, string> = saved?.headers ?? {"content-type": "application/json", host: publicOrigin.host, "x-forwarded-host": publicOrigin.host, "x-forwarded-proto": "https"}
    if (!saved) for (const name of forwardedHeaders) {const value = request.headers[name]; if (typeof value === "string") headers[name] = value}
    const id = encodeURIComponent((request.params as {id: string}).id)
    const target = new URL(`/browser/agent-trigger-deployments/${id}/admission`, controlOrigin)
    target.searchParams.set("access", access(request)); target.searchParams.set("method", request.method)
    // Only authentication and binding metadata cross this control-plane boundary.
    const response = await fetch(target, {method: "POST", headers, body: JSON.stringify(thumbprint ? {publicKeyThumbprint: thumbprint} : {}), redirect: "manual", signal: AbortSignal.timeout(15_000)})
    // Preserve the owner's public security policy when moving the document off Cloud.
    for (const name of ["content-security-policy", "content-security-policy-report-only", "x-content-type-options", "x-frame-options", "referrer-policy", "cross-origin-opener-policy", "strict-transport-security"]) {
      const value = response.headers.get(name)
      if (value) reply.header(name, value)
    }
    const cookies = response.headers.getSetCookie()
    if (cookies.length) {
      reply.header("set-cookie", cookies)
      const jar = new Map((headers.cookie ?? "").split(/;\s*/).filter(Boolean).map(value => [value.split("=")[0]!, value]))
      for (const cookie of cookies) {const value = cookie.split(";")[0]!; jar.set(value.split("=")[0]!, value)}
      headers.cookie = [...jar.values()].join("; ")
    }
    if (!response.ok) throw Object.assign(Error("Cloud admission refused"), {statusCode: [400, 401, 403, 404, 429, 503].includes(response.status) ? response.status : 503})
    const value = await response.json() as Admission
    if (!value.deployment || value.deployment.id !== decodeURIComponent(id) || ![value.deployment.kernelId, value.deployment.sessionId, value.deployment.triggerId].every(item => typeof item === "string" && item.length > 0 && item.length < 256)) throw Error("Cloud returned an invalid agent binding")
    if (saved && JSON.stringify(saved.row) !== JSON.stringify(value.deployment)) throw Error("Agent deployment changed; reopen its address")
    admitted.set(request, {row: value.deployment, headers})
    return value
  }
  app.addHook("onClose", async () => {limits.clear()})
  app.get("/ready", async () => ({status: "ready"}))
  for (const url of ["/agent-triggers/:id", "/agent-triggers/:id/", "/agent-triggers/:id/*"]) app.route({method: ["GET", "HEAD", "POST"], url, logLevel: "silent",
    onRequest: async (request, reply) => {await metadata(request, reply)},
    handler: async (request, reply) => {
      const row = admitted.get(request)!.row, base = `/agent-triggers/${encodeURIComponent(row.id)}/`
      const path = (request.params as {"*"?: string})["*"] ?? ""
      if (["events", "history", "resend"].includes(path)) return reply.code(410).send({error: "Runtime history uses the browser's encrypted kernel connection"})
      if (path) return reply.code(404).send({error: "trigger not found"})
      if (access(request) === "read") return reply.header("Cache-Control", "private, no-cache").type("text/html; charset=utf-8").send(agentTriggerPage({alias: "Chariox", online: false, busy: false, enabled: true, entries: [], turns: [], nextCursor: null, unconfirmed: []}, base, undefined, {scriptUrl: "/assets/agent-trigger-viewer.js", deploymentId: row.id}))
      const now = performance.now(), limit = limits.get(row.id) ?? {tokens: 3, at: now}
      limit.tokens = Math.min(3, limit.tokens + (now - limit.at) / 10_000); limit.at = now; limits.set(row.id, limit)
      if (limit.tokens < 1) return reply.code(429).header("Retry-After", "10").send({error: "too many requests; retry shortly"})
      limit.tokens--
      return invokeAgentTrigger(request, reply, row, base, async (): Promise<TriggerIngressConnection> => {
        const connect = options.connect ?? AgentTriggerIngressClient.connect
        const client = await connect(row.kernelId, async thumbprint => {
          const next = await metadata(request, reply, thumbprint)
          if (!next?.bootstrap) throw Object.assign(Error("agent access denied"), {statusCode: reply.statusCode >= 400 ? reply.statusCode : 403})
          return next.bootstrap
        }, publicOrigin.origin)
        try { await metadata(request, reply); return client }
        catch (error) { await client.close(); throw error }
      })
    },
  })
  app.setErrorHandler((error, request, reply) => {
    const status = (error as {statusCode?: number}).statusCode ?? 503
    if (status === 401 && request.method === "GET" && String(request.headers.accept).includes("text/html")) return reply.redirect(`/auth/login?returnTo=${encodeURIComponent(request.url.split("?")[0]!)}`)
    return reply.code(status).header("Cache-Control", "no-store").header("Retry-After", "5").send({error: status === 401 ? "Sign in to open this agent" : status === 403 ? "agent access denied" : status === 404 ? "trigger not found" : status === 413 ? "payload too large" : "agent offline"})
  })
  return app
}
