import Fastify, {type FastifyRequest, type FastifyReply} from "fastify"
import {AgentTriggerIngressClient, type AgentTriggerRelayBootstrap} from "@chariox/kernel-client/agent-trigger-ingress-client"
import {agentTriggerPage} from "@chariox/agent-trigger-gateway/page"
import {invokeAgentTrigger, type TriggerIngressConnection} from "./hosted-agent-trigger-invoke.js"
import type {TriggerAdmission, TriggerDeployment} from "./agent-trigger-admission.js"

type Admission = {deployment: TriggerDeployment; bootstrap?: AgentTriggerRelayBootstrap}
type Options = {
  publicOrigin: string
  admission: TriggerAdmission
  /** Where a signed-out browser navigation goes. Without it a refused sign-in is a plain 401. */
  signInPath?: string
  connect?: typeof AgentTriggerIngressClient.connect
}
const securityHeaders = ["content-security-policy", "content-security-policy-report-only", "x-content-type-options", "x-frame-options", "referrer-policy", "cross-origin-opener-policy", "strict-transport-security"]

/** MP-08 / MP-09 / MP-10 / MP-11: this is runtime ingress, separate from Cloud and the opaque relay. */
export function buildHostedAgentTriggerServer(options: Options) {
  const publicOrigin = new URL(options.publicOrigin)
  if (publicOrigin.protocol !== "https:" || publicOrigin.origin !== options.publicOrigin) throw Error("Set the exact HTTPS public origin for agent trigger ingress")
  const app = Fastify({logger: false, bodyLimit: 65_536})
  app.removeAllContentTypeParsers()
  app.addContentTypeParser("*", {parseAs: "buffer"}, (_request, body, done) => done(null, body))
  const admitted = new WeakMap<FastifyRequest, {row: TriggerDeployment; headers: Record<string, string>}>()
  const limits = new Map<string, {tokens: number; at: number}>()
  const access = (request: FastifyRequest) => request.method === "POST" || (request.method === "GET" && !String(request.headers["sec-purpose"] ?? request.headers.purpose ?? "").toLowerCase().includes("prefetch") && Object.hasOwn(request.query as object, "prompt")) ? "operate" : "read"
  const metadata = async (request: FastifyRequest, reply: FastifyReply, thumbprint?: string): Promise<Admission | undefined> => {
    const saved = admitted.get(request)
    const headers: Record<string, string> = saved?.headers ?? {}
    if (!saved) for (const name of options.admission.forwardedHeaders) {const value = request.headers[name]; if (typeof value === "string") headers[name] = value}
    const id = (request.params as {id: string}).id
    // Only authentication and binding metadata cross this boundary.
    const response = await options.admission.admit({deploymentId: id, access: access(request), method: request.method, headers, ...(thumbprint ? {thumbprint} : {})})
    // Preserve the owner's public security policy when serving the document from this ingress.
    for (const name of securityHeaders) {
      const value = response.headers[name]
      if (value) reply.header(name, value)
    }
    if (response.cookies.length) {
      reply.header("set-cookie", response.cookies)
      const jar = new Map((headers.cookie ?? "").split(/;\s*/).filter(Boolean).map(value => [value.split("=")[0]!, value]))
      for (const cookie of response.cookies) {const value = cookie.split(";")[0]!; jar.set(value.split("=")[0]!, value)}
      headers.cookie = [...jar.values()].join("; ")
    }
    if (response.status < 200 || response.status > 299) throw Object.assign(Error("admission refused"), {statusCode: [400, 401, 403, 404, 429, 503].includes(response.status) ? response.status : 503})
    const value = response.value
    if (!value?.deployment || value.deployment.id !== id || ![value.deployment.kernelId, value.deployment.sessionId, value.deployment.triggerId].every(item => typeof item === "string" && item.length > 0 && item.length < 256)) throw Error("admission returned an invalid agent binding")
    if (saved && JSON.stringify(saved.row) !== JSON.stringify(value.deployment)) throw Error("Agent deployment changed; reopen its address")
    admitted.set(request, {row: value.deployment, headers})
    return value as Admission
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
    if (status === 401 && options.signInPath && request.method === "GET" && String(request.headers.accept).includes("text/html")) return reply.redirect(`${options.signInPath}?returnTo=${encodeURIComponent(request.url.split("?")[0]!)}`)
    return reply.code(status).header("Cache-Control", "no-store").header("Retry-After", "5").send({error: status === 401 ? "Sign in to open this agent" : status === 403 ? "agent access denied" : status === 404 ? "trigger not found" : status === 413 ? "payload too large" : "agent offline"})
  })
  return app
}
