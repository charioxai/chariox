import type {
  AgentPromptState,
  RuntimeSession,
} from "./kernel-types.js"

export type SessionAgentPromptStateLike = {
  readonly active_prompt?: unknown | null
  readonly queued_prompts?: readonly unknown[] | null
}

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

export function agentPromptStateHasWork(state: SessionAgentPromptStateLike | null | undefined): boolean {
  return Boolean(state?.active_prompt) || Boolean(state?.queued_prompts?.length)
}

function normalizeAgentPromptState(state: Partial<AgentPromptState> | null | undefined): AgentPromptState {
  return {
    active_prompt: state?.active_prompt ?? null,
    queued_prompts: Array.isArray(state?.queued_prompts) ? state.queued_prompts : [],
  }
}
