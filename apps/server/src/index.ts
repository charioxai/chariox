import { spawn } from "node:child_process"
import { readFile } from "node:fs/promises"
import process from "node:process"

import Fastify, { type FastifyRequest } from "fastify"

import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import {
  getWorkflowRunRequest,
  invokeWorkflowEndpointRequest,
} from "@arroba/kernel-client/ipc-requests"

import { createProcessLogger } from "./logging.js"

type ParserKind =
  | "json"
  | "query_params"
  | "headers"
  | "regex"
  | "path_template"
  | "webhook"
  | "custom_command"

type AuthConfig =
  | { mode: "anonymous" }
  | { mode: "bearer"; token_env: string }
  | { mode: "api_key"; header: string; key_env: string }

type ParserConfig = {
  kind: ParserKind
  source?: "body" | "path" | "query" | "headers" | "request"
  pattern?: string
  template?: string
  command?: string
  args?: string[]
}

type InputSchema = {
  type?: "object"
  required?: string[]
  properties?: Record<string, { type?: "string" | "number" | "boolean" | "object" | "array" }>
}

export type WorkflowPublicationConfig = {
  publication_id: string
  session_id: string
  workflow_ref: string
  endpoint_ref: string
  kernel_endpoint?: string
  route?: string
  methods?: Array<"GET" | "POST">
  auth?: AuthConfig
  parser?: ParserConfig
  input_schema?: InputSchema
  mode?: "sync" | "async"
  sync_timeout_ms?: number
  poll_ms?: number
}

type NormalizedInvocation = {
  publication_id: string
  request_id: string
  caller: Record<string, unknown>
  input: unknown
  mode: "sync" | "async"
}

type WorkflowRun = {
  id: string
  status: string
  workflow_id?: string
  endpoint_id?: string
  final_output?: { message: string; artifacts?: unknown[] } | null
}

type WorkflowInvocationResult = {
  accepted: boolean
  queued?: boolean
  workflow_run?: WorkflowRun
  response?: unknown
}

type GatewayDeps = {
  invokeWorkflow?: (invocation: NormalizedInvocation) => Promise<WorkflowInvocationResult>
}

export const buildServer = (config?: WorkflowPublicationConfig, deps: GatewayDeps = {}) => {
  const processLogger = createProcessLogger("workflow-gateway")
  const logger = processLogger.child("gateway.http")
  const app = Fastify({ logger: false })
  const publication = config ?? defaultPublicationConfig()

  app.get("/health", async () => {
    logger.debug("handled health request")
    return { status: "ok" }
  })

  const methods = publication.methods?.length ? publication.methods : ["GET", "POST"]
  for (const method of methods) {
    app.route({
      method,
      url: publication.route ?? "/*",
      handler: async (request, reply) => {
        const auth = authenticateRequest(request, publication.auth ?? { mode: "anonymous" })
        if (!auth.ok) {
          reply.code(401).headers({ "content-type": "application/json" })
          return { error: auth.message }
        }

        const parsed = await parseRequest(request, publication.parser ?? { kind: "json" })
        validateInput(parsed, publication.input_schema)
        const invocation: NormalizedInvocation = {
          publication_id: publication.publication_id,
          request_id: `req_${Date.now()}_${Math.random().toString(16).slice(2)}`,
          caller: auth.caller,
          input: parsed,
          mode: publication.mode ?? "sync",
        }
        const result = deps.invokeWorkflow
          ? await deps.invokeWorkflow(invocation)
          : await invokeKernelWorkflow(publication, invocation)
        return forwardWorkflowResult(reply, result)
      },
    })
  }

  app.addHook("onClose", async () => {
    logger.info("gateway closed")
  })

  return { app, logger }
}

async function invokeKernelWorkflow(
  publication: WorkflowPublicationConfig,
  invocation: NormalizedInvocation,
): Promise<WorkflowInvocationResult> {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const prompt = JSON.stringify(invocation, null, 2)
    const response = await client.send<Record<string, unknown>>(invokeWorkflowEndpointRequest(
      publication.session_id,
      publication.workflow_ref,
      publication.endpoint_ref,
      prompt,
    ))
    if ("WorkflowRunQueued" in response) {
      return { accepted: true, queued: true, response: response.WorkflowRunQueued }
    }
    const invoked = response.WorkflowRunInvoked as { workflow_run: WorkflowRun } | undefined
    if (!invoked?.workflow_run) {
      throw new Error(`unexpected workflow invoke response: ${JSON.stringify(response)}`)
    }
    if ((publication.mode ?? "sync") === "async") {
      return { accepted: true, workflow_run: invoked.workflow_run }
    }
    const workflowRun = await waitForWorkflowRun(client, publication, invoked.workflow_run.id)
    return { accepted: true, workflow_run: workflowRun }
  } finally {
    await client.close().catch(() => {})
  }
}

async function waitForWorkflowRun(
  client: LocalIpcClient,
  publication: WorkflowPublicationConfig,
  workflowRunId: string,
) {
  const timeoutMs = publication.sync_timeout_ms ?? 30_000
  const pollMs = publication.poll_ms ?? 500
  const deadline = Date.now() + timeoutMs
  let latest: WorkflowRun | null = null
  while (Date.now() < deadline) {
    const response = await client.send<Record<string, unknown>>(
      getWorkflowRunRequest(publication.session_id, workflowRunId),
    )
    latest = (response.WorkflowRun as { workflow_run: WorkflowRun } | undefined)?.workflow_run ?? null
    if (latest && isTerminalWorkflowRunStatus(latest.status)) return latest
    await sleep(pollMs)
  }
  return latest ?? { id: workflowRunId, status: "unknown" }
}

function forwardWorkflowResult(reply: { code: (code: number) => unknown; headers: (headers: Record<string, string>) => unknown }, result: WorkflowInvocationResult) {
  const workflowRun = result.workflow_run
  const finalMessage = workflowRun?.final_output?.message
  const transportResponse = parseTransportResponse(finalMessage)
  if (transportResponse) {
    reply.code(transportResponse.status)
    reply.headers(transportResponse.headers)
    return transportResponse.body ?? ""
  }
  if (result.queued) {
    reply.code(202)
    return { accepted: true, queued: true, result: result.response }
  }
  if (workflowRun && !isTerminalWorkflowRunStatus(workflowRun.status)) {
    reply.code(202)
    return { accepted: true, workflow_run: workflowRun }
  }
  reply.code(200)
  return {
    accepted: result.accepted,
    workflow_run: workflowRun ?? null,
    final_output: workflowRun?.final_output ?? null,
  }
}

function parseTransportResponse(message: string | undefined | null): { status: number; headers: Record<string, string>; body: unknown } | null {
  if (!message) return null
  try {
    const parsed = JSON.parse(message) as {
      kind?: string
      status?: number
      headers?: Record<string, string>
      body?: unknown
    }
    if (parsed.kind !== "http_response") return null
    return {
      status: parsed.status ?? 200,
      headers: parsed.headers ?? {},
      body: parsed.body ?? "",
    }
  } catch {
    return null
  }
}

function authenticateRequest(request: FastifyRequest, config: AuthConfig): { ok: true; caller: Record<string, unknown> } | { ok: false; message: string } {
  if (config.mode === "anonymous") {
    return { ok: true, caller: { type: "anonymous" } }
  }
  if (config.mode === "bearer") {
    const expected = readRequiredEnv(config.token_env)
    const actual = request.headers.authorization
    if (actual !== `Bearer ${expected}`) {
      return { ok: false, message: "missing or invalid bearer token" }
    }
    return { ok: true, caller: { type: "bearer", token_env: config.token_env } }
  }
  const expected = readRequiredEnv(config.key_env)
  const actual = headerValue(request, config.header)
  if (actual !== expected) {
    return { ok: false, message: "missing or invalid API key" }
  }
  return { ok: true, caller: { type: "api_key", key_env: config.key_env } }
}

async function parseRequest(request: FastifyRequest, config: ParserConfig): Promise<unknown> {
  switch (config.kind) {
    case "json":
      return request.body ?? {}
    case "query_params":
      return request.query ?? {}
    case "headers":
      return request.headers
    case "webhook":
      return { headers: request.headers, body: request.body ?? {}, query: request.query ?? {} }
    case "regex":
      return parseRegex(sourceValue(request, config.source ?? "path"), config)
    case "path_template":
      return parsePathTemplate(String(request.url.split("?")[0] ?? "/"), config)
    case "custom_command":
      return await parseCustomCommand(request, config)
  }
}

function parseRegex(source: string, config: ParserConfig) {
  if (!config.pattern) throw new Error("regex parser requires pattern")
  const match = new RegExp(config.pattern).exec(source)
  if (!match) throw new Error("request did not match regex parser")
  return { ...(match.groups ?? {}) }
}

function parsePathTemplate(pathname: string, config: ParserConfig) {
  if (!config.template) throw new Error("path_template parser requires template")
  const templateParts = config.template.split("/").filter(Boolean)
  const pathParts = pathname.split("/").filter(Boolean)
  if (templateParts.length !== pathParts.length) {
    throw new Error("request path did not match template")
  }
  const output: Record<string, string> = {}
  for (let index = 0; index < templateParts.length; index += 1) {
    const template = templateParts[index] ?? ""
    const value = decodeURIComponent(pathParts[index] ?? "")
    if (template.startsWith(":")) output[template.slice(1)] = value
    else if (template !== value) throw new Error("request path did not match template")
  }
  return output
}

async function parseCustomCommand(request: FastifyRequest, config: ParserConfig) {
  if (!config.command) throw new Error("custom_command parser requires command")
  const envelope = JSON.stringify({
    method: request.method,
    url: request.url,
    headers: request.headers,
    query: request.query ?? {},
    body: request.body ?? {},
  })
  const output = await runCommand(config.command, config.args ?? [], envelope)
  return JSON.parse(output || "{}")
}

function validateInput(value: unknown, schema: InputSchema | undefined) {
  if (!schema) return
  if (schema.type === "object" && (!value || typeof value !== "object" || Array.isArray(value))) {
    throw new Error("input schema expected object")
  }
  const object = value as Record<string, unknown>
  for (const field of schema.required ?? []) {
    if (object[field] === undefined || object[field] === null) {
      throw new Error(`input schema missing required field ${field}`)
    }
  }
  for (const [field, spec] of Object.entries(schema.properties ?? {})) {
    if (object[field] === undefined || !spec.type) continue
    if (!matchesJsonType(object[field], spec.type)) {
      throw new Error(`input schema field ${field} expected ${spec.type}`)
    }
  }
}

function matchesJsonType(value: unknown, type: string) {
  if (type === "array") return Array.isArray(value)
  if (type === "object") return value != null && typeof value === "object" && !Array.isArray(value)
  return typeof value === type
}

function sourceValue(request: FastifyRequest, source: ParserConfig["source"]) {
  if (source === "body") return typeof request.body === "string" ? request.body : JSON.stringify(request.body ?? {})
  if (source === "query") return JSON.stringify(request.query ?? {})
  if (source === "headers") return JSON.stringify(request.headers)
  if (source === "request") return JSON.stringify({ method: request.method, url: request.url, body: request.body ?? {}, query: request.query ?? {} })
  return String(request.url.split("?")[0] ?? "/")
}

function headerValue(request: FastifyRequest, header: string) {
  const value = request.headers[header.toLowerCase()]
  return Array.isArray(value) ? value[0] : value
}

function readRequiredEnv(name: string) {
  const value = process.env[name]
  if (!value) throw new Error(`required env ${name} is not set`)
  return value
}

function defaultKernelEndpoint() {
  return process.env.ARROBA_KERNEL_URL ?? `ws://${process.env.ARROBA_KERNEL_HOST ?? "127.0.0.1"}:${process.env.ARROBA_KERNEL_PORT ?? "43118"}`
}

function defaultPublicationConfig(): WorkflowPublicationConfig {
  const config: WorkflowPublicationConfig = {
    publication_id: process.env.ARROBA_PUBLICATION_ID ?? "default",
    session_id: requiredProcessEnv("ARROBA_PUBLICATION_SESSION_ID"),
    workflow_ref: requiredProcessEnv("ARROBA_PUBLICATION_WORKFLOW"),
    endpoint_ref: requiredProcessEnv("ARROBA_PUBLICATION_ENDPOINT"),
    route: process.env.ARROBA_PUBLICATION_ROUTE ?? "/*",
    mode: process.env.ARROBA_PUBLICATION_MODE === "async" ? "async" : "sync",
  }
  if (process.env.ARROBA_KERNEL_URL) config.kernel_endpoint = process.env.ARROBA_KERNEL_URL
  return config
}

function requiredProcessEnv(name: string) {
  const value = process.env[name]
  if (!value) throw new Error(`required env ${name} is not set`)
  return value
}

async function runCommand(command: string, args: string[], input: string): Promise<string> {
  return await new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["pipe", "pipe", "pipe"] })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
    child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
    child.on("error", reject)
    child.on("close", (code) => {
      if (code === 0) resolve(stdout)
      else reject(new Error(`custom parser exited ${code}: ${stderr}`))
    })
    child.stdin.end(input)
  })
}

function isTerminalWorkflowRunStatus(status: string) {
  return ["completed", "failed", "stopped"].includes(status.toLowerCase())
}

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

export async function loadPublicationConfig(path: string) {
  return JSON.parse(await readFile(path, "utf8")) as WorkflowPublicationConfig
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const config = process.env.ARROBA_PUBLICATION_CONFIG
    ? await loadPublicationConfig(process.env.ARROBA_PUBLICATION_CONFIG)
    : undefined
  const { app, logger } = buildServer(config)
  const host = process.env.HOST ?? "0.0.0.0"
  const port = Number(process.env.PORT ?? 3000)
  logger.info("starting workflow gateway", { host, port })

  app
    .listen({ host, port })
    .then((address) => {
      logger.info("workflow gateway listening", { host, port, address })
    })
    .catch((error) => {
      logger.error("workflow gateway failed to start", { error: error.message, host, port })
      process.exit(1)
    })
}
