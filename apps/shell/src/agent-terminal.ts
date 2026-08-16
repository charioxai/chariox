import { createInterface } from "node:readline"

import { AgentTerminal, redactSensitiveText, type AgentTerminalClient, type AgentTerminalContext } from "@chariox/kernel-client/agent-terminal"
import { LocalIpcClient } from "@chariox/kernel-client/ipc"

type JsonRpcRequest = {
  jsonrpc?: string
  id?: string | number | null
  method?: string
  params?: Record<string, unknown>
}

type AgentTerminalRequestOptions = {
  signal?: AbortSignal | undefined
  cancel?: ((requestId: string | number) => void) | undefined
}

const tools = [
  {
    name: "chariox_search",
    description: "Search Chariox terminal operations. Results are bounded; use chariox_describe before executing unfamiliar operations.",
    inputSchema: {
      type: "object",
      properties: { query: { type: "string" }, limit: { type: "integer", minimum: 1, maximum: 50 } },
      additionalProperties: false,
    },
  },
  {
    name: "chariox_describe",
    description: "Describe one Chariox terminal operation by its stable catalog id.",
    inputSchema: {
      type: "object",
      required: ["operation_id"],
      properties: { operation_id: { type: "string" }, command_id: { type: "string" } },
      additionalProperties: false,
    },
  },
  {
    name: "chariox_execute",
    description: "Execute a Chariox terminal command with explicit workspace/session/agent context. Human focus is never changed.",
    inputSchema: {
      type: "object",
      required: ["context"],
      properties: {
        command: { type: "string" },
        operation_id: { type: "string" },
        registry_revision: { type: "string" },
        input: {},
        context: {
          type: "object",
          required: ["workspace", "worktree"],
          properties: {
            workspace: { type: "string" },
            worktree: { type: "string" },
            workspace_id: { type: ["string", "null"] },
            worktree_id: { type: ["string", "null"] },
            session_id: { type: ["string", "null"] },
            attachment_id: { type: ["string", "null"] },
            agent_id: { type: ["string", "null"] },
            workflow_id: { type: ["string", "null"] },
            provider: { type: "string" },
            model: { type: "string" },
            effort: { type: "string" },
            variables: { type: "object", additionalProperties: { type: "string" } },
            targets: { type: "object", additionalProperties: { type: "string" } },
          },
          additionalProperties: false,
        },
      },
      additionalProperties: false,
    },
  },
  {
    name: "chariox_wait",
    description: "Wait for the explicitly targeted agent in a Chariox session and return the shared session snapshot.",
    inputSchema: {
      type: "object",
      required: ["context"],
      properties: {
        timeout_ms: { type: "integer", minimum: 0, maximum: 120000 },
        context: { type: "object", required: ["workspace", "worktree", "session_id", "attachment_id", "agent_id"], additionalProperties: true },
      },
      additionalProperties: false,
    },
  },
  {
    name: "chariox_status",
    description: "Return connection, registry, and explicitly targeted session state.",
    inputSchema: {
      type: "object",
      required: ["context"],
      properties: {
        context: { type: "object", required: ["workspace", "worktree"], additionalProperties: true },
      },
      additionalProperties: false,
    },
  },
]

const SERVER_INSTRUCTIONS = "Use chariox_search with a bounded query, then chariox_describe before chariox_execute. Every execute and wait call must include explicit workspace, worktree, and operation targets such as session_id and agent_id. Human focus is never changed; use chariox_status to inspect shared state and chariox_wait for the explicitly targeted agent."

export type AgentTerminalServerOptions = {
  endpoint: string
  clientId?: string | undefined
  relayAuthToken?: string | undefined
  targetDaemonId?: string | undefined
  targetDaemonAlias?: string | undefined
  homeKernelEndpoint?: string | undefined
  targetKernelRef?: string | undefined
  targetMachineRef?: string | undefined
  targetSessionId?: string | undefined
  input?: NodeJS.ReadableStream | undefined
  output?: NodeJS.WritableStream | undefined
  client?: AgentTerminalServerClient | undefined
  bootstrapClient?: AgentTerminalServerClient | undefined
  eventClientFactory?: (() => AgentTerminalClient) | undefined
}

type AgentTerminalServerClient = {
  send: (request: Record<string, unknown>) => Promise<Record<string, unknown>>
  close: () => Promise<void>
}

type AgentTerminalConnection = {
  endpoint: string
  relayAuthToken?: string | undefined
  relayAuthTokenProvider?: (() => Promise<string>) | undefined
  targetDaemonId?: string | undefined
  targetDaemonAlias?: string | undefined
}

export async function resolveAgentTerminalConnection(options: AgentTerminalServerOptions): Promise<AgentTerminalConnection> {
  const targetKernelRef = options.targetKernelRef?.trim()
  const homeKernelEndpoint = options.homeKernelEndpoint?.trim()
  if (!targetKernelRef && !homeKernelEndpoint) {
    return {
      endpoint: options.endpoint,
      relayAuthToken: options.relayAuthToken,
      targetDaemonId: options.targetDaemonId,
      targetDaemonAlias: options.targetDaemonAlias,
    }
  }
  if (!targetKernelRef || !homeKernelEndpoint) {
    throw new Error("agent terminal remote bootstrap requires both homeKernelEndpoint and targetKernelRef")
  }
  if (options.client) {
    throw new Error("agent terminal remote bootstrap cannot be combined with an injected client")
  }
  const injectedBootstrapClient = options.bootstrapClient
  const resolve = async (): Promise<AgentTerminalConnection> => {
    const bootstrapClient = injectedBootstrapClient ?? new LocalIpcClient(homeKernelEndpoint)
    try {
      const response = await bootstrapClient.send({
        ResolveKernelClientConnection: {
          kernel_ref: targetKernelRef,
          machine_ref: options.targetMachineRef?.trim() || null,
          client_id: options.clientId?.trim() || null,
          session_id: options.targetSessionId?.trim() || null,
        },
      })
      const connection = (response.KernelClientConnectionResolved as { connection?: Record<string, unknown> } | undefined)?.connection
      const relayUrl = typeof connection?.relay_url === "string" ? connection.relay_url.trim() : ""
      const relayToken = typeof connection?.relay_token === "string" ? connection.relay_token.trim() : ""
      if (!relayUrl || !relayToken) throw new Error("home kernel returned an invalid remote agent-terminal connection")
      const resolvedConnection = connection
      if (!resolvedConnection) throw new Error("home kernel returned an invalid remote agent-terminal connection")
      return {
        endpoint: relayUrl,
        relayAuthToken: relayToken,
        targetDaemonId: typeof resolvedConnection.target_daemon_id === "string" ? resolvedConnection.target_daemon_id : undefined,
        targetDaemonAlias: typeof resolvedConnection.target_daemon_alias === "string" ? resolvedConnection.target_daemon_alias : undefined,
      }
    } finally {
      if (!injectedBootstrapClient) await bootstrapClient.close().catch(() => {})
    }
  }
  const resolved = await resolve()
  return {
    ...resolved,
    relayAuthTokenProvider: async () => (await resolve()).relayAuthToken!,
  }
}

function createAgentTerminalClientFactory(connection: AgentTerminalConnection): () => LocalIpcClient {
  // A remote bootstrap token is one-shot across the control and event clients.
  // Share consumption at the factory boundary so a lazily-created event client
  // cannot replay the bootstrap token after the control client has used it.
  let bootstrapToken = connection.relayAuthToken
  let refreshPromise: Promise<string> | null = null
  const relayAuthTokenProvider = connection.relayAuthTokenProvider
    ? async (): Promise<string> => {
      if (bootstrapToken) {
        const token = bootstrapToken
        bootstrapToken = undefined
        return token
      }
      if (!refreshPromise) {
        refreshPromise = connection.relayAuthTokenProvider!().finally(() => {
          refreshPromise = null
        })
      }
      return refreshPromise
    }
    : undefined
  return () => new LocalIpcClient(connection.endpoint, {
    relayAuthToken: relayAuthTokenProvider ? undefined : connection.relayAuthToken,
    relayAuthTokenProvider,
    targetDaemonId: connection.targetDaemonId,
    targetDaemonAlias: connection.targetDaemonAlias,
  })
}

export async function runAgentTerminalServer(options: AgentTerminalServerOptions): Promise<void> {
  const input = options.input ?? process.stdin
  const output = options.output ?? process.stdout
  const connection = await resolveAgentTerminalConnection(options)
  const createClient = createAgentTerminalClientFactory(connection)
  const client = options.client ?? createClient()
  const terminal = new AgentTerminal(
    client,
    options.clientId,
    options.eventClientFactory ?? (options.client ? undefined : createClient),
  )
  const readline = createInterface({ input })
  const pending = new Set<Promise<void>>()
  const inFlight = new Map<string | number, AbortController>()
  let writeChain = Promise.resolve()
  try {
    for await (const line of readline) {
      if (!line.trim()) continue
      let request: JsonRpcRequest
      try {
        request = JSON.parse(line) as JsonRpcRequest
      } catch (error) {
        writeResponse(output, { jsonrpc: "2.0", id: null, error: { code: -32700, message: String(error) } })
        continue
      }
      const controller = request.id !== undefined && request.id !== null ? new AbortController() : undefined
      if (controller && (typeof request.id === "string" || typeof request.id === "number")) {
        inFlight.set(request.id, controller)
      }
      const task = (async () => {
        const response = await handleRequest(terminal, request, {
          signal: controller?.signal,
          cancel: (requestId) => inFlight.get(requestId)?.abort(),
        })
        if (response !== null) {
          const nextWrite = writeChain.then(() => writeResponse(output, response))
          writeChain = nextWrite
          await nextWrite
        }
      })()
      pending.add(task)
      void task.then(
        () => {
          pending.delete(task)
          if (controller && (typeof request.id === "string" || typeof request.id === "number") && inFlight.get(request.id) === controller) inFlight.delete(request.id)
        },
        () => {
          pending.delete(task)
          if (controller && (typeof request.id === "string" || typeof request.id === "number") && inFlight.get(request.id) === controller) inFlight.delete(request.id)
        },
      )
    }
    abortInFlightRequests(inFlight)
    await Promise.all([...pending])
    await writeChain
  } finally {
    readline.close()
    await terminal.close()
    await client.close()
  }
}

/**
 * A deliberately small JSONL adapter for hosts that do not speak MCP. Each
 * line is one request and every response is one line; there is no connection
 * or focus state hidden in the adapter.
 */
export async function runAgentTerminalJsonl(options: AgentTerminalServerOptions): Promise<void> {
  const input = options.input ?? process.stdin
  const output = options.output ?? process.stdout
  const connection = await resolveAgentTerminalConnection(options)
  const createClient = createAgentTerminalClientFactory(connection)
  const client = options.client ?? createClient()
  const terminal = new AgentTerminal(
    client,
    options.clientId,
    options.eventClientFactory ?? (options.client ? undefined : createClient),
  )
  const readline = createInterface({ input })
  const pending = new Set<Promise<void>>()
  const inFlight = new Map<string | number, AbortController>()
  let writeChain = Promise.resolve()
  try {
    for await (const line of readline) {
      if (!line.trim()) continue
      let request: Record<string, unknown>
      try {
        request = JSON.parse(line) as Record<string, unknown>
      } catch (error) {
        const nextWrite = writeChain.then(() => writeResponse(output, { id: null, ok: false, error: String(error) }))
        writeChain = nextWrite
        continue
      }
      const requestId = typeof request.id === "string" || typeof request.id === "number" ? request.id : undefined
      const controller = requestId === undefined ? undefined : new AbortController()
      if (controller && requestId !== undefined) inFlight.set(requestId, controller)
      const task = (async () => {
        let response: Record<string, unknown>
        try {
          const result = await executeJsonlRequest(terminal, request, controller?.signal)
          response = { id: request.id ?? null, ok: true, result }
        } catch (error) {
        response = { id: request.id ?? null, ok: false, error: safeErrorMessage(error) }
        }
        const nextWrite = writeChain.then(() => writeResponse(output, response))
        writeChain = nextWrite
        await nextWrite
      })()
      pending.add(task)
      void task.finally(() => {
        pending.delete(task)
        if (controller && requestId !== undefined && inFlight.get(requestId) === controller) inFlight.delete(requestId)
      })
    }
    abortInFlightRequests(inFlight)
    await Promise.all([...pending])
    await writeChain
  } finally {
    readline.close()
    await terminal.close()
    await client.close()
  }
}

async function executeJsonlRequest(terminal: AgentTerminalApi, request: Record<string, unknown>, signal?: AbortSignal): Promise<unknown> {
  const op = request.op
  if (op === "status") return terminal.status(asContext(request.context, "context"))
  if (op === "search") return terminal.search({ query: asOptionalString(request.query), limit: asOptionalBoundedInteger(request.limit, "limit", 1, 50) })
  if (op === "describe") return terminal.describe(asRequiredString(request.operation_id ?? request.command_id, "operation_id"))
    if (op === "execute") {
      const context = asContext(request.context, "context")
      if (Boolean(request.operation_id) === Boolean(request.command)) throw new Error("execute requires exactly one of operation_id or command")
      const registryRevision = asOptionalString(request.registry_revision)
      return request.operation_id
      ? terminal.executeOperation(asRequiredString(request.operation_id, "operation_id"), request.input, context, { signal, registry_revision: registryRevision })
      : terminal.execute(asRequiredString(request.command, "command"), context, { signal, registry_revision: registryRevision })
  }
  if (op === "wait") return terminal.wait(asContext(request.context, "context", true), asOptionalBoundedInteger(request.timeout_ms, "timeout_ms", 0, 120000), signal)
  throw new Error(`unknown agent terminal operation: ${String(op)}`)
}

export function abortInFlightRequests(inFlight: Map<string | number, AbortController>): void {
  for (const controller of inFlight.values()) controller.abort()
}

export type AgentTerminalApi = Pick<AgentTerminal, "search" | "describe" | "execute" | "executeOperation" | "wait" | "status">

export async function handleRequest(terminal: AgentTerminalApi, request: JsonRpcRequest, options: AgentTerminalRequestOptions = {}): Promise<Record<string, unknown> | null> {
  if (request.method === "notifications/cancelled") {
    const requestId = request.params?.requestId
    if (typeof requestId === "string" || typeof requestId === "number") options.cancel?.(requestId)
    return null
  }
  if (!Object.prototype.hasOwnProperty.call(request, "id")) return null
  const id = request.id ?? null
  try {
    switch (request.method) {
      case "initialize":
        return { jsonrpc: "2.0", id, result: { protocolVersion: "2024-11-05", capabilities: { tools: {} }, serverInfo: { name: "chariox-agent-terminal", version: "0.1.0" }, instructions: SERVER_INSTRUCTIONS } }
      case "notifications/initialized":
        return null
      case "tools/list":
        return { jsonrpc: "2.0", id, result: { tools } }
      case "tools/call":
        try {
          return { jsonrpc: "2.0", id, result: await callTool(terminal, request.params ?? {}, options.signal) }
        } catch (error) {
          return {
            jsonrpc: "2.0",
            id,
            result: jsonToolResult({ error: safeErrorMessage(error) }, true),
          }
        }
      case "ping":
        return { jsonrpc: "2.0", id, result: {} }
      default:
        return { jsonrpc: "2.0", id, error: { code: -32601, message: `method not found: ${request.method ?? ""}` } }
    }
  } catch (error) {
    return { jsonrpc: "2.0", id, error: { code: -32000, message: safeErrorMessage(error) } }
  }
}

async function callTool(terminal: AgentTerminalApi, params: Record<string, unknown>, signal?: AbortSignal): Promise<Record<string, unknown>> {
  const name = params.name
  const argumentsValue = asArguments(params.arguments)
  if (name === "chariox_search" || name === "chariox.search") {
    assertKnownKeys(argumentsValue, ["query", "limit"], "chariox_search")
    const result = await terminal.search({ query: asOptionalString(argumentsValue.query), limit: asOptionalBoundedInteger(argumentsValue.limit, "limit", 1, 50) })
    return jsonToolResult(result)
  }
  if (name === "chariox_describe" || name === "chariox.describe") {
    assertKnownKeys(argumentsValue, ["operation_id", "command_id"], "chariox_describe")
    const result = await terminal.describe(asRequiredString(argumentsValue.operation_id ?? argumentsValue.command_id, "operation_id"))
    return jsonToolResult(result)
  }
  if (name === "chariox_execute" || name === "chariox.execute") {
    assertKnownKeys(argumentsValue, ["command", "operation_id", "registry_revision", "input", "context"], "chariox_execute")
    const context = asContext(argumentsValue.context, "context")
    if (Boolean(argumentsValue.operation_id) === Boolean(argumentsValue.command)) throw new Error("chariox_execute requires exactly one of operation_id or command")
    const registryRevision = asOptionalString(argumentsValue.registry_revision)
    const result = argumentsValue.operation_id
      ? await terminal.executeOperation(
        asRequiredString(argumentsValue.operation_id, "operation_id"),
        argumentsValue.input,
        context,
        { signal, registry_revision: registryRevision },
      )
      : await terminal.execute(asRequiredString(argumentsValue.command, "command"), context, { signal, registry_revision: registryRevision })
    return jsonToolResult(result, !result.ok)
  }
  if (name === "chariox_wait" || name === "chariox.wait") {
    assertKnownKeys(argumentsValue, ["timeout_ms", "context"], "chariox_wait")
    const result = await terminal.wait(
      asContext(argumentsValue.context, "context", true),
      asOptionalBoundedInteger(argumentsValue.timeout_ms, "timeout_ms", 0, 120000),
      signal,
    )
    return jsonToolResult(result, result.timed_out)
  }
  if (name === "chariox_status") {
    assertKnownKeys(argumentsValue, ["context"], "chariox_status")
    const result = await terminal.status(asContext(argumentsValue.context, "context"))
    return jsonToolResult(result)
  }
  throw new Error(`unknown tool: ${String(name)}`)
}

function jsonToolResult(value: unknown, isError = false): Record<string, unknown> {
  return { content: [{ type: "text", text: JSON.stringify(value) }], structuredContent: value, isError }
}

function writeResponse(output: NodeJS.WritableStream, response: Record<string, unknown>): void {
  output.write(`${JSON.stringify(response)}\n`)
}

function safeErrorMessage(error: unknown): string {
  return redactSensitiveText(error instanceof Error ? error.message : String(error))
}

function asRequiredString(value: unknown, name: string): string {
  if (typeof value !== "string" || !value.trim()) throw new Error(`${name} is required`)
  return value
}

function asOptionalString(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined
}

function asOptionalNumber(value: unknown): number | undefined {
  return typeof value === "number" ? value : undefined
}

function asOptionalBoundedInteger(value: unknown, name: string, minimum: number, maximum: number): number | undefined {
  if (value === undefined) return undefined
  if (typeof value !== "number" || !Number.isInteger(value) || value < minimum || value > maximum) {
    throw new Error(`${name} must be an integer between ${minimum} and ${maximum}`)
  }
  return value
}

function asArguments(value: unknown): Record<string, unknown> {
  if (value === undefined) return {}
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("tool arguments must be an object")
  return value as Record<string, unknown>
}

function assertKnownKeys(value: Record<string, unknown>, allowed: string[], name: string): void {
  const unknown = Object.keys(value).find((key) => !allowed.includes(key))
  if (unknown) throw new Error(`${name} does not accept argument ${unknown}`)
}

function asContext(value: unknown, name: string, requireAgent = false): AgentTerminalContext {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${name} must be an object`)
  const context = value as AgentTerminalContext
  if (typeof context.workspace !== "string" || !context.workspace.trim() || typeof context.worktree !== "string" || !context.worktree.trim()) {
    throw new Error(`${name} requires workspace and worktree`)
  }
  if (requireAgent && (typeof context.session_id !== "string" || !context.session_id.trim() || typeof context.agent_id !== "string" || !context.agent_id.trim())) {
    throw new Error(`${name} requires session_id and agent_id`)
  }
  return context
}
