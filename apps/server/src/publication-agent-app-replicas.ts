import type {
  WorkflowPublicationConfig,
} from "./publication-types.js"

type ReplicaPoolState = {
  nextIndex: number
  callerSessions: Map<string, string>
}

const replicaPools = new Map<string, ReplicaPoolState>()
const AGENT_APP_SESSION_COOKIE = "arroba_agent_app_session"

export type AgentAppCallerSession = {
  readonly callerKey: string
  readonly setCookie?: string
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

export function selectAgentAppReplica(
  publication: WorkflowPublicationConfig,
  callerKey: string,
): WorkflowPublicationConfig {
  const sessions = normalizedReplicaSessions(publication)
  if (sessions.length <= 1 || publication.agent_app?.replicas?.count === undefined) {
    return publication
  }
  const pool = replicaPool(publication.publication_id)
  if (publication.agent_app.replicas.per_caller_ordering !== false) {
    const existing = pool.callerSessions.get(callerKey)
    if (existing && sessions.includes(existing)) {
      return { ...publication, session_id: existing }
    }
  }
  const sessionId = sessions[pool.nextIndex % sessions.length] ?? publication.session_id
  pool.nextIndex += 1
  if (publication.agent_app.replicas.per_caller_ordering !== false) {
    pool.callerSessions.set(callerKey, sessionId)
  }
  return { ...publication, session_id: sessionId }
}

function normalizedReplicaSessions(publication: WorkflowPublicationConfig): string[] {
  const sessions = publication.replica_session_ids?.length
    ? publication.replica_session_ids
    : [publication.session_id]
  return [...new Set(sessions.filter(Boolean))]
}

function replicaPool(publicationId: string): ReplicaPoolState {
  const existing = replicaPools.get(publicationId)
  if (existing) return existing
  const created: ReplicaPoolState = { nextIndex: 0, callerSessions: new Map() }
  replicaPools.set(publicationId, created)
  return created
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
