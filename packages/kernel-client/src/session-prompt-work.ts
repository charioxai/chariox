import type {
  AgentInstance,
  RuntimeSession,
} from "./kernel-types.js"
import {
  agentRuntimeActivityHasTurnWork,
  agentRuntimeActivityIsBusy,
  agentRuntimePromptStatusIsActivePrompt,
  projectAgentRuntimeActivity,
} from "./agent-activity.js"
import {
  agentPromptStateHasWork,
  sessionHasAgent,
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
    const activities = Object.entries(session.agent_activity)
      .filter(([agentId]) => sessionHasAgent(session, agentId))
    const projectedQueued = promptWorkCountFromProjectedActivities(
      activities.map(([, activity]) => activity),
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

export function sessionAgentIsBusy(
  session: RuntimeSession | null | undefined,
  agentId: string | null | undefined,
): boolean {
  if (!session || !agentId) {
    return false
  }
  if (!sessionHasAgent(session, agentId)) {
    return false
  }
  if (session.agent_activity && !(agentId in session.agent_activity)) {
    return false
  }
  const projected = session.agent_activity?.[agentId]
  if (projected) {
    return agentRuntimeActivityIsBusy(projected)
  }
  const promptState = sessionPromptStateRecordForAgent(session, agentId)
  if (promptState !== undefined) {
    return Boolean(promptState?.active_prompt) || Boolean(promptState?.queued_prompts?.length)
  }
  return legacyTopLevelSessionHasPromptWork(session, agentId)
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
    return session.agents.find((agent) => agentRuntimeActivityIsBusy(session.agent_activity?.[agent.id]))?.id ?? null
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
  activity: NonNullable<RuntimeSession["agent_activity"]>[string],
  promptState?: NonNullable<RuntimeSession["prompt_states"]>[string],
): boolean {
  const projection = projectAgentRuntimeActivity(activity)
  if (agentRuntimeActivityHasTurnWork(activity)) {
    return true
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
  if (projection.activePromptCountExplicit) {
    return projection.activePromptCount
  }
  return agentRuntimeActivityHasActivePrompt(activity, promptState) ? 1 : 0
}

function promptWorkCountFromProjectedActivities(
  activities: readonly NonNullable<RuntimeSession["agent_activity"]>[string][],
): number | null {
  let count = 0
  for (const activity of activities) {
    const projection = projectAgentRuntimeActivity(activity)
    if (!projection.queuedPromptCountExplicit) {
      return null
    }
    count += projection.queuedPromptCount
  }
  return count
}

function legacyBusyAgentCount(session: RuntimeSession): number {
  return session.agents.filter((agent: AgentInstance) => agent.is_processing || agent.state === "Working").length
}
