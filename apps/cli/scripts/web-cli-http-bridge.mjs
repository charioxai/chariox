#!/usr/bin/env node
import { createServer } from "node:http"

import {
  LocalIpcClient,
  attachToSessionRequest,
  cancelActivePromptRequest,
  focusAgentRequest,
  getSessionStateRequest,
  submitPromptRequest,
} from "@arroba/kernel-client"

const host = process.env.ARROBA_WEB_CLI_BRIDGE_HOST ?? "127.0.0.1"
const port = Number.parseInt(process.env.ARROBA_WEB_CLI_BRIDGE_PORT ?? process.env.PORT ?? "43210", 10)
const secret = process.env.ARROBA_WEB_CLI_BRIDGE_SECRET ?? null
const kernelEndpoint = process.env.ARROBA_WEB_CLI_KERNEL_URL
  ?? process.env.ARROBA_KERNEL_URL
  ?? defaultKernelEndpoint()
const clientId = process.env.ARROBA_WEB_CLI_CLIENT_ID ?? `arroba-web-cli-${process.pid}`
const client = new LocalIpcClient(kernelEndpoint)
const attachments = new Map()

if (!Number.isInteger(port) || port <= 0) {
  throw new Error("ARROBA_WEB_CLI_BRIDGE_PORT must be a positive integer")
}

const server = createServer(async (request, response) => {
  try {
    if (!isAuthorized(request)) {
      sendJson(response, 401, { error: { code: "authorization_denied", message: "Bridge authorization failed" } })
      return
    }
    if (request.method === "GET" && request.url === "/web-cli/state") {
      sendJson(response, 200, {
        runtime: {
          status: "CONNECTED",
          kernelEndpoint,
          clientId,
        },
      })
      return
    }
    if (request.method === "POST" && request.url === "/web-cli/prompt") {
      const input = await readJson(request)
      const sessionId = requireString(input.sessionId, "sessionId")
      const agentRef = requireString(input.agentRef, "agentRef")
      const prompt = requireString(input.prompt, "prompt")
      const attachmentId = await ensureAttachment(sessionId)
      const targetAgentId = await resolveAgentId(sessionId, agentRef)
      const result = await client.send(submitPromptRequest(sessionId, attachmentId, targetAgentId, prompt, []))
      sendJson(response, 200, {
        accepted: true,
        sessionId,
        agentId: targetAgentId,
        attachmentId,
        result,
      })
      return
    }
    if (request.method === "POST" && request.url === "/web-cli/stop") {
      const input = await readJson(request)
      const sessionId = requireString(input.sessionId, "sessionId")
      const agentRef = requireString(input.agentRef, "agentRef")
      const attachmentId = await ensureAttachment(sessionId)
      const agentId = await resolveAgentId(sessionId, agentRef)
      await client.send(focusAgentRequest(sessionId, agentId))
      const result = await client.send(cancelActivePromptRequest(sessionId, attachmentId))
      sendJson(response, 200, {
        accepted: true,
        sessionId,
        agentId,
        attachmentId,
        result,
      })
      return
    }
    sendJson(response, 404, { error: { code: "not_found", message: "Route not found" } })
  } catch (error) {
    sendJson(response, 400, {
      error: {
        code: "web_cli_bridge_error",
        message: error instanceof Error ? error.message : String(error),
      },
    })
  }
})

server.listen(port, host, () => {
  console.log(`[web-cli-http-bridge] listening on http://${host}:${port}`)
  console.log(`[web-cli-http-bridge] kernel ${kernelEndpoint}`)
})

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    server.close(() => process.exit(0))
  })
}

function defaultKernelEndpoint() {
  const kernelHost = process.env.ARROBA_KERNEL_HOST ?? "127.0.0.1"
  const kernelPort = process.env.ARROBA_KERNEL_PORT ?? "43118"
  return `ws://${kernelHost}:${kernelPort}/kernel`
}

function isAuthorized(request) {
  if (!secret) {
    return true
  }
  return request.headers.authorization === `Bearer ${secret}`
}

async function ensureAttachment(sessionId) {
  const existing = attachments.get(sessionId)
  if (existing) {
    return existing
  }
  const response = await client.send(attachToSessionRequest(sessionId, clientId))
  const attachmentId = response?.SessionAttached?.attachment?.id
    ?? response?.attachment?.id
  if (typeof attachmentId !== "string" || attachmentId.length === 0) {
    throw new Error("Kernel did not return an attachment id")
  }
  attachments.set(sessionId, attachmentId)
  return attachmentId
}

async function resolveAgentId(sessionId, agentRef) {
  const response = await client.send(getSessionStateRequest(sessionId))
  const session = response?.SessionState?.session ?? response?.session
  const agents = Array.isArray(session?.agents) ? session.agents : []
  const agent = agents.find((candidate) =>
    candidate.id === agentRef
    || candidate.agent_ref === agentRef
    || candidate.alias === agentRef
  )
  if (!agent?.id) {
    throw new Error(`Agent ${agentRef} was not found in session ${sessionId}`)
  }
  return agent.id
}

function requireString(value, name) {
  if (typeof value !== "string" || value.length === 0) {
    throw new Error(`${name} is required`)
  }
  return value
}

async function readJson(request) {
  const chunks = []
  for await (const chunk of request) {
    chunks.push(chunk)
  }
  const body = Buffer.concat(chunks).toString("utf8")
  return body ? JSON.parse(body) : {}
}

function sendJson(response, statusCode, payload) {
  response.writeHead(statusCode, {
    "content-type": "application/json; charset=utf-8",
  })
  response.end(JSON.stringify(payload))
}
