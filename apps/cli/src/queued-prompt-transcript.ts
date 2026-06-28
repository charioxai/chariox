import type {
  PromptQueueItem,
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import {
  queuedPromptActionability,
  type QueuedPromptActionability,
} from "@arroba/kernel-client/queued-prompt-controls"
import { agentRuntimeActivityIsBusy, sessionHasProjectedRuntimeState } from "./session-state.js"
import { formatTranscriptPreview } from "./transcript-preview.js"
import { reindexTranscriptEntries, trimSingleTrailingNewline } from "./transcript-text.js"

export type QueuedPromptTranscriptSyncResult = {
  entries: TranscriptEntry[]
  changed: boolean
}

export function queuedPromptsForAgent(session: RuntimeSession, agentId: string): PromptQueueItem[] | null {
  if (session.agent_activity && !projectedActivityAllowsPromptQueue(session, agentId)) {
    return []
  }
  const promptStates = session.prompt_states
  if (promptStates) {
    return promptStates[agentId]?.queued_prompts ?? []
  }
  if (session.agent_activity) {
    return null
  }
  const topLevelPrompts = session.queued_prompts.filter((prompt) => prompt.target_agent_id === agentId)
  return topLevelPrompts
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
      ? queuedPromptActionabilityForPrompt(session, agentId, prompt)
      : queuedPromptActionability(entry.queuedPrompt.status)
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
      queuedPromptTranscriptEntry(prompt, agentId, nextTranscriptEntryId(nextEntries), session),
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
    ...Object.keys(session.agent_activity ?? {}),
  ])
  if (sessionHasProjectedRuntimeState(session)) {
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
): TranscriptEntry {
  const actionability = queuedPromptActionabilityForPrompt(session, agentId, prompt)
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

function queuedPromptActionabilityForPrompt(
  session: RuntimeSession,
  agentId: string,
  prompt: PromptQueueItem,
): QueuedPromptActionability {
  const projected = session.agent_activity?.[agentId]?.queued_prompt_controls?.[prompt.id]
  return queuedPromptActionability(prompt.status, projected)
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
