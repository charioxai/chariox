import type { PromptQueueItem, RuntimeSession } from "./kernel-types.js"
import { agentRuntimeActivityIsBusy } from "./agent-activity.js"

export function sessionAgentIsBusy(session: RuntimeSession | null | undefined, agentId: string | null | undefined): boolean {
  if (!session || !agentId) {
    return false
  }
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return false
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    return agentRuntimeActivityIsBusy(projected)
  }
  const promptState = session.prompt_states?.[agentId]
  return Boolean(promptState?.active_prompt)
    || Boolean(promptState?.queued_prompts?.length)
    || Boolean(session.active_prompt?.target_agent_id === agentId)
    || Boolean(session.queued_prompts.some((prompt) => prompt.target_agent_id === agentId))
}

export function sessionHasActivePrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return false
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    if (projected.active_turn) {
      return projected.active_turn.prompt_id === promptId
    }
    if (!projected.busy && projected.prompt_status === "none") {
      return false
    }
    const prompt = legacyPromptForAgent(session, agentId)
    return prompt ? prompt.id === promptId : false
  }
  return legacySessionHasPrompt(session, agentId, promptId)
}

export function sessionPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return null
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    if (!projected.busy && projected.prompt_status === "none" && !projected.active_turn) {
      return null
    }
    const prompt = legacyPromptForAgent(session, agentId)
    const activeTurnPromptId = projected.active_turn?.prompt_id
    if (activeTurnPromptId && prompt?.id !== activeTurnPromptId) {
      return null
    }
    return prompt
  }
  return legacyPromptForAgent(session, agentId)
}

function legacySessionHasPrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  const promptState = session.prompt_states?.[agentId]
  return promptState?.active_prompt?.id === promptId
    || Boolean(promptState?.queued_prompts?.some((prompt) => prompt.id === promptId))
    || Boolean(session.active_prompt?.target_agent_id === agentId && session.active_prompt.id === promptId)
    || session.queued_prompts.some((prompt) => prompt.target_agent_id === agentId && prompt.id === promptId)
}

function legacyPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const promptState = session.prompt_states?.[agentId]
  return promptState?.active_prompt
    ?? promptState?.queued_prompts?.[promptState.queued_prompts.length - 1]
    ?? (session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null)
    ?? [...session.queued_prompts].reverse().find((prompt) => prompt.target_agent_id === agentId)
    ?? null
}
