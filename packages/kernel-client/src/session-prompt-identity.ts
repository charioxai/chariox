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
  return null
}

export function sessionHasActivePrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  const projected = sessionProjectedPromptActivityForAgent(session, agentId)
  if (projected === "not_found" || projected === "idle") {
    return false
  }
  if (projected?.activeTurnPromptId) {
    if (projected.activeTurnPromptId === promptId) {
      return true
    }
    const prompt = activePromptForAgent(session, agentId)
    return promptMatchesId(prompt, projected.activeTurnPromptId)
      && promptMatchesId(prompt, promptId)
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
  return false
}

export function sessionPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const projected = sessionProjectedPromptActivityForAgent(session, agentId)
  if (projected === "not_found" || projected === "idle") {
    return null
  }
  if (projected?.activeTurnPromptId) {
    const prompt = legacyPromptForAgent(session, agentId)
    return promptMatchesId(prompt, projected.activeTurnPromptId) ? prompt : null
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
    return promptMatchesId(prompt, projected.activeTurnPromptId) ? prompt : null
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
  return false
}

function legacyPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt ?? null
  }
  return null
}

function activePromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.active_prompt ?? null
  }
  return null
}

function promptMatchesId(prompt: PromptQueueItem | null | undefined, promptId: string): boolean {
  return prompt?.id === promptId || prompt?.pending_prompt_id === promptId
}
