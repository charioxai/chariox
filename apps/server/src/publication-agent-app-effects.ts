import { normalize, sep } from "node:path"

import { normalizeFinalOutput } from "./publication-final-output.js"
import type {
  AgentAppRouteConfig,
  WorkflowPublicationConfig,
  WorkflowRun,
} from "./publication-types.js"

type AgentAppStoredAsset = {
  readonly path: string
  readonly mimeType: string
  readonly content: string | Buffer
}

type AgentAppEffectStore = {
  readonly overlays: Map<string, AgentAppStoredAsset>
  readonly persistentPatches: Map<string, AgentAppStoredAsset>
  readonly invocationRoutes: Map<string, AgentAppRouteConfig>
}

const effectStores = new Map<string, AgentAppEffectStore>()

export function rememberAgentAppInvocationRoute(
  publication: WorkflowPublicationConfig,
  requestId: string,
  route: AgentAppRouteConfig,
): void {
  if (publication.agent_app?.enabled !== true) return
  storeForPublication(publication).invocationRoutes.set(requestId, route)
}

export function registerAgentAppWorkflowRunEffects(
  publication: WorkflowPublicationConfig,
  workflowRun: WorkflowRun | null | undefined,
  requestId?: string | null,
): void {
  if (publication.agent_app?.enabled !== true || !workflowRun?.final_output) return
  const store = storeForPublication(publication)
  const route = requestId ? store.invocationRoutes.get(requestId) : undefined
  const effects = parseResponseEffects(normalizeFinalOutput(workflowRun.final_output).text)
  if (!effects) return
  for (const asset of effects.overlay) {
    const stored = normalizeStoredAsset(asset)
    if (!stored || !publicationAllowsOverlay(publication, route, stored.path)) continue
    store.overlays.set(stored.path, stored)
  }
  if (publication.agent_app.persistent_patch?.enabled === true && route?.manipulation?.level === "persistent_patch") {
    for (const asset of effects.persistentPatch) {
      const stored = normalizeStoredAsset(asset)
      if (!stored || !routeAllowsOverlay(route, stored.path)) continue
      store.persistentPatches.set(stored.path, stored)
    }
  }
}

function publicationAllowsOverlay(
  publication: WorkflowPublicationConfig,
  route: AgentAppRouteConfig | undefined,
  path: string,
): boolean {
  if (route) return routeAllowsOverlay(route, path)
  return (publication.agent_app?.routes ?? []).some((candidate) => routeAllowsOverlay(candidate, path))
}

export function resolveAgentAppEffectAsset(
  publication: WorkflowPublicationConfig,
  requestPath: string,
): AgentAppStoredAsset | null {
  if (publication.agent_app?.enabled !== true) return null
  const normalizedPath = normalizeAppPath(requestPath)
  if (!normalizedPath) return null
  const store = storeForPublication(publication)
  return store.overlays.get(normalizedPath) ?? store.persistentPatches.get(normalizedPath) ?? null
}

type ResponseEffects = {
  readonly overlay: readonly unknown[]
  readonly persistentPatch: readonly unknown[]
}

function parseResponseEffects(message: string): ResponseEffects | null {
  try {
    const parsed = JSON.parse(message) as { readonly effects?: Record<string, unknown> } | null
    if (!parsed?.effects || typeof parsed.effects !== "object") return null
    return {
      overlay: Array.isArray(parsed.effects.overlay) ? parsed.effects.overlay : [],
      persistentPatch: Array.isArray(parsed.effects.persistent_patch) ? parsed.effects.persistent_patch : [],
    }
  } catch {
    return null
  }
}

function normalizeStoredAsset(value: unknown): AgentAppStoredAsset | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null
  const record = value as Record<string, unknown>
  const path = typeof record.path === "string" ? normalizeAppPath(record.path) : null
  const mimeType = typeof record.mime_type === "string" && record.mime_type.trim()
    ? record.mime_type.trim()
    : "application/octet-stream"
  const content = typeof record.content === "string" ? record.content : null
  if (!path || content === null) return null
  return { path, mimeType, content }
}

function routeAllowsOverlay(route: AgentAppRouteConfig | undefined, path: string): boolean {
  const manipulation = route?.manipulation
  const level = manipulation?.level ?? "none"
  if (!["overlay", "state_and_overlay", "full_ephemeral", "persistent_patch"].includes(level)) return false
  const protectedPaths = manipulation?.protected_paths ?? []
  if (protectedPaths.some((pattern) => pathMatchesPattern(path, pattern))) return false
  const allowedPaths = manipulation?.allowed_paths ?? []
  return allowedPaths.length === 0 || allowedPaths.some((pattern) => pathMatchesPattern(path, pattern))
}

function normalizeAppPath(value: string): string | null {
  const normalized = normalize(`/${value}`).replaceAll(sep, "/")
  if (normalized.includes("\0") || normalized.includes("..")) return null
  return normalized.startsWith("/") ? normalized : `/${normalized}`
}

function pathMatchesPattern(path: string, pattern: string): boolean {
  const normalizedPattern = normalizeAppPath(pattern)
  if (!normalizedPattern) return false
  if (normalizedPattern.endsWith("/**")) {
    const prefix = normalizedPattern.slice(0, -3)
    return path === prefix || path.startsWith(`${prefix}/`)
  }
  if (normalizedPattern.endsWith("/*")) {
    const prefix = normalizedPattern.slice(0, -2)
    return path.startsWith(`${prefix}/`) && !path.slice(prefix.length + 1).includes("/")
  }
  return path === normalizedPattern
}

function storeForPublication(publication: WorkflowPublicationConfig): AgentAppEffectStore {
  const key = publication.publication_id
  const existing = effectStores.get(key)
  if (existing) return existing
  const created: AgentAppEffectStore = {
    overlays: new Map(),
    persistentPatches: new Map(),
    invocationRoutes: new Map(),
  }
  effectStores.set(key, created)
  return created
}
