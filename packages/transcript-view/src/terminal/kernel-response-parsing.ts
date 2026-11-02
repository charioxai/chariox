export function variantPayload<T>(value: Record<string, unknown>, variant: string): T | null {
  const payload = value[variant]
  return payload && typeof payload === "object" ? payload as T : null
}

export function sessionStatePayload(value: Record<string, unknown>): Record<string, unknown> | null {
  const payload = variantPayload<{
    session?: Record<string, unknown>
    room_workflows?: unknown
    agent_activity: Record<string, unknown>
    agent_activity_revision?: number | null
  }>(value, "SessionState")
  if (
    !payload?.session
  ) {
    return null
  }
  return mergeAgentActivityIntoSession({ ...payload.session, ...(payload.room_workflows ? { room_workflows: payload.room_workflows, room_workflows_fresh: true } : {}) }, payload.agent_activity, payload.agent_activity_revision)
}

export function promptSubmittedSessionPayload(value: Record<string, unknown>): Record<string, unknown> | null {
  const payload = variantPayload<{
    session?: Record<string, unknown>
    agent_activity: Record<string, unknown>
    agent_activity_revision?: number | null
  }>(value, "PromptSubmitted")
  if (
    !payload?.session
  ) {
    return null
  }
  return mergeAgentActivityIntoSession(payload.session, payload.agent_activity, payload.agent_activity_revision)
}

export function mergeAgentActivityIntoSession(
  session: Record<string, unknown>,
  agentActivity: unknown,
  revision?: unknown,
): Record<string, unknown> | null {
  if (!agentActivity || typeof agentActivity !== "object" || Array.isArray(agentActivity)) {
    return null
  }
  const projectedSession = { ...session, agent_activity: agentActivity as Record<string, unknown> }
  return typeof revision === "number" && Number.isFinite(revision)
    ? { ...projectedSession, agent_activity_revision: revision }
    : projectedSession
}

export function providerRunPayload(response: Record<string, unknown>): Record<string, unknown> | null {
  const launched = variantPayload<{ provider_run?: Record<string, unknown> }>(
    response,
    "ProviderRunLaunched",
  )?.provider_run
  if (launched) {
    return launched
  }
  return variantPayload<{ provider_run?: Record<string, unknown> }>(
    response,
    "ProviderRunLaunchAccepted",
  )?.provider_run
    ?? variantPayload<{ provider_run?: Record<string, unknown> }>(
      response,
      "ExternalProviderSessionImported",
    )?.provider_run
    ?? variantPayload<{ provider_run?: Record<string, unknown> }>(
      response,
      "ExternalProviderAgentImported",
    )?.provider_run
    ?? null
}

export function readStringField(value: unknown, field: string): string | null {
  if (!value || typeof value !== "object") {
    return null
  }
  const candidate = (value as Record<string, unknown>)[field]
  return typeof candidate === "string" && candidate.length > 0 ? candidate : null
}

export function readRecordField(value: unknown, field: string): Record<string, unknown> | null {
  if (!value || typeof value !== "object") {
    return null
  }
  const candidate = (value as Record<string, unknown>)[field]
  return candidate && typeof candidate === "object" && !Array.isArray(candidate)
    ? candidate as Record<string, unknown>
    : null
}

export function readNumberField(value: unknown, field: string): number | null {
  if (!value || typeof value !== "object") {
    return null
  }
  const candidate = (value as Record<string, unknown>)[field]
  return typeof candidate === "number" && Number.isFinite(candidate) ? candidate : null
}

export function readBooleanField(value: unknown, field: string): boolean | null {
  if (!value || typeof value !== "object") {
    return null
  }
  const candidate = (value as Record<string, unknown>)[field]
  return typeof candidate === "boolean" ? candidate : null
}

export function readStringArrayField(value: unknown, field: string): string[] | null {
  if (!value || typeof value !== "object") {
    return null
  }
  const candidate = (value as Record<string, unknown>)[field]
  return Array.isArray(candidate)
    ? candidate.filter((item): item is string => typeof item === "string" && item.length > 0)
    : null
}

export function requireMatchingSessionId(
  session: unknown,
  expectedSessionId: string,
  label: string,
): string {
  const responseSessionId = readStringField(session, "id")
  if (!responseSessionId) {
    throw new Error(`kernel returned ${label} without session id, expected ${expectedSessionId}`)
  }
  if (responseSessionId !== expectedSessionId) {
    throw new Error(`kernel returned ${label} for session ${responseSessionId}, expected ${expectedSessionId}`)
  }
  return responseSessionId
}

export function validateMatchingSessionIdIfPresent(
  session: unknown,
  expectedSessionId: string,
  label: string,
  field = "id",
): string | null {
  const responseSessionId = readStringField(session, field)
  if (responseSessionId && responseSessionId !== expectedSessionId) {
    throw new Error(`kernel returned ${label} for session ${responseSessionId}, expected ${expectedSessionId}`)
  }
  return responseSessionId
}

export function validateMatchingAgentIdIfPresent(
  agent: unknown,
  expectedAgentId: string,
  label: string,
): string | null {
  const responseAgentId = readStringField(agent, "id")
  if (responseAgentId && responseAgentId !== expectedAgentId) {
    throw new Error(`kernel returned ${label} agent ${responseAgentId}, expected ${expectedAgentId}`)
  }
  return responseAgentId
}

export function validateMatchingAgentSessionIdIfPresent(
  agent: unknown,
  expectedSessionId: string,
  expectedAgentId: string,
  label: string,
): string | null {
  const responseSessionId = readStringField(agent, "session_id")
  if (responseSessionId && responseSessionId !== expectedSessionId) {
    throw new Error(`kernel returned ${label} agent ${expectedAgentId} for session ${responseSessionId}, expected ${expectedSessionId}`)
  }
  return responseSessionId
}

export function firstSessionAgentId(session: unknown): string | null {
  if (!session || typeof session !== "object") {
    return null
  }
  const focused = readStringField(session, "focused_agent_id")
  const agents = (session as { agents?: unknown }).agents
  if (!Array.isArray(agents)) {
    return focused
  }
  if (focused && sessionHasAgentId(session, focused)) {
    return focused
  }
  if (focused) {
    return null
  }
  return readStringField(agents[0], "id")
}

/** The session's focus agent, or null when it has none (zero focus). */
export function sessionFocusAgentId(session: unknown): string | null {
  const focused = readStringField(session, "focused_agent_id")
  return focused && sessionHasAgentId(session, focused) ? focused : null
}

export function sessionAgentIdOrFirst(session: unknown, requestedAgentId: string | null | undefined): string | null {
  if (requestedAgentId && sessionHasAgentId(session, requestedAgentId)) {
    return requestedAgentId
  }
  return firstSessionAgentId(session)
}

export function sessionHasAgentId(session: unknown, agentId: string | null | undefined): boolean {
  if (!agentId || !session || typeof session !== "object") {
    return false
  }
  const agents = (session as { agents?: unknown }).agents
  return Array.isArray(agents) && agents.some((agent) => readStringField(agent, "id") === agentId)
}

export function sessionHasAgentIdentity(session: unknown, agentIdentity: string | null | undefined): boolean {
  if (!agentIdentity || !session || typeof session !== "object") {
    return false
  }
  const agents = (session as { agents?: unknown }).agents
  return Array.isArray(agents) && agents.some((agent) => (
    readStringField(agent, "id") === agentIdentity
    || readStringField(agent, "agent_ref") === agentIdentity
  ))
}
