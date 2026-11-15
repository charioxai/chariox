import type { FastifyReply, FastifyRequest } from "fastify"
import { decodeBody, invokeHttpTrigger, sendPageMessage, type AgentHttpOptions, type HttpTriggerKernel, type HttpTriggerResponse, type Invocation, type TriggerReply } from "@chariox/agent-trigger-gateway/http-trigger"
import { agentTriggerErrorPage } from "@chariox/agent-trigger-gateway/page"
export type AgentTriggerBinding = {kernelId: string; sessionId: string; triggerId: string}

export type TriggerIngressConnection = {
  send<T>(request: unknown): Promise<T>
  close(): Promise<void>
}

/** The kernel requests of one firing over the ingress connection: options, the invocation and its reply. */
function ingressKernel(kernel: TriggerIngressConnection, binding: AgentTriggerBinding): HttpTriggerKernel {
  const trigger = {session_id: binding.sessionId, trigger_id: binding.triggerId}
  return {
    options: async () => (await kernel.send<{AgentTriggerHttpOptionsRead: {enabled: boolean; options: AgentHttpOptions}}>({ReadAgentTriggerHttpOptions: trigger})).AgentTriggerHttpOptionsRead,
    invoke: async (input, http) => (await kernel.send<{AgentTriggerInvoked: {invocation: Invocation}}>({InvokeAgentTrigger: {...trigger, input, ...(http ? {http} : {})}})).AgentTriggerInvoked.invocation,
    reply: async promptId => (await kernel.send<{AgentTriggerReplyRead: {reply: TriggerReply}}>({ReadAgentTriggerReply: {...trigger, prompt_id: promptId}})).AgentTriggerReplyRead.reply,
  }
}

// MP-08 / MP-09 / MP-10 / MP-11: ingress accepts work and reads only its own firing's reply; it never attaches to output.
// `path` is the request path below the trigger's base; "send" is the trigger page's own message box.
export async function invokeAgentTrigger(request: FastifyRequest, reply: FastifyReply, binding: AgentTriggerBinding,
  base: string, path: string, connect: () => Promise<TriggerIngressConnection>): Promise<unknown> {
  const raw = request.body instanceof Buffer ? request.body : Buffer.alloc(0)
  const contentType = String(request.headers["content-type"] ?? "text/plain")
  const decoded = decodeBody(request.method, raw, contentType)
  reply.header("Cache-Control", "no-store")
  if ("status" in decoded) return reply.code(decoded.status).send(decoded.body)
  let kernel: TriggerIngressConnection | undefined
  try {
    kernel = await connect()
    const operations = ingressKernel(kernel, binding)
    let result: HttpTriggerResponse
    if (path === "send") {
      const prompt = (decoded.body as {prompt?: unknown} | null)?.prompt
      if (request.method !== "POST" || typeof prompt !== "string" || !prompt.trim()) return reply.code(400).send({error: "invalid request body"})
      result = await sendPageMessage(prompt, operations)
    } else {
      const url = new URL(request.url, "https://ingress.local")
      result = await invokeHttpTrigger({method: request.method, path: `/${path}`, search: url.search, query: request.query as Record<string, unknown>,
        headers: request.headers, body: decoded.body, raw, contentType}, operations, base)
    }
    if (result.status !== 503) {
      reply.code(result.status)
      for (const [name, value] of Object.entries(result.headers)) reply.header(name, value)
      return reply.send(result.body)
    }
  } catch (error) {
    const status = (error as {statusCode?: number}).statusCode
    if (status && status !== 503) return reply.code(status).send({error: status === 502 ? "delivery unconfirmed; check agent history before sending again" : "agent request refused"})
  } finally { await kernel?.close() }
  reply.code(503).header("Retry-After", "5")
  return String(request.headers.accept).includes("text/html")
    ? reply.type("text/html; charset=utf-8").send(agentTriggerErrorPage("agent offline", base))
    : reply.send({error: "agent offline"})
}
