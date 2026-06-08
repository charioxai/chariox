import type {
  WorkflowPublicationConfig,
} from "./publication-types.js"

type ReplicaPoolState = {
  nextIndex: number
  callerSessions: Map<string, string>
}

const replicaPools = new Map<string, ReplicaPoolState>()

export function agentAppCallerKey(headers: Record<string, string | string[] | undefined>): string {
  const explicit = firstHeader(headers["x-arroba-agent-app-caller"])
  if (explicit) return explicit
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
