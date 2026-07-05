import type {
  AgentInstance,
  RuntimeSession,
} from "./kernel-types.js"
import {
  agentLegacyProcessingStateIsBusy,
  agentRuntimeActivityProjectionHasTurnWork,
  agentRuntimePromptStatusIsActivePrompt,
  type AgentRuntimeActivityProjection,
} from "./agent-activity.js"
import {
  agentPromptStateHasWork,
  sessionProjectedPromptActivityEntriesForSessionAgents,
  sessionProjectedPromptActivityForAgent,
  sessionPromptStateEntriesForSessionAgents,
  sessionPromptStateRecordForAgent,
} from "./session-agent-prompt-state.js"

export function sessionHasAgentRuntimeProjection(session: RuntimeSession | null | undefined): boolean {
  return Boolean(session?.agent_activity || session?.prompt_states)
}

export type SessionPromptWorkSummary = {
  readonly active: number
  readonly queued: number
  readonly busyAgents: number
}

export function sessionPromptWorkSummary(session: RuntimeSession): SessionPromptWorkSummary {
  const promptStates = session.prompt_states
  const promptStateEntries = sessionPromptStateEntriesForSessionAgents(session)
  const queued = promptStates
    ? promptStateEntries.reduce((count, [, state]) => count + (state?.queued_prompts?.length ?? 0), 0)
    : session.queued_prompts.length

  if (session.agent_activity) {
    const activities = sessionProjectedPromptActivityEntriesForSessionAgents(session)
    const projectedQueued = promptWorkCountFromProjectedActivities(
      activities.map(([, projection]) => projection),
    )
    return {
      active: activities.reduce(
        (count, [agentId, projection]) =>
          count + projectedActivePromptCount(projection, promptStates?.[agentId]),
        0,
      ),
      queued: projectedQueued ?? queued,
      busyAgents: activities.filter(([, projection]) => projection.busy).length,
    }
  }

  if (promptStates) {
    const busyAgents = promptStateEntries.filter(([, state]) => agentPromptStateHasWork(state)).length
    return {
      active: promptStateEntries.filter(([, state]) => Boolean(state?.active_prompt)).length,
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

export function sessionQueuedPromptCount(
  session: RuntimeSession,
  agentId: string | null | undefined = null,
): number {
  if (!agentId) {
    return sessionPromptWorkSummary(session).queued
  }
  const projected = sessionProjectedPromptActivityForAgent(session, agentId)
  if (projected === "not_found" || projected === "idle") {
    return 0
  }
  if (projected) {
    if (projected.queuedPromptCountExplicit) {
      return projected.queuedPromptCount
    }
    if (!projected.busy) {
      return 0
    }
  }
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
  if (promptState !== undefined) {
    return promptState?.queued_prompts?.length ?? 0
  }
  if (session.agent_activity) {
    return 0
  }
  return session.queued_prompts.filter((prompt) => prompt.target_agent_id === agentId).length
}

export function sessionAgentIsBusy(
  session: RuntimeSession | null | undefined,
  agentId: string | null | undefined,
): boolean {
  if (!session || !agentId) {
    return false
  }
  const projected = sessionProjectedPromptActivityForAgent(session, agentId)
  if (projected === "not_found" || projected === "idle") {
    return false
  }
  if (projected) {
    return projected.busy
  }
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
  if (promptState !== undefined) {
    return Boolean(promptState?.active_prompt) || Boolean(promptState?.queued_prompts?.length)
  }
  return legacyTopLevelSessionHasPromptWork(session, agentId)
}

export function sessionAgentBusyForProviderRunRecovery(
  session: RuntimeSession | null | undefined,
  agentId: string | null | undefined,
): boolean | null {
  if (!sessionHasAgentRuntimeProjection(session)) {
    return null
  }
  return sessionAgentIsBusy(session, agentId)
}

export function sessionHasPromptWork(session: RuntimeSession): boolean {
  const summary = sessionPromptWorkSummary(session)
  return summary.active > 0 || summary.queued > 0 || summary.busyAgents > 0
}

export function sessionHasProcessingAgent(session: RuntimeSession): boolean {
  return sessionPromptWorkSummary(session).busyAgents > 0
}

export function sessionPromptWorkByAgent(session: RuntimeSession): Record<string, boolean> {
  const state: Record<string, boolean> = {}
  for (const agent of session.agents) {
    state[agent.id] = sessionAgentIsBusy(session, agent.id)
  }
  return state
}

export function sessionProjectedStreamingAgentId(session: RuntimeSession): string | null {
  if (session.agent_activity) {
    return sessionProjectedPromptActivityEntriesForSessionAgents(session)
      .find(([, projection]) => agentRuntimeActivityProjectionHasTurnWork(projection))?.[0] ?? null
  }
  if (session.prompt_states) {
    const activeAgents = session.agents.filter((agent) => {
      const promptState = session.prompt_states?.[agent.id]
      return Boolean(promptState?.active_prompt)
    })
    return activeAgents.length === 1 ? activeAgents[0]?.id ?? null : null
  }
  return session.active_prompt?.target_agent_id ?? null
}

function legacyTopLevelSessionHasPromptWork(session: RuntimeSession, agentId: string): boolean {
  return Boolean(session.active_prompt?.target_agent_id === agentId)
    || Boolean(session.queued_prompts.some((prompt) => prompt.target_agent_id === agentId))
}

function agentRuntimeActivityHasActivePrompt(
  projection: AgentRuntimeActivityProjection,
  promptState?: NonNullable<RuntimeSession["prompt_states"]>[string],
): boolean {
  if (agentRuntimeActivityProjectionHasTurnWork(projection)) {
    return true
  }
  if (projection.busy && promptState?.active_prompt) {
    return true
  }
  return agentRuntimePromptStatusIsActivePrompt(projection.promptStatus)
}

function projectedActivePromptCount(
  projection: AgentRuntimeActivityProjection,
  promptState?: NonNullable<RuntimeSession["prompt_states"]>[string],
): number {
  if (projection.activePromptCountExplicit) {
    return projection.activePromptCount
  }
  return agentRuntimeActivityHasActivePrompt(projection, promptState) ? 1 : 0
}

function promptWorkCountFromProjectedActivities(
  activities: readonly AgentRuntimeActivityProjection[],
): number | null {
  let count = 0
  for (const projection of activities) {
    if (!projection.queuedPromptCountExplicit) {
      return null
    }
    count += projection.queuedPromptCount
  }
  return count
}

function legacyBusyAgentCount(session: RuntimeSession): number {
  return session.agents.filter((agent: AgentInstance) => agentLegacyProcessingStateIsBusy(agent)).length
}
