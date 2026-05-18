import { spawn } from "node:child_process"
import { readFileSync } from "node:fs"
import { readFile } from "node:fs/promises"
import type { IncomingMessage } from "node:http"
import type { Duplex } from "node:stream"
import process from "node:process"

import Fastify from "fastify"
import { WebSocket as WsSocket, WebSocketServer, type RawData } from "ws"

import { LocalIpcClient } from "@arroba/kernel-client/ipc"
import type {
  WorkflowPublicationDefinition,
} from "@arroba/kernel-client/kernel-types"
import {
  getWorkflowPublicationRequest,
  getWorkflowRunRequest,
} from "@arroba/kernel-client/ipc-requests"

import {
  defaultKernelEndpoint,
  invokeKernelWorkflow,
  redeemPublicationPairCode,
} from "./kernel-publication-client.js"
import { createProcessLogger } from "./logging.js"
import {
  authenticateRequest,
  handleConnectorHandshake,
  isPairedSenderAuthEnabled,
  objectBody,
} from "./publication-auth.js"
import type {
  AuthConfig,
  GatewayDeps,
  GatewayRequest,
  InputSchema,
  KernelLookupClient,
  NormalizedInvocation,
  ParserConfig,
  PublicationInvocationOptions,
  TlsConfig,
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"
import { isTerminalWorkflowRunStatus } from "./workflow-run-status.js"

export type {
  PublicationInvocationOptions,
  WorkflowPublicationConfig,
} from "./publication-types.js"

export const buildServer = (config?: WorkflowPublicationConfig, deps: GatewayDeps = {}) => {
  const processLogger = createProcessLogger("workflow-gateway")
  const logger = processLogger.child("gateway.http")
  const publication = config ?? defaultPublicationConfig()
  const httpsOptions = resolveHttpsOptions(publication.tls)
  const app = Fastify({ logger: false, ...(httpsOptions ? { https: httpsOptions } : {}) } as never)
  const webSocketServer = installPublicationWebSocket(app, publication, deps)
  installRawBodyParsers(app)

  app.get("/health", async () => {
    logger.debug("handled health request")
    return { status: "ok" }
  })

  if (isPairedSenderAuthEnabled(publication.auth)) {
    app.post("/.well-known/arroba/publication/pair", async (request, reply) => {
      const body = objectBody(request.body)
      const pairCode = String(body.pair_code ?? "")
      if (!pairCode) {
        reply.code(400)
        return { error: "missing pair_code" }
      }
      const senderCredential = deps.redeemPublicationPairCode
        ? await deps.redeemPublicationPairCode(publication, pairCode, optionalString(body.display_name))
        : await redeemPublicationPairCode(publication, pairCode, optionalString(body.display_name))
      return {
        sender: senderCredential.sender,
        credential: senderCredential.credential,
      }
    })
  } else {
    app.post("/.well-known/arroba/publication/pair", async (_request, reply) => {
      reply.code(404)
      return { error: "publication pairing is not enabled" }
    })
  }

  const methods = publication.methods?.length ? publication.methods : ["GET", "POST"]
  for (const method of methods) {
    app.route({
      method,
      url: publication.route ?? "/*",
      handler: async (request, reply) => {
        const handshake = handleConnectorHandshake(request as unknown as GatewayRequest, reply, publication)
        if (handshake.handled) return handshake.payload

        const auth = await authenticateRequest(
          request as unknown as GatewayRequest,
          publication,
          publication.auth ?? { mode: "anonymous" },
          deps,
        )
        if (!auth.ok) {
          reply.code(401).headers({ "content-type": "application/json" })
          return { error: auth.message }
        }

        const parsed = await parseAndValidateRequest(request as unknown as GatewayRequest, publication).catch((error) => {
          reply.code(400).headers({ "content-type": "application/json" })
          return { __arroba_parse_error: error instanceof Error ? error.message : String(error) }
        })
        if (isParseErrorPayload(parsed)) {
          return { error: parsed.__arroba_parse_error }
        }
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
    webSocketServer.close()
    logger.info("gateway closed")
  })

  return { app, logger }
}

function installPublicationWebSocket(
  app: ReturnType<typeof Fastify>,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
) {
  const webSocketServer = new WebSocketServer({ noServer: true })
  app.server.on("upgrade", (request: IncomingMessage, socket: Duplex, head: Buffer) => {
    const pathname = new URL(request.url ?? "/", "http://127.0.0.1").pathname
    if (pathname !== "/.well-known/arroba/publication/ws") return
    webSocketServer.handleUpgrade(request, socket, head, (webSocket) => {
      webSocketServer.emit("connection", webSocket, request)
    })
  })
  webSocketServer.on("connection", (webSocket, request) => {
    void handlePublicationWebSocket(webSocket, request, publication, deps)
  })
  return webSocketServer
}

async function handlePublicationWebSocket(
  webSocket: WsSocket,
  request: IncomingMessage,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
) {
  const gatewayRequest = gatewayRequestFromIncomingMessage(request)
  const auth = await authenticateRequest(gatewayRequest, publication, publication.auth ?? { mode: "anonymous" }, deps)
  if (!auth.ok) {
    sendWebSocketJson(webSocket, { type: "error", error: auth.message })
    webSocket.close(1008, auth.message)
    return
  }
  webSocket.on("message", (data) => {
    void handlePublicationWebSocketMessage(webSocket, data, publication, deps, auth.caller)
  })
  sendWebSocketJson(webSocket, { type: "ready", publication_id: publication.publication_id })
}

async function handlePublicationWebSocketMessage(
  webSocket: WsSocket,
  data: RawData,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
  caller: Record<string, unknown>,
) {
  try {
    const envelope = parseWebSocketEnvelope(data)
    if (envelope.type !== "invoke") {
      sendWebSocketJson(webSocket, { type: "error", error: "expected invoke message" })
      return
    }
    validateInput(envelope.input, publication.input_schema)
    const invocation: NormalizedInvocation = {
      publication_id: publication.publication_id,
      request_id: `ws_${Date.now()}_${Math.random().toString(16).slice(2)}`,
      caller,
      input: envelope.input,
      mode: publication.mode ?? "sync",
    }
    const result = deps.invokeWorkflow
      ? await deps.invokeWorkflow(invocation)
      : await invokeKernelWorkflow(publication, invocation)
    sendWebSocketJson(webSocket, { type: "accepted", ...result })
    if (!deps.invokeWorkflow && result.workflow_run?.id) {
      await streamWorkflowRun(webSocket, publication, result.workflow_run.id)
    } else if (result.workflow_run && isTerminalWorkflowRunStatus(result.workflow_run.status)) {
      sendWebSocketJson(webSocket, { type: "final", workflow_run: result.workflow_run })
    }
  } catch (error) {
    sendWebSocketJson(webSocket, { type: "error", error: error instanceof Error ? error.message : String(error) })
  }
}

async function streamWorkflowRun(webSocket: WsSocket, publication: WorkflowPublicationConfig, workflowRunId: string) {
  const client = new LocalIpcClient(publication.kernel_endpoint ?? defaultKernelEndpoint())
  try {
    const timeoutMs = publication.sync_timeout_ms ?? 30_000
    const pollMs = publication.poll_ms ?? 500
    const deadline = Date.now() + timeoutMs
    let lastStatus: string | null = null
    while (Date.now() < deadline && webSocket.readyState === WsSocket.OPEN) {
      const response = await client.send<Record<string, unknown>>(
        getWorkflowRunRequest(publication.session_id, workflowRunId),
      )
      const workflowRun = (response.WorkflowRun as { workflow_run: WorkflowRun } | undefined)?.workflow_run ?? null
      if (workflowRun && workflowRun.status !== lastStatus) {
        lastStatus = workflowRun.status
        sendWebSocketJson(webSocket, { type: "status", workflow_run: workflowRun })
      }
      if (workflowRun && isTerminalWorkflowRunStatus(workflowRun.status)) {
        sendWebSocketJson(webSocket, { type: "final", workflow_run: workflowRun })
        return
      }
      await sleep(pollMs)
    }
    sendWebSocketJson(webSocket, { type: "timeout", workflow_run_id: workflowRunId })
  } finally {
    await client.close().catch(() => {})
  }
}

function gatewayRequestFromIncomingMessage(request: IncomingMessage): GatewayRequest {
  return {
    method: request.method ?? "GET",
    url: request.url ?? "/",
    headers: request.headers as Record<string, string | string[] | undefined>,
    query: Object.fromEntries(new URL(request.url ?? "/", "http://127.0.0.1").searchParams),
    raw: request,
  }
}

function parseWebSocketEnvelope(data: RawData): { type: string; input: unknown } {
  const raw = Array.isArray(data) ? Buffer.concat(data).toString("utf8") : Buffer.from(data as Buffer).toString("utf8")
  const parsed = JSON.parse(raw) as { type?: unknown; input?: unknown; payload?: unknown }
  return {
    type: typeof parsed.type === "string" ? parsed.type : "",
    input: parsed.input ?? parsed.payload ?? {},
  }
}

function sendWebSocketJson(webSocket: WsSocket, payload: unknown) {
  if (webSocket.readyState === WsSocket.OPEN) {
    webSocket.send(JSON.stringify(payload))
  }
}

export async function invokePublicationInput(
  publication: WorkflowPublicationConfig,
  options: PublicationInvocationOptions,
): Promise<WorkflowInvocationResult> {
  validateInput(options.input, publication.input_schema)
  const invocation: NormalizedInvocation = {
    publication_id: publication.publication_id,
    request_id: `${options.requestIdPrefix ?? "ipc"}_${Date.now()}_${Math.random().toString(16).slice(2)}`,
    caller: options.caller ?? {
      type: "ipc",
      proof: { auth: "ipc", connector: "ipc" },
    },
    input: options.input,
    mode: options.mode ?? publication.mode ?? "sync",
  }
  return options.deps?.invokeWorkflow
    ? await options.deps.invokeWorkflow(invocation)
    : await invokeKernelWorkflow(publication, invocation)
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

function isParseErrorPayload(value: unknown): value is { __arroba_parse_error: string } {
  return !!value
    && typeof value === "object"
    && "__arroba_parse_error" in value
    && typeof (value as { __arroba_parse_error?: unknown }).__arroba_parse_error === "string"
}

function optionalString(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : null
}

function installRawBodyParsers(app: ReturnType<typeof Fastify>) {
  app.removeContentTypeParser("application/json")
  app.addContentTypeParser("application/json", { parseAs: "string" }, (request: { raw: { arrobaRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    try {
      done(null, body ? JSON.parse(body) : {})
    } catch (error) {
      done(error as Error)
    }
  })

  app.addContentTypeParser("application/x-www-form-urlencoded", { parseAs: "string" }, (request: { raw: { arrobaRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    done(null, Object.fromEntries(new URLSearchParams(body)))
  })

  app.addContentTypeParser("text/plain", { parseAs: "string" }, (request: { raw: { arrobaRawBody?: string } }, body: string, done: (error: Error | null, body?: unknown) => void) => {
    setRawRequestBody(request, body)
    done(null, body)
  })
}

function setRawRequestBody(request: { raw: { arrobaRawBody?: string } }, body: string) {
  request.raw.arrobaRawBody = body
}

async function parseRequest(request: GatewayRequest, config: ParserConfig): Promise<unknown> {
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

async function parseAndValidateRequest(
  request: GatewayRequest,
  publication: WorkflowPublicationConfig,
): Promise<unknown> {
  const parsed = await parseRequest(request, publication.parser ?? { kind: "json" })
  validateInput(parsed, publication.input_schema)
  return parsed
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

async function parseCustomCommand(request: GatewayRequest, config: ParserConfig) {
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

function sourceValue(request: GatewayRequest, source: ParserConfig["source"]) {
  if (source === "body") return typeof request.body === "string" ? request.body : JSON.stringify(request.body ?? {})
  if (source === "query") return JSON.stringify(request.query ?? {})
  if (source === "headers") return JSON.stringify(request.headers)
  if (source === "request") return JSON.stringify({ method: request.method, url: request.url, body: request.body ?? {}, query: request.query ?? {} })
  return String(request.url.split("?")[0] ?? "/")
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
  const tls = tlsConfigFromEnv()
  if (tls) config.tls = tls
  return config
}

function tlsConfigFromEnv(): TlsConfig | undefined {
  const keyFile = process.env.ARROBA_PUBLICATION_TLS_KEY_FILE
  const certFile = process.env.ARROBA_PUBLICATION_TLS_CERT_FILE
  if (!keyFile && !certFile) return undefined
  const tls: TlsConfig = { enabled: process.env.ARROBA_PUBLICATION_TLS_ENABLED !== "false" }
  if (keyFile) tls.key_file = keyFile
  if (certFile) tls.cert_file = certFile
  return tls
}

function resolveHttpsOptions(tls: TlsConfig | undefined) {
  if (!tls || tls.enabled === false) return undefined
  if (!tls.key_file || !tls.cert_file) {
    throw new Error("HTTPS requires tls.key_file and tls.cert_file")
  }
  return {
    key: readFileSync(tls.key_file),
    cert: readFileSync(tls.cert_file),
  }
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

async function sleep(ms: number) {
  await new Promise((resolve) => setTimeout(resolve, ms))
}

export async function loadPublicationConfig(path: string) {
  return JSON.parse(await readFile(path, "utf8")) as WorkflowPublicationConfig
}

export async function loadPublicationConfigFromKernel(
  sessionId: string,
  publicationRef: string,
  kernelEndpoint = defaultKernelEndpoint(),
  client?: KernelLookupClient,
): Promise<WorkflowPublicationConfig> {
  const ownedClient = client ?? new LocalIpcClient(kernelEndpoint)
  try {
    const response = await ownedClient.send(
      getWorkflowPublicationRequest(sessionId, publicationRef),
    )
    const publication = (response.WorkflowPublication as { publication?: WorkflowPublicationDefinition } | undefined)?.publication
    if (!publication) {
      throw new Error(`unexpected workflow publication response: ${JSON.stringify(response)}`)
    }
    return publicationConfigFromKernelRecord(publication, kernelEndpoint)
  } finally {
    if (!client) {
      await ownedClient.close?.().catch(() => {})
    }
  }
}

export function publicationConfigFromKernelRecord(
  publication: WorkflowPublicationDefinition,
  kernelEndpoint = defaultKernelEndpoint(),
): WorkflowPublicationConfig {
  const config: WorkflowPublicationConfig = {
    publication_id: publication.id,
    session_id: publication.session_id,
    workflow_ref: publication.workflow_id,
    endpoint_ref: publication.endpoint_id,
    kernel_endpoint: kernelEndpoint,
    route: publication.route ?? "/*",
    auth: asAuthConfig(publication.auth) ?? { mode: "anonymous" },
    parser: asParserConfig(publication.parser) ?? { kind: "json" },
    mode: publication.mode === "async" ? "async" : "sync",
  }
  const methods = normalizeHttpMethods(publication.methods)
  if (methods) config.methods = methods
  const inputSchema = asInputSchema(publication.input_schema)
  if (inputSchema) config.input_schema = inputSchema
  return config
}

export async function loadGatewayPublicationConfig(): Promise<WorkflowPublicationConfig | undefined> {
  if (process.env.ARROBA_PUBLICATION_CONFIG) {
    return withEnvTlsConfig(await loadPublicationConfig(process.env.ARROBA_PUBLICATION_CONFIG))
  }
  if (
    process.env.ARROBA_PUBLICATION_SESSION_ID
    && process.env.ARROBA_PUBLICATION_ID
    && (!process.env.ARROBA_PUBLICATION_WORKFLOW || !process.env.ARROBA_PUBLICATION_ENDPOINT)
  ) {
    return withEnvTlsConfig(await loadPublicationConfigFromKernel(
      process.env.ARROBA_PUBLICATION_SESSION_ID,
      process.env.ARROBA_PUBLICATION_ID,
      defaultKernelEndpoint(),
    ))
  }
  return undefined
}

function withEnvTlsConfig(config: WorkflowPublicationConfig) {
  const tls = tlsConfigFromEnv()
  if (tls) return { ...config, tls }
  return config
}

function normalizeHttpMethods(methods: string[] | undefined): Array<"GET" | "POST"> | undefined {
  const normalized = (methods ?? [])
    .map((method) => method.toUpperCase())
    .filter((method): method is "GET" | "POST" => method === "GET" || method === "POST")
  return normalized.length > 0 ? normalized : undefined
}

function asAuthConfig(value: unknown): AuthConfig | undefined {
  return isPlainObject(value) && typeof value.mode === "string" ? value as AuthConfig : undefined
}

function asParserConfig(value: unknown): ParserConfig | undefined {
  return isPlainObject(value) && typeof value.kind === "string" ? value as ParserConfig : undefined
}

function asInputSchema(value: unknown): InputSchema | undefined {
  return isPlainObject(value) ? value as InputSchema : undefined
}

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === "object" && !Array.isArray(value)
}

if (import.meta.url === `file://${process.argv[1]}`) {
  const config = await loadGatewayPublicationConfig()
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
