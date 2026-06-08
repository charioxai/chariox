import { readFile, stat } from "node:fs/promises"
import { extname, normalize, resolve, sep } from "node:path"

import {
  invokeKernelWorkflow,
} from "./kernel-publication-client.js"
import {
  forwardHumanHttpResult,
} from "./publication-human-http.js"
import type {
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
  const invocation: NormalizedInvocation = {
    publication_id: publication.publication_id,
    request_id: `agentapp_${Date.now()}_${Math.random().toString(16).slice(2)}`,
    caller: {
      type: "anonymous",
      proof: {
        transport: "agent_app_human_http",
        route: route.path,
      },
    },
    input: { prompt },
    mode: "async",
  }
  const result = deps.invokeWorkflow
    ? await deps.invokeWorkflow(invocation)
    : await invokeKernelWorkflow({ ...publication, mode: "async" }, invocation)
  return forwardHumanHttpResult(reply as never, publication, result, invocation.request_id)
}

function promptFromRoute(requestUrl: string, route: AgentAppRouteConfig): string {
  if (route.prompt_source !== "path_tail") return ""
  const path = new URL(requestUrl, "http://agent-app.local").pathname
  const routePrefix = route.path.endsWith("*") ? route.path.slice(0, -1) : route.path
  if (!path.startsWith(routePrefix)) return ""
  return decodeURIComponent(path.slice(routePrefix.length).replace(/^\/+/, ""))
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
