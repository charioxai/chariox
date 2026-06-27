import type {
  PromptQueueItem,
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import { promptOriginFromRecord, promptOriginIsExternal } from "@arroba/kernel-client/prompt-origin"
import { agentRuntimeActivityIsBusy } from "./session-state.js"
import { formatTranscriptPreview } from "./transcript-preview.js"
import { reindexTranscriptEntries, trimSingleTrailingNewline } from "./transcript-text.js"

export type QueuedPromptTranscriptSyncResult = {
  entries: TranscriptEntry[]
  changed: boolean
}

export const QUEUED_PROMPT_STEER_EXTERNAL_REASON =
  "Steering is unavailable while the active provider turn was started outside Arroba."
export const QUEUED_PROMPT_STALE_REASON =
  "This prompt is no longer waiting in the queue."

type QueuedPromptActionability = {
  status: string
  steerDisabled: boolean
  canSteer: boolean
  canCancel: boolean
  steerDisabledReason: string | null
  cancelDisabledReason: string | null
}

export function queuedPromptsForAgent(session: RuntimeSession, agentId: string): PromptQueueItem[] | null {
  if (session.agent_activity && !projectedActivityAllowsPromptQueue(session, agentId)) {
    return []
  }
  const promptStates = session.prompt_states
  if (promptStates) {
    return promptStates[agentId]?.queued_prompts ?? []
  }
  const topLevelPrompts = session.queued_prompts.filter((prompt) => prompt.target_agent_id === agentId)
  if (!session.agent_activity) {
    return topLevelPrompts
  }
  return topLevelPrompts.length ? topLevelPrompts : null
}

export function syncQueuedPromptEntriesForAgent(
  entries: readonly TranscriptEntry[],
  session: RuntimeSession,
  agentId: string,
): QueuedPromptTranscriptSyncResult {
  const queuedPrompts = queuedPromptsForAgent(session, agentId)
  if (queuedPrompts === null) {
    return { entries: entries.map((entry) => ({ ...entry })), changed: false }
  }
  const queuedById = new Map(queuedPrompts.map((prompt) => [prompt.id, prompt]))
  const queuedIds = new Set(queuedById.keys())
  const steerDisabled = promptOriginIsExternal(activePromptOrigin(session, agentId))
  let changed = false
  const retained = entries.flatMap((entry) => {
    if (!entry.queuedPrompt) {
      return [entry]
    }
    const keep = entry.queuedPrompt.agentId === agentId && queuedIds.has(entry.queuedPrompt.promptId)
    if (!keep) {
      changed = true
      return []
    }
    const prompt = queuedById.get(entry.queuedPrompt.promptId)
    const actionability = prompt
      ? queuedPromptActionabilityForPrompt(session, agentId, prompt, steerDisabled)
      : queuedPromptActionability(entry.queuedPrompt.status, steerDisabled)
    if (!queuedPromptActionabilityMatches(entry.queuedPrompt, actionability)) {
      changed = true
      return [{
        ...entry,
        queuedPrompt: {
          ...entry.queuedPrompt,
          ...actionability,
        },
      }]
    }
    return [entry]
  })
  const existingQueuedIds = new Set(
    retained
      .map((entry) => entry.queuedPrompt?.promptId)
      .filter((promptId): promptId is string => Boolean(promptId)),
  )
  let nextEntries = retained
  for (const prompt of queuedPrompts) {
    if (existingQueuedIds.has(prompt.id)) {
      continue
    }
    changed = true
    nextEntries = [
      ...nextEntries,
      queuedPromptTranscriptEntry(prompt, agentId, nextTranscriptEntryId(nextEntries), session, steerDisabled),
    ]
  }
  if (!changed) {
    return { entries: entries.map((entry) => ({ ...entry })), changed: false }
  }
  return {
    entries: reindexTranscriptEntries(nextEntries.map((entry) => ({ ...entry })), 0),
    changed: true,
  }
}

export function syncQueuedPromptEntriesByAgent(
  entriesByAgent: Record<string, TranscriptEntry[]>,
  session: RuntimeSession,
): { entriesByAgent: Record<string, TranscriptEntry[]>; previews: Record<string, string>; changed: boolean } {
  let changed = false
  const entriesByAgentNext: Record<string, TranscriptEntry[]> = { ...entriesByAgent }
  const previews: Record<string, string> = {}
  const agentIds = new Set([
    ...session.agents.map((agent) => agent.id),
    ...Object.keys(session.prompt_states ?? {}),
  ])
  if (session.prompt_states || session.agent_activity) {
    for (const [agentId, entries] of Object.entries(entriesByAgent)) {
      if (entries.some((entry) => entry.queuedPrompt)) {
        agentIds.add(agentId)
      }
    }
  }
  for (const agentId of agentIds) {
    const synced = syncQueuedPromptEntriesForAgent(entriesByAgentNext[agentId] ?? [], session, agentId)
    if (synced.changed) {
      changed = true
      entriesByAgentNext[agentId] = synced.entries
      previews[agentId] = formatTranscriptPreview(synced.entries)
    }
  }
  return { entriesByAgent: entriesByAgentNext, previews, changed }
}

function queuedPromptTranscriptEntry(
  prompt: PromptQueueItem,
  agentId: string,
  id: number,
  session: RuntimeSession,
  steerDisabled: boolean,
): TranscriptEntry {
  const actionability = queuedPromptActionabilityForPrompt(session, agentId, prompt, steerDisabled)
  return {
    id,
    role: "user",
    text: trimSingleTrailingNewline(prompt.prompt),
    queuedPrompt: {
      promptId: prompt.id,
      agentId,
      ...actionability,
    },
  }
}

export function queuedPromptActionability(
  status: string | null | undefined,
  steerDisabled: boolean,
): QueuedPromptActionability {
  const normalizedStatus = normalizeQueuedPromptStatus(status)
  const queued = normalizedStatus === "queued"
  return {
    status: normalizedStatus,
    steerDisabled,
    canSteer: queued && !steerDisabled,
    canCancel: queued,
    steerDisabledReason: queued
      ? steerDisabled ? QUEUED_PROMPT_STEER_EXTERNAL_REASON : null
      : QUEUED_PROMPT_STALE_REASON,
    cancelDisabledReason: queued ? null : QUEUED_PROMPT_STALE_REASON,
  }
}

function queuedPromptActionabilityForPrompt(
  session: RuntimeSession,
  agentId: string,
  prompt: PromptQueueItem,
  fallbackSteerDisabled: boolean,
): QueuedPromptActionability {
  const projected = session.agent_activity?.[agentId]?.queued_prompt_controls?.[prompt.id]
  if (!projected) {
    return queuedPromptActionability(prompt.status, fallbackSteerDisabled)
  }
  return {
    status: normalizeQueuedPromptStatus(projected.status),
    steerDisabled: projected.can_steer !== true && Boolean(projected.steer_disabled_reason),
    canSteer: projected.can_steer === true,
    canCancel: projected.can_cancel === true,
    steerDisabledReason: projected.steer_disabled_reason ?? null,
    cancelDisabledReason: projected.cancel_disabled_reason ?? null,
  }
}

function normalizeQueuedPromptStatus(status: string | null | undefined): string {
  return status?.trim().toLowerCase() || "queued"
}

function queuedPromptActionabilityMatches(
  current: NonNullable<TranscriptEntry["queuedPrompt"]>,
  next: QueuedPromptActionability,
): boolean {
  return current.status === next.status
    && current.steerDisabled === next.steerDisabled
    && current.canSteer === next.canSteer
    && current.canCancel === next.canCancel
    && current.steerDisabledReason === next.steerDisabledReason
    && current.cancelDisabledReason === next.cancelDisabledReason
}

function activePromptOrigin(session: RuntimeSession, agentId: string): string | null {
  if (session.agent_activity) {
    const projectedActivity = session.agent_activity[agentId]
    const activeTurnOrigin = promptOriginFromRecord(projectedActivity?.active_turn)
    if (activeTurnOrigin) {
      return activeTurnOrigin
    }
    if (projectedActivity && !agentRuntimeActivityIsBusy(projectedActivity)) {
      return null
    }
  }
  const stateActivePrompt = session.prompt_states?.[agentId]?.active_prompt
  if (stateActivePrompt) {
    return promptOriginFromRecord(stateActivePrompt, "arroba")
  }
  if (session.active_prompt?.target_agent_id === agentId) {
    return promptOriginFromRecord(session.active_prompt, "arroba")
  }
  return null
}

function projectedActivityAllowsPromptQueue(session: RuntimeSession, agentId: string): boolean {
  if (!session.agent_activity) {
    return true
  }
  const activity = session.agent_activity[agentId]
  if (!activity) {
    return false
  }
  return agentRuntimeActivityIsBusy(activity)
}

function nextTranscriptEntryId(entries: readonly TranscriptEntry[]) {
  return entries.reduce((maxId, entry) => Math.max(maxId, entry.id), 0) + 1
}
