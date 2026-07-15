import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs"
import { dirname, join } from "node:path"
import process from "node:process"

import type {
  WorkflowPublicationConfig,
} from "./publication-types.js"

type ReplicaPoolState = {
  nextIndex: number
  callerSessions: Map<string, string>
  busyCounts: Map<string, number>
  pending: PendingReplicaDispatch[]
  invocationLeases: Map<string, TrackedReplicaLease>
  stateFile?: string
}

const replicaPools = new Map<string, ReplicaPoolState>()
const AGENT_APP_SESSION_COOKIE = "arroba_agent_app_session"

type PendingReplicaDispatch = {
  readonly publication: WorkflowPublicationConfig
  readonly callerKey: string
  readonly dispatch: (lease: AgentAppReplicaLease) => Promise<void>
}

type TrackedReplicaLease = {
  readonly release: () => void
  timeout?: ReturnType<typeof setTimeout>
}

export type AgentAppCallerSession = {
  readonly callerKey: string
  readonly setCookie?: string
}

export type AgentAppReplicaLease = {
  readonly publication: WorkflowPublicationConfig
  release: () => void
}

export type AgentAppReplicaStatus = {
  readonly totalReplicaCount: number
  readonly activeReplicaCount: number
  readonly readyReplicaCount: number
  readonly queueDepth: number
}

export function clearAgentAppReplicaPoolsForTests(): void {
  replicaPools.clear()
}

export function agentAppCallerSession(
  headers: Record<string, string | string[] | undefined>,
  createSessionId: () => string,
): AgentAppCallerSession {
  const explicit = firstHeader(headers["x-arroba-agent-app-caller"])
  if (explicit) return { callerKey: explicit }

  const cookie = agentAppSessionCookie(headers)
  if (cookie) return { callerKey: cookie }

  const sessionId = createSessionId()
  return {
    callerKey: sessionId,
    setCookie: `${AGENT_APP_SESSION_COOKIE}=${encodeURIComponent(sessionId)}; Path=/; HttpOnly; SameSite=Lax`,
  }
}

export function agentAppCallerKey(headers: Record<string, string | string[] | undefined>): string {
  const explicit = firstHeader(headers["x-arroba-agent-app-caller"])
  if (explicit) return explicit
  const cookie = agentAppSessionCookie(headers)
  if (cookie) return cookie
  const forwarded = firstHeader(headers["x-forwarded-for"])
  if (forwarded) return forwarded.split(",")[0]?.trim() || "anonymous"
  return "anonymous"
}

export function acquireAgentAppReplica(
  publication: WorkflowPublicationConfig,
  callerKey: string,
): AgentAppReplicaLease | null {
  const sessions = normalizedReplicaSessions(publication)
  if (publication.agent_app?.replicas?.count === undefined) {
    return {
      publication,
      release: () => {},
    }
  }
  const pool = replicaPool(publication)
  if (
    publication.agent_app?.replicas?.per_caller_ordering !== false
    && pool.pending.some((pending) => pending.callerKey === callerKey)
  ) return null
  return acquireIdleAgentAppReplica(publication, callerKey, sessions, pool)
}

function acquireIdleAgentAppReplica(
  publication: WorkflowPublicationConfig,
  callerKey: string,
  sessions: string[],
  pool: ReplicaPoolState,
): AgentAppReplicaLease | null {
  if (publication.agent_app?.replicas?.per_caller_ordering !== false) {
    const existing = pool.callerSessions.get(callerKey)
    if (existing && sessions.includes(existing)) {
      if ((pool.busyCounts.get(existing) ?? 0) > 0) return null
      return leaseReplica(publication, existing, pool)
    }
  }
  const sessionId = nextIdleSession(sessions, pool)
  if (!sessionId) return null
  if (publication.agent_app?.replicas?.per_caller_ordering !== false) {
    pool.callerSessions.set(callerKey, sessionId)
    persistReplicaPool(publication, pool)
  }
  return leaseReplica(publication, sessionId, pool)
}

export function enqueueAgentAppReplicaDispatch(
  publication: WorkflowPublicationConfig,
  callerKey: string,
  dispatch: (lease: AgentAppReplicaLease) => Promise<void>,
): boolean {
  const pool = replicaPool(publication)
  const maxQueueDepth = publication.agent_app?.replicas?.max_queue_depth ?? 100
  if (pool.pending.length >= maxQueueDepth) return false
  pool.pending.push({ publication, callerKey, dispatch })
  drainReplicaPool(publication)
  return true
}

export function trackAgentAppReplicaInvocation(
  publication: WorkflowPublicationConfig,
  requestId: string,
  lease: AgentAppReplicaLease,
): void {
  if (!requestId) return
  const pool = replicaPool(publication)
  const tracked: TrackedReplicaLease = {
    release: lease.release,
  }
  const timeoutMs = publication.agent_app?.replicas?.timeout_ms
  if (typeof timeoutMs === "number" && timeoutMs > 0) {
    tracked.timeout = setTimeout(() => {
      releaseAgentAppReplicaInvocation(publication, requestId)
    }, timeoutMs)
    tracked.timeout.unref?.()
  }
  pool.invocationLeases.set(requestId, tracked)
}

export function releaseAgentAppReplicaInvocation(
  publication: WorkflowPublicationConfig,
  requestId: string | null | undefined,
): void {
  if (!requestId) return
  const pool = replicaPool(publication)
  const tracked = pool.invocationLeases.get(requestId)
  if (!tracked) return
  pool.invocationLeases.delete(requestId)
  if (tracked.timeout) clearTimeout(tracked.timeout)
  tracked.release()
}

export function agentAppReplicaStatus(publication: WorkflowPublicationConfig): AgentAppReplicaStatus {
  const sessions = normalizedReplicaSessions(publication)
  const pool = replicaPool(publication)
  const activeReplicaCount = sessions.filter((sessionId) => (pool.busyCounts.get(sessionId) ?? 0) > 0).length
  return {
    totalReplicaCount: sessions.length,
    activeReplicaCount,
    readyReplicaCount: Math.max(0, sessions.length - activeReplicaCount),
    queueDepth: pool.pending.length,
  }
}

function normalizedReplicaSessions(publication: WorkflowPublicationConfig): string[] {
  const sessions = publication.replica_session_ids?.length
    ? publication.replica_session_ids
    : [publication.session_id]
  return [...new Set(sessions.filter(Boolean))]
}

function replicaPool(publication: WorkflowPublicationConfig): ReplicaPoolState {
  const publicationId = publication.publication_id
  const existing = replicaPools.get(publicationId)
  if (existing) return existing
  const created: ReplicaPoolState = loadPersistedReplicaPool(publication) ?? {
    nextIndex: 0,
    callerSessions: new Map(),
    busyCounts: new Map(),
    pending: [],
    invocationLeases: new Map(),
    ...optionalStateFile(publication),
  }
  replicaPools.set(publicationId, created)
  return created
}

function nextIdleSession(sessions: string[], pool: ReplicaPoolState): string | null {
  for (let offset = 0; offset < sessions.length; offset += 1) {
    const index = (pool.nextIndex + offset) % sessions.length
    const sessionId = sessions[index]
    if (!sessionId || (pool.busyCounts.get(sessionId) ?? 0) > 0) continue
    pool.nextIndex = (index + 1) % sessions.length
    return sessionId
  }
  return null
}

function leaseReplica(
  publication: WorkflowPublicationConfig,
  sessionId: string,
  pool: ReplicaPoolState,
): AgentAppReplicaLease {
  pool.busyCounts.set(sessionId, (pool.busyCounts.get(sessionId) ?? 0) + 1)
  let released = false
  return {
    publication: { ...publication, session_id: sessionId },
    release: () => {
      if (released) return
      released = true
      const nextCount = Math.max(0, (pool.busyCounts.get(sessionId) ?? 0) - 1)
      if (nextCount === 0) {
        pool.busyCounts.delete(sessionId)
      } else {
        pool.busyCounts.set(sessionId, nextCount)
      }
      queueMicrotask(() => drainReplicaPool(publication))
    },
  }
}

function drainReplicaPool(publication: WorkflowPublicationConfig): void {
  const pool = replicaPool(publication)
  while (pool.pending.length > 0) {
    let dispatched = false
    for (let index = 0; index < pool.pending.length; index += 1) {
      const pending = pool.pending[index]
      if (!pending) continue
      const sessions = normalizedReplicaSessions(pending.publication)
      const lease = acquireIdleAgentAppReplica(
        pending.publication,
        pending.callerKey,
        sessions,
        pool,
      )
      if (!lease) continue
      pool.pending.splice(index, 1)
      void pending.dispatch(lease).catch(() => lease.release())
      dispatched = true
      break
    }
    if (!dispatched) return
  }
}

type PersistedReplicaPool = {
  readonly schema_version: 1
  readonly next_index?: number
  readonly caller_sessions?: Record<string, string>
}

function loadPersistedReplicaPool(publication: WorkflowPublicationConfig): ReplicaPoolState | null {
  const stateFile = stateFileForPublication(publication)
  if (!stateFile || !existsSync(stateFile)) return null
  try {
    const persisted = JSON.parse(readFileSync(stateFile, "utf8")) as PersistedReplicaPool
    if (persisted.schema_version !== 1) return null
    return {
      nextIndex: typeof persisted.next_index === "number" && Number.isInteger(persisted.next_index) && persisted.next_index >= 0
        ? persisted.next_index
        : 0,
      callerSessions: new Map(Object.entries(persisted.caller_sessions ?? {})),
      busyCounts: new Map(),
      pending: [],
      invocationLeases: new Map(),
      stateFile,
    }
  } catch {
    return {
      nextIndex: 0,
      callerSessions: new Map(),
      busyCounts: new Map(),
      pending: [],
      invocationLeases: new Map(),
      stateFile,
    }
  }
}

function persistReplicaPool(publication: WorkflowPublicationConfig, pool: ReplicaPoolState): void {
  if (!pool.stateFile) return
  const persisted: PersistedReplicaPool = {
    schema_version: 1,
    next_index: pool.nextIndex,
    caller_sessions: Object.fromEntries(pool.callerSessions.entries()),
  }
  mkdirSync(dirname(pool.stateFile), { recursive: true })
  const temporary = `${pool.stateFile}.${process.pid}.tmp`
  writeFileSync(temporary, `${JSON.stringify(persisted, null, 2)}\n`, "utf8")
  renameSync(temporary, pool.stateFile)
}

function optionalStateFile(publication: WorkflowPublicationConfig): { readonly stateFile?: string } {
  const stateFile = stateFileForPublication(publication)
  return stateFile ? { stateFile } : {}
}

function stateFileForPublication(publication: WorkflowPublicationConfig): string | null {
  const root = process.env.ARROBA_PUBLICATION_RUNTIME_STATE_DIR
    || (publication.package_root ? join(publication.package_root, ".arroba-publication-runtime") : null)
  if (!root) return null
  return join(root, "agent-app", safeStateSegment(publication.publication_id), "replicas.json")
}

function safeStateSegment(value: string): string {
  return value.replace(/[^a-zA-Z0-9_.-]/g, "_") || "publication"
}

function firstHeader(value: string | string[] | undefined): string | null {
  const raw = Array.isArray(value) ? value[0] : value
  return typeof raw === "string" && raw.trim() ? raw.trim() : null
}

function agentAppSessionCookie(headers: Record<string, string | string[] | undefined>): string | null {
  const cookieHeader = firstHeader(headers.cookie)
  if (!cookieHeader) return null
  for (const part of cookieHeader.split(";")) {
    const [rawName, ...rawValue] = part.trim().split("=")
    if (rawName !== AGENT_APP_SESSION_COOKIE) continue
    const value = rawValue.join("=")
    if (!value) return null
    try {
      return decodeURIComponent(value)
    } catch {
      return value
    }
  }
  return null
}
