import type { AgentInstance, PromptQueueItem, RuntimeSession } from "./kernel-types.js"
import { agentRuntimeActivityIsBusy } from "./agent-activity.js"

export type SessionPromptWorkSummary = {
  readonly active: number
  readonly queued: number
  readonly busyAgents: number
}

export function sessionPromptWorkSummary(session: RuntimeSession): SessionPromptWorkSummary {
  const promptStates = session.prompt_states
  const queued = promptStates
    ? Object.values(promptStates).reduce((count, state) => count + (state?.queued_prompts?.length ?? 0), 0)
    : session.queued_prompts.length

  if (session.agent_activity) {
    const activities = Object.entries(session.agent_activity)
    return {
      active: activities.filter(([agentId, activity]) =>
        agentRuntimeActivityHasActivePrompt(activity, promptStates?.[agentId]),
      ).length,
      queued,
      busyAgents: activities.filter(([, activity]) => agentRuntimeActivityIsBusy(activity)).length,
    }
  }

  if (promptStates) {
    return {
      active: Object.values(promptStates).filter((state) => Boolean(state?.active_prompt)).length,
      queued,
      busyAgents: legacyBusyAgentCount(session),
    }
  }

  return {
    active: session.active_prompt ? 1 : 0,
    queued,
    busyAgents: legacyBusyAgentCount(session),
  }
}

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
  const promptState = promptStateForAgent(session, agentId)
  if (promptState !== undefined) {
    return Boolean(promptState?.active_prompt) || Boolean(promptState?.queued_prompts?.length)
  }
  return legacyTopLevelSessionHasPromptWork(session, agentId)
}

export function sessionAgentRuntimeState(
  session: RuntimeSession | null | undefined,
  agent: AgentInstance,
): AgentInstance["state"] {
  if (!session) {
    return legacyAgentRuntimeState(agent)
  }
  if (session.agent_activity && !(agent.id in session.agent_activity)) {
    return agent.state === "Error" ? "Error" : "Idle"
  }
  const projected = session.agent_activity?.[agent.id]
  if (projected) {
    if (projected.status === "error") {
      return "Error"
    }
    return agentRuntimeActivityIsBusy(projected) ? "Working" : "Idle"
  }
  const promptState = promptStateForAgent(session, agent.id)
  if (promptState !== undefined) {
    if (agent.state === "Error") {
      return "Error"
    }
    return Boolean(promptState?.active_prompt) || Boolean(promptState?.queued_prompts?.length)
      ? "Working"
      : "Idle"
  }
  return legacyAgentRuntimeState(agent)
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
    if (!agentRuntimeActivityIsBusy(projected)) {
      return false
    }
  }
  return legacySessionHasPrompt(session, agentId, promptId)
}

export function sessionPromptForAgent(session: RuntimeSession, agentId: string): PromptQueueItem | null {
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return null
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    const activeTurnPromptId = projected.active_turn?.prompt_id
    if (activeTurnPromptId) {
      const prompt = legacyPromptForAgent(session, agentId)
      return prompt?.id === activeTurnPromptId ? prompt : null
    }
    if (!agentRuntimeActivityIsBusy(projected)) {
      return null
    }
  }
  return legacyPromptForAgent(session, agentId)
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

function promptStateForAgent(session: RuntimeSession, agentId: string) {
  const promptStates = session.prompt_states
  if (!promptStates || !Object.prototype.hasOwnProperty.call(promptStates, agentId)) {
    return undefined
  }
  return promptStates[agentId] ?? null
}

function legacyTopLevelSessionHasPromptWork(session: RuntimeSession, agentId: string): boolean {
  return Boolean(session.active_prompt?.target_agent_id === agentId)
    || Boolean(session.queued_prompts.some((prompt) => prompt.target_agent_id === agentId))
}

function agentRuntimeActivityHasActivePrompt(
  activity: NonNullable<RuntimeSession["agent_activity"]>[string],
  promptState?: NonNullable<RuntimeSession["prompt_states"]>[string],
): boolean {
  if (activity.active_turn) {
    return activity.active_turn.status !== "none" && activity.active_turn.status !== "queued"
  }
  if (agentRuntimeActivityIsBusy(activity) && promptState?.active_prompt) {
    return true
  }
  return activity.prompt_status === "running"
    || activity.prompt_status === "cancelling"
    || activity.prompt_status === "settling"
}

function legacyBusyAgentCount(session: RuntimeSession): number {
  return session.agents.filter((agent) => agent.is_processing || agent.state === "Working").length
}

function legacyAgentRuntimeState(agent: AgentInstance): AgentInstance["state"] {
  return agent.is_processing && agent.state !== "Error" ? "Working" : agent.state
}
