import { readFile, stat } from "node:fs/promises"
import { extname, normalize, resolve, sep } from "node:path"

import {
  rememberAgentAppInvocationRoute,
  resolveAgentAppEffectAsset,
  registerAgentAppWorkflowRunEffects,
} from "./publication-agent-app-effects.js"
import {
  agentAppCallerKey,
  selectAgentAppReplica,
} from "./publication-agent-app-replicas.js"
import {
  invokeKernelWorkflow,
} from "./kernel-publication-client.js"
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
  WorkflowPublicationConfig,
} from "./publication-types.js"

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
  const selectedPublication = selectAgentAppReplica(publication, agentAppCallerKey(request.headers))
  const invocation: NormalizedInvocation = {
    publication_id: selectedPublication.publication_id,
    request_id: `agentapp_${Date.now()}_${Math.random().toString(16).slice(2)}`,
    caller: {
      type: "anonymous",
      proof: {
        transport: "agent_app_human_http",
        route: route.path,
        replica_session_id: selectedPublication.session_id,
      },
    },
    input: { prompt },
    mode: "async",
  }
  rememberAgentAppInvocationRoute(publication, invocation.request_id, route)
  const result = deps.invokeWorkflow
    ? await deps.invokeWorkflow(invocation)
    : await invokeKernelWorkflow({ ...selectedPublication, mode: "async" }, invocation)
  registerAgentAppWorkflowRunEffects(selectedPublication, result.workflow_run, invocation.request_id)
  return forwardHumanHttpResult(reply as never, selectedPublication, result, invocation.request_id)
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
  })
  const contentType = response.headers.get("content-type") ?? "application/json; charset=utf-8"
  const text = await response.text()
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
  const pathname = new URL(request.url, "http://agent-app.local").pathname
  const effectAsset = resolveAgentAppEffectAsset(publication, pathname)
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
