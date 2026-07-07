import type {
  PromptQueueItem,
  RuntimeSession,
} from "./kernel-types.js"
import {
  type AgentRuntimeActivityProjection,
  normalizeAgentRuntimePromptStatus,
} from "./agent-activity.js"
import {
  externalProviderObservedIdentityKey,
  type ExternalProviderObservedTranscriptIdentityFields,
} from "./external-provider-observation.js"
import {
  EXTERNAL_PROMPT_ORIGIN,
  promptOriginFromRecord,
} from "./prompt-origin.js"
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
  readonly source_attachment_id?: string | null
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
      const stateActivePrompt = sessionPromptStateRecordForAgent(session, agentId)?.active_prompt
      const activeTurnRecord = activePromptLifecycleRecordFromProjectedTurn(agentId, projection)
      if (activeTurnRecord) {
        records.push(activePromptLifecycleRecordWithPromptState(activeTurnRecord, stateActivePrompt))
        continue
      }
      if (!projection.busy) {
        continue
      }
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
  const nextPromptFingerprints = nextPromptRecords.map(activePromptLifecycleRecordFingerprint)
  const nextPromptIdentitySet = new Set(nextPromptRecords.map(activePromptLifecycleRecordIdentityFingerprint))
  const settledPromptRecords = currentPromptRecords
    .filter((prompt) => !nextPromptIdentitySet.has(activePromptLifecycleRecordIdentityFingerprint(prompt)))

  return {
    activePromptChanged:
      previousPromptFingerprints.length !== nextPromptFingerprints.length
      || previousPromptFingerprints.some((fingerprint, index) => fingerprint !== nextPromptFingerprints[index]),
    settledAgentIds: settledPromptRecords
      .map((prompt) => prompt.target_agent_id)
      .filter((agentId): agentId is string => Boolean(agentId)),
    cancelledPromptSettled:
      currentPromptRecords.some((prompt) =>
        prompt.status === "cancelling"
        && !nextPromptIdentitySet.has(activePromptLifecycleRecordIdentityFingerprint(prompt))),
  }
}

function activePromptLifecycleRecordFromPrompt(prompt: PromptQueueItem): ActivePromptLifecycleRecord {
  const promptOrigin = promptOriginFromRecord(prompt)
  return {
    ...prompt,
    status: normalizeAgentRuntimePromptStatus(prompt.status) ?? prompt.status,
    promptOrigin,
    ...(promptOrigin === EXTERNAL_PROMPT_ORIGIN && prompt.external_provider !== undefined
      ? { externalProvider: prompt.external_provider }
      : {}),
    ...(promptOrigin === EXTERNAL_PROMPT_ORIGIN && prompt.external_provider_session_id !== undefined
      ? { externalProviderSessionId: prompt.external_provider_session_id }
      : {}),
    ...(promptOrigin === EXTERNAL_PROMPT_ORIGIN && prompt.external_provider_turn_id !== undefined
      ? { externalProviderTurnId: prompt.external_provider_turn_id }
      : {}),
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
    promptOrigin: activePromptLifecycleRecordPromptOriginFromProjectedTurn(projection),
    target_agent_id: agentId,
    ...(projection.activeTurnProviderRunId ? { providerRunId: projection.activeTurnProviderRunId } : {}),
    ...(projection.activeTurnSourceAttachmentId !== undefined
      ? { source_attachment_id: projection.activeTurnSourceAttachmentId }
      : {}),
    ...(projection.activeTurnExternalProvider ? { externalProvider: projection.activeTurnExternalProvider } : {}),
    ...(projection.activeTurnExternalProviderSessionId
      ? { externalProviderSessionId: projection.activeTurnExternalProviderSessionId }
      : {}),
    ...(projection.activeTurnExternalProviderTurnId
      ? { externalProviderTurnId: projection.activeTurnExternalProviderTurnId }
      : {}),
  }
}

function activePromptLifecycleRecordPromptOriginFromProjectedTurn(
  projection: AgentRuntimeActivityProjection,
): string | null {
  if (projection.activeTurnPromptOrigin !== undefined) {
    return projection.activeTurnPromptOrigin
  }
  return null
}

function activePromptLifecycleRecordWithPromptState(
  record: ActivePromptLifecycleRecord,
  stateActivePrompt: PromptQueueItem | null | undefined,
): ActivePromptLifecycleRecord {
  if (record.source_attachment_id !== undefined || !stateActivePrompt) {
    return record
  }
  if (!activePromptLifecycleRecordMatchesPromptState(record, stateActivePrompt)) {
    return record
  }
  return {
    ...record,
    source_attachment_id: stateActivePrompt.source_attachment_id,
  }
}

function activePromptLifecycleRecordMatchesPromptState(
  record: ActivePromptLifecycleRecord,
  stateActivePrompt: PromptQueueItem,
): boolean {
  if (stateActivePrompt.target_agent_id && stateActivePrompt.target_agent_id !== record.target_agent_id) {
    return false
  }
  if (stateActivePrompt.id === record.id) {
    return true
  }
  const stateExternalKey = activePromptLifecycleRecordExternalIdentityKey(
    activePromptLifecycleRecordFromPrompt(stateActivePrompt),
  )
  if (!stateExternalKey) {
    return false
  }
  return stateExternalKey.provider === normalizeExternalProviderLifecycleProvider(record.externalProvider)
    && stateExternalKey.providerSessionId === (record.externalProviderSessionId ?? "")
    && stateExternalKey.providerTurnId === (record.externalProviderTurnId ?? "")
}

function activePromptLifecycleRecordFingerprint(prompt: ActivePromptLifecycleRecord): string {
  return [
    activePromptLifecycleRecordIdentityFingerprint(prompt),
    prompt.status ?? "",
    prompt.source_attachment_id ?? "",
  ].join("\u001f")
}

function activePromptLifecycleRecordIdentityFingerprint(prompt: ActivePromptLifecycleRecord): string {
  const externalIdentityKey = activePromptLifecycleRecordExternalIdentityKey(prompt)
  return [
    prompt.id,
    prompt.promptOrigin ?? "",
    prompt.target_agent_id ?? "",
    prompt.providerRunId ?? "",
    externalIdentityKey?.provider ?? normalizeExternalProviderLifecycleProvider(prompt.externalProvider),
    externalIdentityKey?.providerSessionId ?? "",
    externalIdentityKey?.providerTurnId ?? "",
  ].join("\u001f")
}

function activePromptLifecycleRecordExternalIdentityKey(prompt: ActivePromptLifecycleRecord) {
  return externalProviderObservedIdentityKey({
    promptId: null,
    ...(prompt.externalProvider !== undefined ? { externalProvider: prompt.externalProvider } : {}),
    ...(prompt.externalProviderSessionId !== undefined
      ? { externalProviderSessionId: prompt.externalProviderSessionId }
      : {}),
    ...(prompt.externalProviderTurnId !== undefined ? { externalProviderTurnId: prompt.externalProviderTurnId } : {}),
  })
}

function compareActivePromptLifecycleRecords(
  left: ActivePromptLifecycleRecord,
  right: ActivePromptLifecycleRecord,
): number {
  return left.id.localeCompare(right.id)
}

function normalizeExternalProviderLifecycleProvider(value: string | null | undefined): string {
  return value?.trim().toLowerCase() ?? ""
}
