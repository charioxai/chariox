import type {
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

export function sessionProjectedPromptActivityEntriesForSessionAgents(
  session: RuntimeSession,
): readonly (readonly [string, AgentRuntimeActivityProjection])[] {
  if (!session.agent_activity) {
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

export function agentPromptStateHasWork(state: SessionAgentPromptStateLike | null | undefined): boolean {
  return Boolean(state?.active_prompt) || Boolean(state?.queued_prompts?.length)
}

export function sessionProjectedPromptActivityForAgent(
  session: RuntimeSession,
  agentId: string,
): SessionProjectedPromptActivity {
  if (!sessionHasAgent(session, agentId)) {
    return "not_found"
  }
  if (!session.agent_activity) {
    return null
  }
  const activity = session.agent_activity[agentId]
  if (!activity) {
    return "not_found"
  }
  const projection = projectAgentRuntimeActivity(activity)
  if (!projection.activeTurnPromptId && !projection.busy) {
    return "idle"
  }
  return projection
}
