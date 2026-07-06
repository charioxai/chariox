import type {
  PromptQueueItem,
  RuntimeSession,
} from "./kernel-types.js"
import {
  type AgentRuntimeActivityProjection,
  normalizeAgentRuntimePromptStatus,
} from "./agent-activity.js"
import type { ExternalProviderObservedTranscriptIdentityFields } from "./external-provider-observation.js"
import { promptOriginFromRecord } from "./prompt-origin.js"
import {
  sessionHasAgentActivityProjection,
  sessionHasPromptStateProjection,
  sessionProjectedPromptActivityEntriesForSessionAgents,
  sessionPromptStateEntriesForSessionAgents,
  sessionPromptStateRecordForAgent,
} from "./session-agent-prompt-state.js"

export type ActivePromptLifecycleRecord = ExternalProviderObservedTranscriptIdentityFields & {
  readonly id: string
  readonly status?: string
  readonly promptOrigin?: string | null
  readonly target_agent_id?: string | null
  readonly providerRunId?: string | null
}

export type PromptLifecycleTransition = {
  readonly activePromptChanged: boolean
  readonly cancelledPromptSettled: boolean
  readonly settledAgentIds: string[]
}

export function sessionActivePromptLifecycleRecords(session: RuntimeSession): ActivePromptLifecycleRecord[] {
  if (sessionHasAgentActivityProjection(session)) {
    const records: ActivePromptLifecycleRecord[] = []
    for (const [agentId, projection] of sessionProjectedPromptActivityEntriesForSessionAgents(session)) {
      const activeTurnRecord = activePromptLifecycleRecordFromProjectedTurn(agentId, projection)
      if (activeTurnRecord) {
        records.push(activeTurnRecord)
        continue
      }
      if (!projection.busy) {
        continue
      }
      const stateActivePrompt = sessionPromptStateRecordForAgent(session, agentId)?.active_prompt
      if (stateActivePrompt) {
        records.push(activePromptLifecycleRecordFromPrompt(stateActivePrompt))
      }
    }
    return records.sort(compareActivePromptLifecycleRecords)
  }
  if (sessionHasPromptStateProjection(session)) {
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
  const previousPromptFingerprints = currentPromptRecords.map(activePromptLifecycleRecordFingerprint)
  const nextPromptRecords = sessionActivePromptLifecycleRecords(nextSession)
  const nextPromptIds = nextPromptRecords.map((prompt) => prompt.id)
  const nextPromptFingerprints = nextPromptRecords.map(activePromptLifecycleRecordFingerprint)
  const nextPromptIdSet = new Set(nextPromptIds)
  const settledPromptRecords = currentPromptRecords
    .filter((prompt) => !nextPromptIdSet.has(prompt.id))

  return {
    activePromptChanged:
      previousPromptFingerprints.length !== nextPromptFingerprints.length
      || previousPromptFingerprints.some((fingerprint, index) => fingerprint !== nextPromptFingerprints[index]),
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
    promptOrigin: promptOriginFromRecord(prompt),
  }
}

function activePromptLifecycleRecordFromProjectedTurn(
  agentId: string,
  projection: AgentRuntimeActivityProjection,
): ActivePromptLifecycleRecord | null {
  if (!projection.activeTurnPromptId) {
    return null
  }
  return {
    id: projection.activeTurnPromptId,
    status: projection.activeTurnStatus ?? projection.promptStatus,
    promptOrigin: projection.activeTurnPromptOrigin ?? null,
    target_agent_id: agentId,
    ...(projection.activeTurnProviderRunId ? { providerRunId: projection.activeTurnProviderRunId } : {}),
    ...(projection.activeTurnExternalProvider ? { externalProvider: projection.activeTurnExternalProvider } : {}),
    ...(projection.activeTurnExternalProviderSessionId
      ? { externalProviderSessionId: projection.activeTurnExternalProviderSessionId }
      : {}),
    ...(projection.activeTurnExternalProviderTurnId
      ? { externalProviderTurnId: projection.activeTurnExternalProviderTurnId }
      : {}),
  }
}

function activePromptLifecycleRecordFingerprint(prompt: ActivePromptLifecycleRecord): string {
  return [
    prompt.id,
    prompt.status ?? "",
    prompt.promptOrigin ?? "",
    prompt.target_agent_id ?? "",
    prompt.providerRunId ?? "",
    prompt.externalProvider ?? "",
    prompt.externalProviderSessionId ?? "",
    prompt.externalProviderTurnId ?? "",
  ].join("\u001f")
}

function compareActivePromptLifecycleRecords(
  left: ActivePromptLifecycleRecord,
  right: ActivePromptLifecycleRecord,
): number {
  return left.id.localeCompare(right.id)
}
