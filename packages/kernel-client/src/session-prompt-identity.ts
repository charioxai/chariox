import type {
  AgentPromptState,
  PromptQueueItem,
  RuntimeSession,
} from "./kernel-types.js"
import {
  sessionActivePromptLifecycleRecords,
} from "./session-prompt-lifecycle.js"
import {
  sessionHasAgent,
  sessionProjectedPromptActivityForAgent,
  sessionPromptStateRecordForAgent,
} from "./session-agent-prompt-state.js"

export function sessionPromptStateForAgent(
  session: RuntimeSession,
  agentId: string | null | undefined,
): AgentPromptState | null {
  if (!agentId || !sessionHasAgent(session, agentId)) {
    return null
  }
  const projectedPromptState = sessionPromptStateRecordForAgent(session, agentId)
  if (projectedPromptState !== undefined) {
    return projectedPromptState
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
  const projected = sessionProjectedPromptActivityForAgent(session, agentId)
  if (projected === "not_found" || projected === "idle") {
    return false
  }
  if (projected?.activeTurnPromptId) {
    return projected.activeTurnPromptId === promptId
  }
  return legacySessionHasPrompt(session, agentId, promptId)
}

export function sessionPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const projected = sessionProjectedPromptActivityForAgent(session, agentId)
  if (projected === "not_found" || projected === "idle") {
    return null
  }
  if (projected?.activeTurnPromptId) {
    const prompt = legacyPromptForAgent(session, agentId)
    return prompt?.id === projected.activeTurnPromptId ? prompt : null
  }
  return legacyPromptForAgent(session, agentId)
}

export function sessionActivePromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const projected = sessionProjectedPromptActivityForAgent(session, agentId)
  if (projected === "not_found" || projected === "idle") {
    return null
  }
  if (projected?.activeTurnPromptId) {
    const prompt = activePromptForAgent(session, agentId)
    return prompt?.id === projected.activeTurnPromptId ? prompt : null
  }
  return activePromptForAgent(session, agentId)
}

export function sessionActivePromptIdForAgent(
  session: RuntimeSession,
  agentId: string | null | undefined,
): string | null {
  if (agentId) {
    const projected = sessionProjectedPromptActivityForAgent(session, agentId)
    if (projected === "not_found" || projected === "idle") {
      return null
    }
    if (projected?.activeTurnPromptId) {
      return projected.activeTurnPromptId
    }
    return activePromptForAgent(session, agentId)?.id ?? null
  }

  const records = sessionActivePromptLifecycleRecords(session)
  return records.length === 1 ? records[0]?.id ?? null : null
}

function legacySessionHasPrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt?.id === promptId
      || Boolean(promptState?.queued_prompts?.some((prompt) => prompt.id === promptId))
  }
  return Boolean(session.active_prompt?.target_agent_id === agentId && session.active_prompt.id === promptId)
    || session.queued_prompts.some((prompt) => prompt.target_agent_id === agentId && prompt.id === promptId)
}

function legacyPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
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
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt ?? null
  }
  if (session.agent_activity) {
    return null
  }
  return session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null
}
