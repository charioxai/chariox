import type {
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import {
  queuedPromptActionabilityMatches,
  queuedPromptProjectionForAgent,
  type ProjectedQueuedPrompt,
} from "@arroba/kernel-client/queued-prompt-controls"
import { sessionHasProjectedRuntimeState } from "./session-state.js"
import { formatTranscriptPreview } from "./transcript-preview.js"
import { reindexTranscriptEntries, trimSingleTrailingNewline } from "./transcript-text.js"

export type QueuedPromptTranscriptSyncResult = {
  entries: TranscriptEntry[]
  changed: boolean
}

export function syncQueuedPromptEntriesForAgent(
  entries: readonly TranscriptEntry[],
  session: RuntimeSession,
  agentId: string,
): QueuedPromptTranscriptSyncResult {
  const projection = queuedPromptProjectionForAgent(
    session as Parameters<typeof queuedPromptProjectionForAgent>[0],
    agentId,
  )
  if (projection.action === "preserve") {
    return { entries: entries.map((entry) => ({ ...entry })), changed: false }
  }
  const queuedPrompts = projection.prompts
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
    const actionability = prompt ?? entry.queuedPrompt
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
      queuedPromptTranscriptEntry(prompt, agentId, nextTranscriptEntryId(nextEntries)),
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
  prompt: ProjectedQueuedPrompt,
  agentId: string,
  id: number,
): TranscriptEntry {
  return {
    id,
    role: "user",
    text: trimSingleTrailingNewline(prompt.prompt),
    queuedPrompt: {
      promptId: prompt.id,
      agentId,
      status: prompt.status,
      steerDisabled: prompt.steerDisabled,
      canSteer: prompt.canSteer,
      canCancel: prompt.canCancel,
      steerDisabledReason: prompt.steerDisabledReason,
      cancelDisabledReason: prompt.cancelDisabledReason,
    },
  }
}

function nextTranscriptEntryId(entries: readonly TranscriptEntry[]) {
  return entries.reduce((maxId, entry) => Math.max(maxId, entry.id), 0) + 1
}
