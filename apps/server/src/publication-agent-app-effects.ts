import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs"
import { dirname, join, normalize, sep } from "node:path"
import process from "node:process"

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
  readonly invocationOverlays: Map<string, Map<string, AgentAppStoredAsset>>
  readonly sessionOverlays: Map<string, Map<string, AgentAppStoredAsset>>
  readonly persistentPatches: Map<string, AgentAppStoredAsset>
  readonly invocationRoutes: Map<string, AgentAppInvocationContext>
  readonly stateFile?: string
}

type AgentAppInvocationContext = {
  readonly route: AgentAppRouteConfig
  readonly sessionKey?: string | null
  readonly runtimeSessionId?: string | null
}

type AgentAppResolveContext = {
  readonly sessionKey?: string | null
  readonly invocationRequestId?: string | null
}

const effectStores = new Map<string, AgentAppEffectStore>()
const INVOCATION_EFFECT_TTL_MS = 10 * 60 * 1000

export function clearAgentAppEffectStoresForTests(): void {
  effectStores.clear()
}

export function rememberAgentAppInvocationRoute(
  publication: WorkflowPublicationConfig,
  requestId: string,
  route: AgentAppRouteConfig,
  context: { sessionKey?: string | null; runtimeSessionId?: string | null } = {},
): void {
  if (publication.agent_app?.enabled !== true) return
  storeForPublication(publication).invocationRoutes.set(requestId, {
    route,
    sessionKey: context.sessionKey ?? null,
    runtimeSessionId: context.runtimeSessionId ?? null,
  })
  persistStore(publication)
}

export function publicationForAgentAppInvocation(
  publication: WorkflowPublicationConfig,
  requestId: string,
): WorkflowPublicationConfig {
  if (publication.agent_app?.enabled !== true) return publication
  const runtimeSessionId = storeForPublication(publication).invocationRoutes.get(requestId)?.runtimeSessionId
  return runtimeSessionId?.trim() ? { ...publication, session_id: runtimeSessionId.trim() } : publication
}

export function registerAgentAppWorkflowRunEffects(
  publication: WorkflowPublicationConfig,
  workflowRun: WorkflowRun | null | undefined,
  requestId?: string | null,
): void {
  if (publication.agent_app?.enabled !== true || !workflowRun?.final_output) return
  const store = storeForPublication(publication)
  const context = requestId ? store.invocationRoutes.get(requestId) : undefined
  const route = context?.route
  const effects = parseResponseEffects(normalizeFinalOutput(workflowRun.final_output).text)
  if (!effects) return
  for (const asset of effects.overlay) {
    const stored = normalizeStoredAsset(asset)
    if (!stored || !publicationAllowsOverlay(publication, route, stored.path)) continue
    const scope = route?.manipulation?.scope ?? "session"
    if (scope === "invocation" && requestId) {
      mapForKey(store.invocationOverlays, requestId).set(stored.path, stored)
    } else {
      mapForKey(store.sessionOverlays, context?.sessionKey ?? "anonymous").set(stored.path, stored)
    }
  }
  if (publication.agent_app.persistent_patch?.enabled === true && route?.manipulation?.level === "persistent_patch") {
    for (const asset of effects.persistentPatch) {
      const stored = normalizeStoredAsset(asset)
      if (!stored || !routeAllowsOverlay(route, stored.path)) continue
      store.persistentPatches.set(stored.path, stored)
    }
  }
  persistStore(publication)
  if (requestId) scheduleInvocationEffectExpiry(publication, requestId)
}

export function expireAgentAppInvocationEffects(
  publication: WorkflowPublicationConfig,
  requestId: string | null | undefined,
): void {
  if (publication.agent_app?.enabled !== true || !requestId) return
  const store = storeForPublication(publication)
  store.invocationOverlays.delete(requestId)
  store.invocationRoutes.delete(requestId)
  persistStore(publication)
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
  context: AgentAppResolveContext = {},
): AgentAppStoredAsset | null {
  if (publication.agent_app?.enabled !== true) return null
  const normalizedPath = normalizeAppPath(requestPath)
  if (!normalizedPath) return null
  const store = storeForPublication(publication)
  const invocationOverlay = context.invocationRequestId
    ? store.invocationOverlays.get(context.invocationRequestId)?.get(normalizedPath)
    : null
  const sessionOverlay = context.sessionKey
    ? store.sessionOverlays.get(context.sessionKey)?.get(normalizedPath)
    : null
  return invocationOverlay ?? sessionOverlay ?? store.persistentPatches.get(normalizedPath) ?? null
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
  const created: AgentAppEffectStore = loadPersistedStore(publication) ?? {
    invocationOverlays: new Map(),
    sessionOverlays: new Map(),
    persistentPatches: new Map(),
    invocationRoutes: new Map(),
    stateFile: stateFileForPublication(publication) ?? undefined,
  }
  effectStores.set(key, created)
  return created
}

function mapForKey<K, V>(store: Map<string, Map<K, V>>, key: string): Map<K, V> {
  const existing = store.get(key)
  if (existing) return existing
  const created = new Map<K, V>()
  store.set(key, created)
  return created
}

function scheduleInvocationEffectExpiry(publication: WorkflowPublicationConfig, requestId: string): void {
  const timeout = setTimeout(() => expireAgentAppInvocationEffects(publication, requestId), INVOCATION_EFFECT_TTL_MS)
  timeout.unref?.()
}

type PersistedAgentAppEffectStore = {
  readonly schema_version: 1
  readonly invocation_overlays?: Record<string, PersistedAgentAppStoredAsset[]>
  readonly session_overlays?: Record<string, PersistedAgentAppStoredAsset[]>
  readonly persistent_patches?: PersistedAgentAppStoredAsset[]
  readonly invocation_routes?: Record<string, PersistedAgentAppInvocationContext>
}

type PersistedAgentAppStoredAsset = {
  readonly path: string
  readonly mime_type: string
  readonly content: string
}

type PersistedAgentAppInvocationContext = {
  readonly route: AgentAppRouteConfig
  readonly session_key?: string | null
  readonly runtime_session_id?: string | null
}

function loadPersistedStore(publication: WorkflowPublicationConfig): AgentAppEffectStore | null {
  const stateFile = stateFileForPublication(publication)
  if (!stateFile || !existsSync(stateFile)) return null
  try {
    const persisted = JSON.parse(readFileSync(stateFile, "utf8")) as PersistedAgentAppEffectStore
    if (persisted.schema_version !== 1) return null
    return {
      invocationOverlays: persistedOverlayMap(persisted.invocation_overlays),
      sessionOverlays: persistedOverlayMap(persisted.session_overlays),
      persistentPatches: assetMapFromArray(persisted.persistent_patches ?? []),
      invocationRoutes: invocationRoutesFromRecord(persisted.invocation_routes ?? {}),
      stateFile,
    }
  } catch {
    return {
      invocationOverlays: new Map(),
      sessionOverlays: new Map(),
      persistentPatches: new Map(),
      invocationRoutes: new Map(),
      stateFile,
    }
  }
}

function persistStore(publication: WorkflowPublicationConfig): void {
  const store = storeForPublication(publication)
  if (!store.stateFile) return
  const persisted: PersistedAgentAppEffectStore = {
    schema_version: 1,
    invocation_overlays: overlayMapToRecord(store.invocationOverlays),
    session_overlays: overlayMapToRecord(store.sessionOverlays),
    persistent_patches: assetArrayFromMap(store.persistentPatches),
    invocation_routes: invocationRoutesToRecord(store.invocationRoutes),
  }
  mkdirSync(dirname(store.stateFile), { recursive: true })
  const temporary = `${store.stateFile}.${process.pid}.tmp`
  writeFileSync(temporary, `${JSON.stringify(persisted, null, 2)}\n`, "utf8")
  renameSync(temporary, store.stateFile)
}

function stateFileForPublication(publication: WorkflowPublicationConfig): string | null {
  const root = process.env.ARROBA_PUBLICATION_RUNTIME_STATE_DIR
    || (publication.package_root ? join(publication.package_root, ".arroba-publication-runtime") : null)
  if (!root) return null
  return join(root, "agent-app", safeStateSegment(publication.publication_id), "effects.json")
}

function persistedOverlayMap(
  record: Record<string, readonly PersistedAgentAppStoredAsset[]> | undefined,
): Map<string, Map<string, AgentAppStoredAsset>> {
  const result = new Map<string, Map<string, AgentAppStoredAsset>>()
  for (const [key, assets] of Object.entries(record ?? {})) {
    result.set(key, assetMapFromArray(assets))
  }
  return result
}

function overlayMapToRecord(
  map: Map<string, Map<string, AgentAppStoredAsset>>,
): Record<string, PersistedAgentAppStoredAsset[]> {
  const record: Record<string, PersistedAgentAppStoredAsset[]> = {}
  for (const [key, assets] of map.entries()) {
    record[key] = assetArrayFromMap(assets)
  }
  return record
}

function assetMapFromArray(assets: readonly PersistedAgentAppStoredAsset[]): Map<string, AgentAppStoredAsset> {
  const map = new Map<string, AgentAppStoredAsset>()
  for (const asset of assets) {
    const normalized = normalizeStoredAsset({
      path: asset.path,
      mime_type: asset.mime_type,
      content: asset.content,
    })
    if (normalized) map.set(normalized.path, normalized)
  }
  return map
}

function assetArrayFromMap(map: Map<string, AgentAppStoredAsset>): PersistedAgentAppStoredAsset[] {
  return [...map.values()].map((asset) => ({
    path: asset.path,
    mime_type: asset.mimeType,
    content: Buffer.isBuffer(asset.content) ? asset.content.toString("base64") : asset.content,
  }))
}

function invocationRoutesFromRecord(
  record: Record<string, PersistedAgentAppInvocationContext>,
): Map<string, AgentAppInvocationContext> {
  const map = new Map<string, AgentAppInvocationContext>()
  for (const [requestId, context] of Object.entries(record)) {
    map.set(requestId, {
      route: context.route,
      sessionKey: context.session_key ?? null,
      runtimeSessionId: context.runtime_session_id ?? null,
    })
  }
  return map
}

function invocationRoutesToRecord(
  map: Map<string, AgentAppInvocationContext>,
): Record<string, PersistedAgentAppInvocationContext> {
  const record: Record<string, PersistedAgentAppInvocationContext> = {}
  for (const [requestId, context] of map.entries()) {
    record[requestId] = {
      route: context.route,
      session_key: context.sessionKey ?? null,
      runtime_session_id: context.runtimeSessionId ?? null,
    }
  }
  return record
}

function safeStateSegment(value: string): string {
  return value.replace(/[^a-zA-Z0-9_.-]/g, "_") || "publication"
}
