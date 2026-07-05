import type {
  PromptQueueItem,
  RuntimeSession,
} from "./kernel-types.js"
import {
  normalizeAgentRuntimePromptStatus,
  projectAgentRuntimeActivity,
} from "./agent-activity.js"
import {
  sessionHasAgent,
  sessionPromptStateEntriesForSessionAgents,
} from "./session-agent-prompt-state.js"

export type ActivePromptLifecycleRecord = {
  readonly id: string
  readonly status?: string
  readonly promptOrigin?: string | null
  readonly target_agent_id?: string | null
}

export type PromptLifecycleTransition = {
  readonly activePromptChanged: boolean
  readonly cancelledPromptSettled: boolean
  readonly settledAgentIds: string[]
}

export function sessionActivePromptLifecycleRecords(session: RuntimeSession): ActivePromptLifecycleRecord[] {
  if (session.agent_activity) {
    const records: ActivePromptLifecycleRecord[] = []
    for (const [agentId, activity] of Object.entries(session.agent_activity)) {
      if (!sessionHasAgent(session, agentId)) {
        continue
      }
      const projection = projectAgentRuntimeActivity(activity)
      if (projection.activeTurnPromptId) {
        records.push({
          id: projection.activeTurnPromptId,
          status: projection.activeTurnStatus ?? projection.promptStatus,
          promptOrigin: projection.activeTurnPromptOrigin ?? null,
          target_agent_id: agentId,
        })
        continue
      }
      if (!projection.busy) {
        continue
      }
      const stateActivePrompt = session.prompt_states?.[agentId]?.active_prompt
      if (stateActivePrompt) {
        records.push(activePromptLifecycleRecordFromPrompt(stateActivePrompt))
      }
    }
    return records.sort(compareActivePromptLifecycleRecords)
  }
  if (session.prompt_states) {
    return sessionPromptStateEntriesForSessionAgents(session)
      .map(([, state]) => state.active_prompt)
      .map((stateActivePrompt) => stateActivePrompt
        ? activePromptLifecycleRecordFromPrompt(stateActivePrompt)
        : null)
      .filter((prompt): prompt is ActivePromptLifecycleRecord => Boolean(prompt))
      .sort(compareActivePromptLifecycleRecords)
  }
  return session.active_prompt
    ? [activePromptLifecycleRecordFromPrompt(session.active_prompt)]
    : []
}

export function sessionPromptLifecycleTransition(
  currentSession: RuntimeSession,
  nextSession: RuntimeSession,
): PromptLifecycleTransition {
  const currentPromptRecords = sessionActivePromptLifecycleRecords(currentSession)
  const previousPromptIds = currentPromptRecords.map((prompt) => prompt.id)
  const nextPromptIds = activePromptLifecycleRecordIds(nextSession)
  const nextPromptIdSet = new Set(nextPromptIds)
  const settledPromptRecords = currentPromptRecords
    .filter((prompt) => !nextPromptIdSet.has(prompt.id))

  return {
    activePromptChanged:
      previousPromptIds.length !== nextPromptIds.length
      || previousPromptIds.some((id, index) => id !== nextPromptIds[index]),
    settledAgentIds: settledPromptRecords
      .map((prompt) => prompt.target_agent_id)
      .filter((agentId): agentId is string => Boolean(agentId)),
    cancelledPromptSettled:
      currentPromptRecords.some((prompt) => prompt.status === "cancelling" && !nextPromptIdSet.has(prompt.id)),
  }
}

function activePromptLifecycleRecordFromPrompt(prompt: PromptQueueItem): ActivePromptLifecycleRecord {
  return {
    ...prompt,
    status: normalizeAgentRuntimePromptStatus(prompt.status) ?? prompt.status,
    promptOrigin: prompt.prompt_origin ?? null,
  }
}

function activePromptLifecycleRecordIds(session: RuntimeSession): string[] {
  return sessionActivePromptLifecycleRecords(session).map((prompt) => prompt.id)
}

function compareActivePromptLifecycleRecords(
  left: ActivePromptLifecycleRecord,
  right: ActivePromptLifecycleRecord,
): number {
  return left.id.localeCompare(right.id)
}
