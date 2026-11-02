import type { FastifyReply, FastifyRequest } from "fastify"
import type { Invocation } from "@chariox/agent-trigger-gateway"
import { agentTriggerErrorPage } from "@chariox/agent-trigger-gateway/page"
export type AgentTriggerBinding = {kernelId: string; sessionId: string; triggerId: string}

export type TriggerIngressConnection = {
  send<T>(request: unknown): Promise<T>
  close(): Promise<void>
}

// MP-08 / MP-09 / MP-10 / MP-11: ingress accepts work; it never attaches to output.
export async function invokeAgentTrigger(request: FastifyRequest, reply: FastifyReply, binding: AgentTriggerBinding,
  base: string, connect: () => Promise<TriggerIngressConnection>): Promise<unknown> {
  const raw = request.body instanceof Buffer ? request.body : Buffer.alloc(0)
  const contentType = String(request.headers["content-type"] ?? "text/plain")
  const type = contentType.split(";")[0]!.trim().toLowerCase()
  let body: unknown = raw.toString("utf8")
  if (request.method === "POST") {
    if (type === "application/json") {
      try { body = raw.length ? JSON.parse(raw.toString("utf8")) : {} }
      catch { return reply.code(400).send({error: "Invalid JSON body"}) }
    } else if (type === "application/x-www-form-urlencoded") body = Object.fromEntries(new URLSearchParams(raw.toString("utf8")))
    else if (type !== "text/plain") return reply.code(415).send({error: "Use JSON, form or plain text"})
  } else body = null
  const query = request.query as Record<string, unknown>
  const prompt = typeof body === "string" ? body : body && typeof body === "object" ? (body as Record<string, unknown>).prompt : query.prompt
  let kernel: TriggerIngressConnection | undefined
  reply.header("Cache-Control", "no-store")
  try {
    kernel = await connect()
    const response = await kernel.send<{AgentTriggerInvoked: {invocation: Invocation}}>({InvokeAgentTrigger: {
      session_id: binding.sessionId, trigger_id: binding.triggerId,
      payload: {...(prompt !== undefined ? {prompt} : {}), query, body,
        http: {body_base64: raw.toString("base64"), content_type: contentType}},
    }})
    const result = response.AgentTriggerInvoked.invocation
    if (result.status === "accepted") {
      if (String(request.headers.accept).includes("text/html")) return reply.code(303).header("Location", `${base}#firing-${encodeURIComponent(result.prompt_id)}`).send()
      return reply.code(202).send({firing_id: result.prompt_id, prompt_id: result.prompt_id, outcome: result.outcome})
    }
    if (result.status !== "offline") return reply.code(({disabled: 403, busy: 429, payload_too_large: 413} as const)[result.status]).send({error: result.status.replaceAll("_", " ")})
  } catch (error) {
    const status = (error as {statusCode?: number}).statusCode
    if (status && status !== 503) return reply.code(status).send({error: status === 502 ? "delivery unconfirmed; check agent history before sending again" : "agent request refused"})
  } finally { await kernel?.close() }
  reply.code(503).header("Retry-After", "5")
  return String(request.headers.accept).includes("text/html")
    ? reply.type("text/html; charset=utf-8").send(agentTriggerErrorPage("agent offline", base))
    : reply.send({error: "agent offline"})
}
