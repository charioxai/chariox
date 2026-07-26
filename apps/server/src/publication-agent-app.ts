import { readFile, stat } from "node:fs/promises"
import {
  closeSync,
  constants as fsConstants,
  fstatSync,
  lstatSync,
  openSync,
  readFileSync,
  unlinkSync,
} from "node:fs"
import { randomUUID } from "node:crypto"
import { extname, normalize, resolve, sep } from "node:path"

import {
  appendCloudPublicationDeploymentLogs,
  type PublicationDeploymentLogEntry,
} from "./publication-cloud-deployment.js"
import {
  rememberAgentAppInvocationRoute,
  resolveAgentAppEffectAsset,
  registerAgentAppWorkflowRunEffects,
} from "./publication-agent-app-effects.js"
import {
  agentAppCallerKey,
  agentAppCallerSession,
  agentAppReplicaStatus,
  acquireAgentAppReplica,
  enqueueAgentAppReplicaDispatch,
  releaseAgentAppReplicaInvocation,
  trackAgentAppReplicaInvocation,
  type AgentAppReplicaLease,
} from "./publication-agent-app-replicas.js"
import {
  invokeKernelWorkflow,
} from "./kernel-publication-client.js"
import {
  publicationCallerForRequest,
  publicationInvocationCaller,
  type VerifiedPublicationCallerClaims,
} from "./publication-caller-claims.js"
import {
  forwardHumanHttpResult,
} from "./publication-human-http.js"
import {
  validateInput,
} from "./publication-parser.js"
import type {
  AgentAppActionConfig,
  AgentAppRouteConfig,
  GatewayDeps,
  NormalizedInvocation,
  WorkflowInvocationResult,
  WorkflowPublicationConfig,
} from "./publication-types.js"
import { isTerminalWorkflowRunStatus } from "./workflow-run-status.js"

type AgentAppFastify = {
  get: (path: string, handler: (request: AgentAppRequest, reply: AgentAppReply) => unknown) => unknown
  route: (options: {
    method: string | string[]
    url: string
    handler: (request: AgentAppRequest, reply: AgentAppReply) => unknown
  }) => unknown
}

type AgentAppRequest = {
  method: string
  url: string
  headers: Record<string, string | string[] | undefined>
  params?: unknown
  body?: unknown
}

type AgentAppReply = {
  code: (code: number) => AgentAppReply
  header: (name: string, value: string) => AgentAppReply
  type: (contentType: string) => AgentAppReply
}

const AGENT_APP_AUDIT_PATH = "/.well-known/arroba/agent-app/audit-log"
const AGENT_APP_AUDIT_TOKEN = randomUUID()
const MAX_AGENT_APP_AUDIT_URL_BYTES = 8 * 1024
let configuredAgentAppAuditUrl: string | undefined

export function consumeAgentAppAuditUrlFromEnv(): void {
  const directUrl = process.env.ARROBA_PUBLICATION_AGENT_APP_AUDIT_URL?.trim()
  const urlFile = process.env.ARROBA_PUBLICATION_AGENT_APP_AUDIT_URL_FILE?.trim()
  delete process.env.ARROBA_PUBLICATION_AGENT_APP_AUDIT_URL
  delete process.env.ARROBA_PUBLICATION_AGENT_APP_AUDIT_URL_FILE
  if (directUrl && urlFile) throw new Error("publication audit URL and URL file cannot both be configured")
  if (directUrl) {
    configuredAgentAppAuditUrl = normalizeAgentAppAuditUrl(directUrl)
    return
  }
  if (!urlFile) return
  configuredAgentAppAuditUrl = readPrivateAgentAppAuditUrlFile(urlFile)
}

export function readPrivateAgentAppAuditUrlFile(path: string): string {
  let descriptor: number
  try {
    descriptor = openSync(path, fsConstants.O_RDONLY | fsConstants.O_NOFOLLOW)
  } catch (error) {
    throw new Error(`publication audit URL file could not be opened safely: ${String(error)}`)
  }
  try {
    const metadata = fstatSync(descriptor)
    const currentUid = process.getuid?.()
    if (
      !metadata.isFile()
      || (metadata.mode & 0o077) !== 0
      || (currentUid !== undefined && metadata.uid !== currentUid)
      || metadata.nlink !== 1
      || metadata.size > MAX_AGENT_APP_AUDIT_URL_BYTES
    ) {
      throw new Error(
        "publication audit URL file must be a bounded, single-link owned regular file with mode 0600",
      )
    }
    const pathMetadata = lstatSync(path)
    if (
      !pathMetadata.isFile()
      || pathMetadata.isSymbolicLink()
      || pathMetadata.dev !== metadata.dev
      || pathMetadata.ino !== metadata.ino
    ) {
      throw new Error("publication audit URL file changed while it was being consumed")
    }
    unlinkSync(path)
    if (fstatSync(descriptor).nlink !== 0) {
      throw new Error("publication audit URL file was not consumed from its validated descriptor")
    }
    return normalizeAgentAppAuditUrl(readFileSync(descriptor, "utf8"))
  } finally {
    closeSync(descriptor)
  }
}

function normalizeAgentAppAuditUrl(value: string): string {
  const trimmed = value.trim()
  if (!trimmed) throw new Error("publication audit URL must not be empty")
  let url: URL
  try {
    url = new URL(trimmed)
  } catch {
    throw new Error("publication audit URL must be an absolute HTTP(S) URL")
  }
  if (
    (url.protocol !== "http:" && url.protocol !== "https:")
    || url.username !== ""
    || url.password !== ""
    || url.hash !== ""
  ) {
    throw new Error("publication audit URL must be an absolute HTTP(S) URL without credentials or fragments")
  }
  return url.href
}

export function isAgentAppPublication(publication: WorkflowPublicationConfig): boolean {
  return publication.agent_app?.enabled === true
}

export function installAgentAppRoutes(
  app: AgentAppFastify,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
): void {
  if (!isAgentAppPublication(publication)) return
  for (const route of publication.agent_app?.routes ?? []) {
    app.route({
      method: "GET",
      url: route.path,
      handler: async (request, reply) => invokeAgentAppRoute(route, request, reply, publication, deps),
    })
  }
  app.route({
    method: "POST",
    url: "/.well-known/arroba/agent-app/actions/:actionId",
    handler: async (request, reply) => invokeAgentAppAction(request, reply, publication),
  })
  app.route({
    method: "POST",
    url: AGENT_APP_AUDIT_PATH,
    handler: async (request, reply) => appendAgentAppAuditLog(request, reply),
  })
  app.get("/.well-known/arroba/agent-app/status", async () => ({
    publication_id: publication.publication_id,
    replicas: agentAppReplicaStatus(publication),
  }))
  app.get("/*", async (request, reply) => serveAgentAppAsset(request, reply, publication))
}

async function invokeAgentAppRoute(
  route: AgentAppRouteConfig,
  request: AgentAppRequest,
  reply: AgentAppReply,
  publication: WorkflowPublicationConfig,
  deps: GatewayDeps,
) {
  const prompt = promptFromRoute(request.url, route)
  const caller = publicationCallerForRequest(request)
  const callerSession = agentAppCallerSession(request.headers, randomUUID, caller)
  if (callerSession.setCookie) reply.header("set-cookie", callerSession.setCookie)
  const requestId = `agentapp_${Date.now()}_${Math.random().toString(16).slice(2)}`
  rememberAgentAppInvocationRoute(publication, requestId, route, {
    sessionKey: callerSession.callerKey,
  })
  const lease = acquireAgentAppReplica(publication, callerSession.callerKey)
  if (!lease) {
    const queued = enqueueAgentAppReplicaDispatch(publication, callerSession.callerKey, async (queuedLease) => {
      await invokeSelectedAgentAppRoute({
        route,
        prompt,
        publication,
        lease: queuedLease,
        requestId,
        callerKey: callerSession.callerKey,
        caller,
        deps,
      })
    })
    if (!queued) {
      reply.code(429)
      return { error: "agent app replica pool queue is full" }
    }
    return forwardHumanHttpResult(reply as never, publication, {
      accepted: true,
      queued: true,
      response: { agent_app_pool_queued: true, invocation_id: requestId },
    }, requestId, false, { prompt })
  }

  const { selectedPublication, result } = await invokeSelectedAgentAppRoute({
    route,
    prompt,
    publication,
    lease,
    requestId,
    callerKey: callerSession.callerKey,
    caller,
    deps,
  })
  return forwardHumanHttpResult(reply as never, selectedPublication, result, requestId, false, { prompt })
}

async function invokeSelectedAgentAppRoute(options: {
  route: AgentAppRouteConfig
  prompt: string
  publication: WorkflowPublicationConfig
  lease: AgentAppReplicaLease
  requestId: string
  callerKey: string
  caller: VerifiedPublicationCallerClaims | null
  deps: GatewayDeps
}): Promise<{ selectedPublication: WorkflowPublicationConfig; result: WorkflowInvocationResult }> {
  const selectedPublication = options.lease.publication
  const invocation: NormalizedInvocation = {
    publication_id: selectedPublication.publication_id,
    request_id: options.requestId,
    caller: publicationInvocationCaller(options.caller, {
      transport: "agent_app_human_http",
      route: options.route.path,
      agent_app_session: options.callerKey,
      agent_app_request_id: options.requestId,
      replica_session_id: selectedPublication.session_id,
      agent_app_actions: routeAgentAppActions(options.publication, options.route),
      agent_app_audit: agentAppAuditProof(),
    }),
    input: { prompt: options.prompt },
    mode: "async",
  }
  rememberAgentAppInvocationRoute(options.publication, options.requestId, options.route, {
    sessionKey: options.callerKey,
    runtimeSessionId: selectedPublication.session_id,
  })
  trackAgentAppReplicaInvocation(options.publication, options.requestId, options.lease)
  try {
    const result = options.deps.invokeWorkflow
      ? await options.deps.invokeWorkflow(invocation)
      : await invokeKernelWorkflow({ ...selectedPublication, mode: "async" }, invocation)
    registerAgentAppWorkflowRunEffects(selectedPublication, result.workflow_run, invocation.request_id)
    if (result.workflow_run && isTerminalWorkflowRunStatus(result.workflow_run.status)) {
      releaseAgentAppReplicaInvocation(options.publication, options.requestId)
    }
    return { selectedPublication, result }
  } catch (error) {
    releaseAgentAppReplicaInvocation(options.publication, options.requestId)
    throw error
  }
}

async function appendAgentAppAuditLog(
  request: AgentAppRequest,
  reply: AgentAppReply,
) {
  const body = request.body && typeof request.body === "object" && !Array.isArray(request.body)
    ? request.body as { token?: unknown; entries?: unknown }
    : null
  if (body?.token !== AGENT_APP_AUDIT_TOKEN) {
    reply.code(403)
    return { error: "agent app audit token is invalid" }
  }
  if (!Array.isArray(body.entries)) {
    reply.code(400)
    return { error: "agent app audit entries are required" }
  }
  const entries = body.entries
    .map(normalizeAuditEntry)
    .filter((entry): entry is PublicationDeploymentLogEntry => Boolean(entry))
  if (entries.length === 0) {
    reply.code(400)
    return { error: "agent app audit entries are invalid" }
  }
  try {
    const appended = await appendCloudPublicationDeploymentLogs({ entries })
    return { accepted: true, appended }
  } catch (error) {
    reply.code(502)
    return { error: error instanceof Error ? error.message : String(error) }
  }
}

function normalizeAuditEntry(value: unknown): PublicationDeploymentLogEntry | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null
  const record = value as Record<string, unknown>
  if (typeof record.message !== "string" || record.message.trim() === "") return null
  return {
    level: typeof record.level === "string" ? record.level : "info",
    message: record.message,
    metadata: record.metadata,
    occurredAt: typeof record.occurredAt === "string" ? record.occurredAt : new Date().toISOString(),
  }
}

function agentAppAuditProof(): { url: string; token: string } | undefined {
  const explicitUrl = configuredAgentAppAuditUrl
    ?? process.env.ARROBA_PUBLICATION_AGENT_APP_AUDIT_URL?.trim()
  if (explicitUrl) {
    return { url: explicitUrl, token: AGENT_APP_AUDIT_TOKEN }
  }
  const port = process.env.PORT?.trim()
  if (!port) return undefined
  return {
    url: `http://127.0.0.1:${port}${AGENT_APP_AUDIT_PATH}`,
    token: AGENT_APP_AUDIT_TOKEN,
  }
}

async function invokeAgentAppAction(
  request: AgentAppRequest,
  reply: AgentAppReply,
  publication: WorkflowPublicationConfig,
) {
  const actionId = (request.params as { actionId?: string } | undefined)?.actionId
  if (!actionId) {
    reply.code(400)
    return { error: "agent app action id is required" }
  }
  if (!allowedAgentAppActions(publication).has(actionId)) {
    reply.code(403)
    return { error: "agent app action is not allowed by any wrapped route" }
  }
  const action = publication.agent_app?.actions?.[actionId]
  if (!action) {
    reply.code(404)
    return { error: "agent app action not found" }
  }
  try {
    validateInput(request.body ?? {}, action.input_schema)
    const response = await invokeHttpAction(action, request.body ?? {})
    reply.code(response.status).type(response.contentType)
    return response.body
  } catch (error) {
    reply.code(400)
    return { error: error instanceof Error ? error.message : String(error) }
  }
}

async function invokeHttpAction(
  action: AgentAppActionConfig,
  input: unknown,
): Promise<{ status: number; contentType: string; body: unknown }> {
  const transport = action.transport
  if (transport?.kind !== "http" || !transport.url) {
    throw new Error("agent app action requires http transport url")
  }
  const response = await fetch(transport.url, {
    method: transport.method ?? "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
    redirect: "manual",
    signal: AbortSignal.timeout(30_000),
  })
  if (response.status >= 300 && response.status < 400) {
    throw new Error("agent app action redirects are forbidden")
  }
  const contentType = response.headers.get("content-type") ?? "application/json; charset=utf-8"
  const text = await readBoundedAgentAppActionResponse(response, 1_048_576)
  let body: unknown = text
  if (contentType.includes("application/json")) {
    try {
      body = text ? JSON.parse(text) : null
    } catch {
      body = text
    }
  }
  return { status: response.status, contentType, body }
}

async function readBoundedAgentAppActionResponse(response: Response, maxBytes: number): Promise<string> {
  const declaredLength = Number(response.headers.get("content-length"))
  if (Number.isFinite(declaredLength) && declaredLength > maxBytes) {
    await response.body?.cancel()
    throw new Error("agent app action response exceeds the byte limit")
  }
  if (!response.body) return ""
  const reader = response.body.getReader()
  const chunks: Uint8Array[] = []
  let byteLength = 0
  try {
    while (true) {
      const chunk = await reader.read()
      if (chunk.done) break
      byteLength += chunk.value.byteLength
      if (byteLength > maxBytes) throw new Error("agent app action response exceeds the byte limit")
      chunks.push(chunk.value)
    }
  } finally {
    await reader.cancel().catch(() => {})
  }
  const bytes = new Uint8Array(byteLength)
  let offset = 0
  for (const chunk of chunks) {
    bytes.set(chunk, offset)
    offset += chunk.byteLength
  }
  return new TextDecoder().decode(bytes)
}

function promptFromRoute(requestUrl: string, route: AgentAppRouteConfig): string {
  if (route.prompt_source !== "path_tail") return ""
  const path = new URL(requestUrl, "http://agent-app.local").pathname
  const routePrefix = route.path.endsWith("*") ? route.path.slice(0, -1) : route.path
  if (!path.startsWith(routePrefix)) return ""
  return decodeURIComponent(path.slice(routePrefix.length).replace(/^\/+/, ""))
}

function allowedAgentAppActions(publication: WorkflowPublicationConfig): Set<string> {
  const allowed = new Set<string>()
  for (const route of publication.agent_app?.routes ?? []) {
    for (const actionId of route.manipulation?.allowed_actions ?? []) {
      allowed.add(actionId)
    }
  }
  return allowed
}

function routeAgentAppActions(
  publication: WorkflowPublicationConfig,
  route: AgentAppRouteConfig,
): Record<string, AgentAppActionConfig> {
  const actions: Record<string, AgentAppActionConfig> = {}
  for (const actionId of route.manipulation?.allowed_actions ?? []) {
    const action = publication.agent_app?.actions?.[actionId]
    if (action) actions[actionId] = action
  }
  return actions
}

async function serveAgentAppAsset(
  request: AgentAppRequest,
  reply: AgentAppReply,
  publication: WorkflowPublicationConfig,
) {
  if (request.url.startsWith("/.well-known/arroba/")) {
    reply.code(404)
    return { error: "not found" }
  }
  const packageRoot = publication.package_root
  if (!packageRoot) {
    reply.code(404)
    return { error: "agent app package root is unavailable" }
  }
  const assets = publication.agent_app?.assets ?? {}
  const publicDir = assets.public_dir ?? "app"
  const index = assets.index ?? "index.html"
  const parsedUrl = new URL(request.url, "http://agent-app.local")
  const pathname = parsedUrl.pathname
  const effectAsset = resolveAgentAppEffectAsset(publication, pathname, {
    sessionKey: agentAppCallerKey(request.headers, publicationCallerForRequest(request)),
    invocationRequestId: parsedUrl.searchParams.get("arroba_invocation"),
  })
  if (effectAsset) {
    reply.type(effectAsset.mimeType)
    return effectAsset.content
  }
  const relativePath = pathname === "/" ? index : decodeURIComponent(pathname.replace(/^\/+/, ""))
  const assetRoot = resolve(packageRoot, publicDir)
  const assetPath = resolve(assetRoot, normalize(relativePath))
  if (!isPathInside(assetRoot, assetPath)) {
    reply.code(403)
    return { error: "asset path is outside the agent app root" }
  }
  try {
    const fileStat = await stat(assetPath)
    if (!fileStat.isFile()) {
      reply.code(404)
      return { error: "not found" }
    }
    reply.type(contentTypeForPath(assetPath))
    return await readFile(assetPath)
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") {
      reply.code(404)
      return { error: "not found" }
    }
    throw error
  }
}

function isPathInside(root: string, path: string): boolean {
  const normalizedRoot = root.endsWith(sep) ? root : `${root}${sep}`
  return path === root || path.startsWith(normalizedRoot)
}

function contentTypeForPath(path: string): string {
  const extension = extname(path).toLowerCase()
  if (extension === ".html") return "text/html; charset=utf-8"
  if (extension === ".css") return "text/css; charset=utf-8"
  if (extension === ".js" || extension === ".mjs") return "text/javascript; charset=utf-8"
  if (extension === ".json") return "application/json; charset=utf-8"
  if (extension === ".svg") return "image/svg+xml"
  if (extension === ".png") return "image/png"
  if (extension === ".jpg" || extension === ".jpeg") return "image/jpeg"
  return "application/octet-stream"
}
