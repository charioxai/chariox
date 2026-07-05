import type {
  AgentPromptState,
  PromptQueueItem,
  RuntimeSession,
} from "./kernel-types.js"
import {
  projectAgentRuntimeActivity,
} from "./agent-activity.js"
import {
  sessionActivePromptLifecycleRecords,
} from "./session-prompt-lifecycle.js"

export function sessionPromptStateForAgent(
  session: RuntimeSession,
  agentId: string | null | undefined,
): AgentPromptState | null {
  if (!agentId) {
    return null
  }
  const promptStates = session.prompt_states
  if (promptStates) {
    return Object.prototype.hasOwnProperty.call(promptStates, agentId)
      ? normalizeAgentPromptState(promptStates[agentId])
      : null
  }
  if (session.agent_activity) {
    return null
  }
  const activePrompt = session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null
  const queuedPrompts = session.queued_prompts.filter((prompt) => prompt.target_agent_id === agentId)
  if (activePrompt || queuedPrompts.length > 0) {
    return {
      active_prompt: activePrompt,
      queued_prompts: queuedPrompts,
    }
  }
  return null
}

export function sessionHasActivePrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  if (!sessionHasAgent(session, agentId)) {
    return false
  }
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return false
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    const projection = projectAgentRuntimeActivity(projected)
    if (projection.activeTurnPromptId) {
      return projection.activeTurnPromptId === promptId
    }
    if (!projection.busy) {
      return false
    }
  }
  return legacySessionHasPrompt(session, agentId, promptId)
}

export function sessionPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  if (!sessionHasAgent(session, agentId)) {
    return null
  }
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return null
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    const projection = projectAgentRuntimeActivity(projected)
    const activeTurnPromptId = projection.activeTurnPromptId ?? null
    if (activeTurnPromptId) {
      const prompt = legacyPromptForAgent(session, agentId)
      return prompt?.id === activeTurnPromptId ? prompt : null
    }
    if (!projection.busy) {
      return null
    }
  }
  return legacyPromptForAgent(session, agentId)
}

export function sessionActivePromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  if (!sessionHasAgent(session, agentId)) {
    return null
  }
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return null
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    const projection = projectAgentRuntimeActivity(projected)
    const activeTurnPromptId = projection.activeTurnPromptId ?? null
    if (activeTurnPromptId) {
      const prompt = activePromptForAgent(session, agentId)
      return prompt?.id === activeTurnPromptId ? prompt : null
    }
    if (!projection.busy) {
      return null
    }
  }
  return activePromptForAgent(session, agentId)
}

export function sessionActivePromptIdForAgent(
  session: RuntimeSession,
  agentId: string | null | undefined,
): string | null {
  if (agentId) {
    const projected = session.agent_activity?.[agentId]
    const projection = projectAgentRuntimeActivity(projected)
    const projectedPromptId = projection.activeTurnPromptId ?? null
    if (projectedPromptId) {
      return projectedPromptId
    }
    if (session.agent_activity && !projection.busy) {
      return null
    }
    return activePromptForAgent(session, agentId)?.id ?? null
  }

  const records = sessionActivePromptLifecycleRecords(session)
  return records.length === 1 ? records[0]?.id ?? null : null
}

function legacySessionHasPrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  const promptState = promptStateForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt?.id === promptId
      || Boolean(promptState?.queued_prompts?.some((prompt) => prompt.id === promptId))
  }
  return Boolean(session.active_prompt?.target_agent_id === agentId && session.active_prompt.id === promptId)
    || session.queued_prompts.some((prompt) => prompt.target_agent_id === agentId && prompt.id === promptId)
}

function legacyPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const promptState = promptStateForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt
      ?? promptState?.queued_prompts?.[promptState.queued_prompts.length - 1]
      ?? null
  }
  return (session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null)
    ?? [...session.queued_prompts].reverse().find((prompt) => prompt.target_agent_id === agentId)
    ?? null
}

function activePromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const promptState = promptStateForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt ?? null
  }
  if (session.agent_activity) {
    return null
  }
  return session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null
}

function promptStateForAgent(session: RuntimeSession, agentId: string) {
  const promptStates = session.prompt_states
  if (!promptStates) {
    return undefined
  }
  if (!Object.prototype.hasOwnProperty.call(promptStates, agentId)) {
    return null
  }
  return promptStates[agentId] ?? null
}

function normalizeAgentPromptState(state: Partial<AgentPromptState> | null | undefined): AgentPromptState {
  return {
    active_prompt: state?.active_prompt ?? null,
    queued_prompts: Array.isArray(state?.queued_prompts) ? state.queued_prompts : [],
  }
}

function sessionHasAgent(session: RuntimeSession, agentId: string): boolean {
  return sessionAgentIds(session).has(agentId)
}

function sessionAgentIds(session: RuntimeSession): ReadonlySet<string> {
  return new Set(session.agents.map((agent) => agent.id))
}
