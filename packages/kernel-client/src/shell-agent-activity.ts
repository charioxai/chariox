import type { AgentInstance, PromptQueueItem, RuntimeSession } from "./kernel-types.js"
import {
  agentRuntimeActiveTurnIsBusy,
  agentRuntimeActivityIsBusy,
  agentRuntimePromptStatusIsActivePrompt,
  normalizeAgentRuntimeActivityStatus,
  normalizeAgentRuntimePromptStatus,
  projectAgentRuntimeActivity,
} from "./agent-activity.js"
import type { AgentRuntimeActivityBusyInput } from "./agent-activity.js"

export type SessionPromptWorkSummary = {
  readonly active: number
  readonly queued: number
  readonly busyAgents: number
}

export type AgentPromptStateLike = {
  readonly active_prompt?: unknown | null
  readonly queued_prompts?: readonly unknown[] | null
}

export type AgentRuntimeProjectionContext = {
  readonly agentActivity?: Record<string, AgentRuntimeActivityBusyInput> | null | undefined
  readonly promptStates?: Record<string, AgentPromptStateLike | null> | null | undefined
}

export type AgentRuntimeDisplayState = AgentInstance["state"] | "Done"

export function sessionPromptWorkSummary(session: RuntimeSession): SessionPromptWorkSummary {
  const promptStates = session.prompt_states
  const queued = promptStates
    ? Object.values(promptStates).reduce((count, state) => count + (state?.queued_prompts?.length ?? 0), 0)
    : session.queued_prompts.length

  if (session.agent_activity) {
    const activities = Object.entries(session.agent_activity)
    const projectedQueued = promptWorkCountFromProjectedActivities(
      activities.map(([, activity]) => activity),
      "queued_prompt_count",
    )
    return {
      active: activities.reduce(
        (count, [agentId, activity]) =>
          count + projectedActivePromptCount(activity, promptStates?.[agentId]),
        0,
      ),
      queued: projectedQueued ?? queued,
      busyAgents: activities.filter(([, activity]) => agentRuntimeActivityIsBusy(activity)).length,
    }
  }

  if (promptStates) {
    const busyAgents = Object.values(promptStates).filter(promptStateHasWork).length
    return {
      active: Object.values(promptStates).filter((state) => Boolean(state?.active_prompt)).length,
      queued,
      busyAgents,
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
  return agentRuntimeStateFromProjection(agent, {
    agentActivity: session.agent_activity,
    promptStates: session.prompt_states,
  })
}

export function sessionAgentRuntimeDisplayState(
  session: RuntimeSession | null | undefined,
  agent: AgentInstance,
): AgentRuntimeDisplayState {
  const runtimeState = sessionAgentRuntimeState(session, agent)
  return sessionAgentHasUnreadIdleOutput(session, agent.id) && runtimeState === "Idle" ? "Done" : runtimeState
}

export function agentRuntimeStateFromProjection(
  agent: AgentInstance,
  context: AgentRuntimeProjectionContext,
): AgentInstance["state"] {
  if (context.agentActivity && !(agent.id in context.agentActivity)) {
    return agent.state === "Error" ? "Error" : "Idle"
  }
  const projected = context.agentActivity?.[agent.id]
  if (projected) {
    if (normalizeAgentRuntimeActivityStatus(projected.status) === "error") {
      return "Error"
    }
    return agentRuntimeActivityIsBusy(projected) ? "Working" : "Idle"
  }
  const promptState = context.promptStates?.[agent.id]
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

export function sessionAgentHasUnreadIdleOutput(
  session: RuntimeSession | null | undefined,
  agentId: string | null | undefined,
): boolean {
  if (!session || !agentId || session.focused_agent_id === agentId) {
    return false
  }
  const activity = session.agent_activity?.[agentId]
  return activity?.unread_idle_output === true
}

export function sessionHasActivePrompt(session: RuntimeSession, agentId: string, promptId: string): boolean {
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return false
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    const activeTurn = projected.active_turn
    if (agentRuntimeActiveTurnIsBusy(activeTurn)) {
      return activeTurn?.prompt_id === promptId
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
    const activeTurnPromptId = agentRuntimeActiveTurnIsBusy(projected.active_turn)
      ? projected.active_turn?.prompt_id
      : null
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

function promptStateHasWork(state: AgentPromptStateLike | null | undefined): boolean {
  return Boolean(state?.active_prompt) || Boolean(state?.queued_prompts?.length)
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

function legacyTopLevelSessionHasPromptWork(session: RuntimeSession, agentId: string): boolean {
  return Boolean(session.active_prompt?.target_agent_id === agentId)
    || Boolean(session.queued_prompts.some((prompt) => prompt.target_agent_id === agentId))
}

function agentRuntimeActivityHasActivePrompt(
  activity: NonNullable<RuntimeSession["agent_activity"]>[string],
  promptState?: NonNullable<RuntimeSession["prompt_states"]>[string],
): boolean {
  const projection = projectAgentRuntimeActivity(activity)
  if (projection.activeTurn) {
    return projection.activePromptCount > 0
  }
  if (agentRuntimeActivityIsBusy(activity) && promptState?.active_prompt) {
    return true
  }
  return agentRuntimePromptStatusIsActivePrompt(projection.promptStatus)
}

function projectedActivePromptCount(
  activity: NonNullable<RuntimeSession["agent_activity"]>[string],
  promptState?: NonNullable<RuntimeSession["prompt_states"]>[string],
): number {
  const projection = projectAgentRuntimeActivity(activity)
  if (activity.active_prompt_count !== undefined && activity.active_prompt_count !== null) {
    return projection.activePromptCount
  }
  return agentRuntimeActivityHasActivePrompt(activity, promptState) ? 1 : 0
}

function promptWorkCountFromProjectedActivities(
  activities: readonly NonNullable<RuntimeSession["agent_activity"]>[string][],
  field: "queued_prompt_count",
): number | null {
  let count = 0
  for (const activity of activities) {
    const projectedCount = nonNegativeInteger(activity[field])
    if (projectedCount === null) {
      return null
    }
    count += projectedCount
  }
  return count
}

function nonNegativeInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value >= 0 ? value : null
}

function legacyBusyAgentCount(session: RuntimeSession): number {
  return session.agents.filter((agent) => agent.is_processing || agent.state === "Working").length
}

function legacyAgentRuntimeState(agent: AgentInstance): AgentInstance["state"] {
  return agent.is_processing && agent.state !== "Error" ? "Working" : agent.state
}
