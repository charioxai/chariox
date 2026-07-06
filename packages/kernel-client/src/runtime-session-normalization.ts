import type {
  AgentPromptState,
  RuntimeSession,
} from "./kernel-types.js"

export function normalizeAgentPromptState(
  state: Partial<AgentPromptState> | null | undefined,
): AgentPromptState {
  return {
    active_prompt: state?.active_prompt ?? null,
    queued_prompts: Array.isArray(state?.queued_prompts) ? state.queued_prompts : [],
  }
}

export function normalizeRuntimeSession(session: RuntimeSession): RuntimeSession {
  const { workflow_watchdogs: legacyWorkflowWatchdogs, ...sessionWithoutLegacyWorkflowWatchdogs } = session
  const promptStates = session.prompt_states
    ? Object.fromEntries(
      Object.entries(session.prompt_states).map(([agentId, state]) => [
        agentId,
        normalizeAgentPromptState(state),
      ]),
    )
    : undefined
  const workflowSchedules = Array.isArray(session.workflow_schedules)
    ? session.workflow_schedules
    : Array.isArray(legacyWorkflowWatchdogs)
      ? legacyWorkflowWatchdogs
      : []

  const normalized: RuntimeSession = {
    ...sessionWithoutLegacyWorkflowWatchdogs,
    queued_prompts: Array.isArray(session.queued_prompts) ? session.queued_prompts : [],
    active_interactions: Array.isArray(session.active_interactions) ? session.active_interactions : [],
    metaagent_tasks: Array.isArray(session.metaagent_tasks) ? session.metaagent_tasks : [],
    workflows: Array.isArray(session.workflows) ? session.workflows : [],
    workflow_publications: Array.isArray(session.workflow_publications) ? session.workflow_publications : [],
    workflow_runs: Array.isArray(session.workflow_runs) ? session.workflow_runs : [],
    workflow_prompt_queues: Array.isArray(session.workflow_prompt_queues) ? session.workflow_prompt_queues : [],
    workflow_queued_prompts: Array.isArray(session.workflow_queued_prompts) ? session.workflow_queued_prompts : [],
    workflow_schedules: workflowSchedules,
    workflow_consoles: Array.isArray(session.workflow_consoles) ? session.workflow_consoles : [],
    workspace_links: Array.isArray(session.workspace_links) ? session.workspace_links : [],
    external_provider_imports: Array.isArray(session.external_provider_imports) ? session.external_provider_imports : [],
  }
  if (promptStates) {
    normalized.prompt_states = promptStates
  }
  return normalized
}

export function normalizeRuntimeSessionWithAgentActivity(payload: {
  session: RuntimeSession
  agent_activity?: RuntimeSession["agent_activity"] | null | undefined
  agent_activity_revision?: number | null | undefined
}): RuntimeSession {
  const normalized = normalizeRuntimeSession(payload.session)
  if (payload.agent_activity === null) {
    const { agent_activity: _agentActivity, agent_activity_revision: _revision, ...withoutProjection } = normalized
    return withoutProjection
  }
  if (payload.agent_activity === undefined) {
    return normalized
  }
  const { agent_activity_revision: _revision, ...withoutRevision } = normalized
  return {
    ...withoutRevision,
    agent_activity: payload.agent_activity,
    ...(typeof payload.agent_activity_revision === "number"
      ? { agent_activity_revision: payload.agent_activity_revision }
      : {}),
  }
}

export function normalizeRuntimeSessions(sessions: RuntimeSession[]): RuntimeSession[] {
  return sessions.map((session) => normalizeRuntimeSession(session))
}
