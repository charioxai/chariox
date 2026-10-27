import type {
  AgentRuntimeActivity,
  AgentPromptState,
  RuntimeSession,
} from "./kernel-types.js"
import {
  projectAgentRuntimeActivity,
  type AgentRuntimeActivityProjection,
} from "./agent-activity.js"
import { normalizeAgentPromptState } from "./runtime-session-normalization.js"

export type SessionAgentPromptStateLike = {
  readonly active_prompt?: unknown | null
  readonly queued_prompts?: readonly unknown[] | null
}

export type SessionProjectedPromptActivity =
  | AgentRuntimeActivityProjection
  | "idle"
  | "not_found"
  | null

export function sessionHasAgent(session: RuntimeSession, agentId: string): boolean {
  return sessionAgentIds(session).has(agentId)
}

export function sessionAgentIds(session: RuntimeSession): ReadonlySet<string> {
  return new Set(session.agents.map((agent) => agent.id))
}

export function sessionHasPromptStateProjection(session: RuntimeSession): boolean {
  return Boolean(session.prompt_states)
}

export function sessionHasAgentActivityProjection(session: RuntimeSession): boolean {
  return Boolean(session.agent_activity)
}

export function sessionPromptStateRecordForAgent(
  session: RuntimeSession,
  agentId: string,
): AgentPromptState | null | undefined {
  const promptStates = session.prompt_states
  if (!promptStates) {
    return undefined
  }
  if (!Object.prototype.hasOwnProperty.call(promptStates, agentId)) {
    return null
  }
  return normalizeAgentPromptState(promptStates[agentId])
}

export function sessionPromptStateEntriesForSessionAgents(
  session: RuntimeSession,
): readonly (readonly [string, AgentPromptState])[] {
  const promptStates = session.prompt_states
  if (!promptStates) {
    return []
  }
  const agentIds = sessionAgentIds(session)
  return Object.entries(promptStates)
    .filter(([agentId]) => agentIds.has(agentId))
    .map(([agentId, state]) => [agentId, normalizeAgentPromptState(state)] as const)
}

export function sessionAgentActivityRecordForAgent(
  session: RuntimeSession,
  agentId: string,
): AgentRuntimeActivity | null | undefined {
  if (!sessionHasAgent(session, agentId)) {
    return null
  }
  if (!sessionHasAgentActivityProjection(session)) {
    return undefined
  }
  const activityByAgent = session.agent_activity
  return activityByAgent?.[agentId] ?? null
}

export function sessionProjectedPromptActivityEntriesForSessionAgents(
  session: RuntimeSession,
): readonly (readonly [string, AgentRuntimeActivityProjection])[] {
  if (!sessionHasAgentActivityProjection(session)) {
    return []
  }
  const entries: (readonly [string, AgentRuntimeActivityProjection])[] = []
  for (const agent of session.agents) {
    const projection = sessionProjectedPromptActivityForAgent(session, agent.id)
    if (!projection || projection === "idle" || projection === "not_found") {
      continue
    }
    entries.push([agent.id, projection])
  }
  return entries
}

export function sessionProjectedPromptActivityForAgent(
  session: RuntimeSession,
  agentId: string,
): SessionProjectedPromptActivity {
  if (!sessionHasAgent(session, agentId)) {
    return "not_found"
  }
  if (!sessionHasAgentActivityProjection(session)) {
    return null
  }
  const activity = sessionAgentActivityRecordForAgent(session, agentId)
  if (!activity) {
    return "not_found"
  }
  const projection = projectAgentRuntimeActivity(activity)
  if (
    !projection.activeTurnPromptId
    && !projection.busy
    && !projection.error
    && !projection.unreadIdleOutput
    && projection.promptStatus === "none"
    && projection.queuedPromptCount === 0
  ) {
    return "idle"
  }
  return projection
}
