import type {
  AgentPromptState,
  PromptQueueItem,
  RuntimeSession,
} from "./kernel-types.js"
import {
  sessionActivePromptLifecycleRecords,
} from "./session-prompt-lifecycle.js"
import {
  sessionAgentActivityRecordForAgent,
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
  if (sessionAgentActivityRecordForAgent(session, agentId) !== undefined) {
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

export function sessionHasPendingPrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  const projected = sessionProjectedPromptActivityForAgent(session, agentId)
  if (projected === "not_found" || projected === "idle") {
    return false
  }
  if (projected?.activeTurnPromptId) {
    if (projected.activeTurnPromptId === promptId) {
      return true
    }
    const promptState = sessionPromptStateRecordForAgent(session, agentId)
    if (promptState !== undefined) {
      return Boolean(promptState?.queued_prompts?.some((prompt) => promptMatchesId(prompt, promptId)))
    }
    return false
  }
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptMatchesId(promptState?.active_prompt, promptId)
      || Boolean(promptState?.queued_prompts?.some((prompt) => promptMatchesId(prompt, promptId)))
  }
  if (projected) {
    return false
  }
  return legacySessionHasPrompt(session, agentId, promptId)
    || session.queued_prompts.some((prompt) => prompt.target_agent_id === agentId && promptMatchesId(prompt, promptId))
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
    return promptMatchesId(promptState?.active_prompt, promptId)
  }
  return Boolean(session.active_prompt?.target_agent_id === agentId && promptMatchesId(session.active_prompt, promptId))
}

function legacyPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt ?? null
  }
  return session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null
}

function activePromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt ?? null
  }
  if (sessionAgentActivityRecordForAgent(session, agentId) !== undefined) {
    return null
  }
  return session.active_prompt?.target_agent_id === agentId ? session.active_prompt : null
}

function promptMatchesId(prompt: PromptQueueItem | null | undefined, promptId: string): boolean {
  return prompt?.id === promptId || prompt?.pending_prompt_id === promptId
}
