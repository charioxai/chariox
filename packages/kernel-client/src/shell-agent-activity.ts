import type { RuntimeSession } from "./kernel-types.js"

export function sessionAgentIsBusy(session: RuntimeSession | null | undefined, agentId: string | null | undefined): boolean {
  if (!session || !agentId) {
    return false
  }
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return false
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    return projected.busy || projected.status === "working" || projected.prompt_status !== "none" || Boolean(projected.active_turn)
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
    const hasLegacyPromptState = Boolean(session.prompt_states?.[agentId])
      || Boolean(session.active_prompt)
      || session.queued_prompts.length > 0
    return hasLegacyPromptState ? legacySessionHasPrompt(session, agentId, promptId) : true
  }
  return legacySessionHasPrompt(session, agentId, promptId)
}

function legacySessionHasPrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  const promptState = session.prompt_states?.[agentId]
  return promptState?.active_prompt?.id === promptId
    || Boolean(promptState?.queued_prompts?.some((prompt) => prompt.id === promptId))
    || session.active_prompt?.id === promptId
    || session.queued_prompts.some((prompt) => prompt.id === promptId)
}
