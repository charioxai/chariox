import { createServer, type IncomingMessage, type ServerResponse } from "node:http"
import process from "node:process"

import { LocalIpcClient } from "../ipc.js"
import { requestNativeProviderInteractionRequest } from "../ipc-requests.js"

export type ClaudePromptOriginState = {
  current: "native" | "external" | null
}

export async function startClaudePermissionBridge(options: {
  client: LocalIpcClient
  sessionId: string
  attachmentId: string
  agentId: string
  promptOrigin: ClaudePromptOriginState
}): Promise<{ url: string; stop: () => Promise<void> }> {
  const server = createServer((request, response) => {
    void handleClaudePermissionBridgeRequest(options, request, response)
  })
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject)
    server.listen(0, "127.0.0.1", () => {
      server.off("error", reject)
      resolve()
    })
  })
  const address = server.address()
  if (!address || typeof address === "string") {
    await new Promise<void>((resolve) => server.close(() => resolve()))
    throw new Error("failed to start Claude permission bridge")
  }
  return {
    url: `http://127.0.0.1:${address.port}`,
    stop: () => new Promise((resolve) => server.close(() => resolve())),
  }
}

async function handleClaudePermissionBridgeRequest(
  options: {
    client: LocalIpcClient
    sessionId: string
    attachmentId: string
    agentId: string
    promptOrigin: ClaudePromptOriginState
  },
  request: IncomingMessage,
  response: ServerResponse,
) {
  if (request.method !== "POST" || request.url !== "/permission") {
    writeJsonResponse(response, 404, { error: "not found" })
    return
  }
  try {
    const payload = await readJsonRequest(request)
    if (!shouldBridgeClaudePermission(payload)) {
      writeJsonResponse(response, 200, { handled: false })
      return
    }
    const toolName = typeof payload.tool_name === "string" ? payload.tool_name : "tool"
    const interactionId = `claude-native-permission-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}`
    const interactionResponse = await options.client.send<Record<string, unknown>>(
      requestNativeProviderInteractionRequest(
        options.sessionId,
        options.agentId,
        interactionId,
        `Approve Claude Code ${toolName}?`,
        formatClaudePermissionMessage(payload),
        300,
      ),
    )
    const resolution = expectVariant<{ resolution: { status?: string; choice_id?: string | null; reply?: string | null } }>(
      interactionResponse,
      "NativeProviderInteractionResolved",
    ).resolution
    const allowed = resolution.reply === "allow" || resolution.choice_id === "allow_once"
    writeJsonResponse(response, 200, {
      handled: true,
      permissionDecision: allowed ? "allow" : "deny",
      permissionDecisionReason: allowed
        ? "Approved through Chariox."
        : resolution.status === "timed_out"
          ? "Timed out waiting for Chariox approval."
          : "Denied through Chariox.",
    })
  } catch (error) {
    writeJsonResponse(response, 500, {
      error: error instanceof Error ? error.message : String(error),
    })
  }
}

type ClaudePermissionPayload = {
  hook_event_name?: unknown
  permission_mode?: unknown
  tool_name?: unknown
  tool_input?: unknown
  prompt?: unknown
}

function shouldBridgeClaudePermission(payload: ClaudePermissionPayload): boolean {
  if (payload.hook_event_name !== "PreToolUse" && payload.hook_event_name !== "PermissionRequest") return false
  const toolName = typeof payload.tool_name === "string" ? payload.tool_name : ""
  return new Set(["Bash", "Write", "Edit", "MultiEdit", "NotebookEdit"]).has(toolName)
}

function formatClaudePermissionMessage(payload: ClaudePermissionPayload): string {
  const toolName = typeof payload.tool_name === "string" ? payload.tool_name : "tool"
  const details = formatClaudeToolInput(payload.tool_input)
  const permissionMode = typeof payload.permission_mode === "string" ? payload.permission_mode : null
  return [
    `Claude Code wants to run ${toolName}.`,
    ...(permissionMode ? [`Permission mode: ${permissionMode}.`] : []),
    ...(details ? ["", details] : []),
  ].join("\n")
}

function formatClaudeToolInput(input: unknown): string {
  if (!input || typeof input !== "object") return ""
  const record = input as Record<string, unknown>
  if (typeof record.command === "string") return ["Command:", "", record.command].join("\n")
  if (typeof record.file_path === "string") {
    const pieces = [`File: ${record.file_path}`]
    if (typeof record.old_string === "string") pieces.push("", "Old:", record.old_string)
    if (typeof record.new_string === "string") pieces.push("", "New:", record.new_string)
    if (typeof record.content === "string") pieces.push("", "Content:", record.content)
    return pieces.join("\n")
  }
  try {
    return JSON.stringify(input, null, 2)
  } catch {
    return String(input)
  }
}

async function readJsonRequest(request: IncomingMessage): Promise<ClaudePermissionPayload> {
  const chunks: Buffer[] = []
  for await (const chunk of request) chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk))
  const raw = Buffer.concat(chunks).toString("utf8")
  return raw.trim() ? JSON.parse(raw) as ClaudePermissionPayload : {}
}

function writeJsonResponse(response: ServerResponse, statusCode: number, body: Record<string, unknown>) {
  response.writeHead(statusCode, { "content-type": "application/json" })
  response.end(JSON.stringify(body))
}

function expectVariant<T>(response: Record<string, unknown>, variant: string): T {
  if (!(variant in response)) {
    throw new Error(`expected ${variant} response, received ${JSON.stringify(response)}`)
  }
  return response[variant] as T
}
