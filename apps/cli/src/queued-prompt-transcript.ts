import type {
  PromptQueueItem,
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import { formatTranscriptPreview } from "./transcript-preview.js"
import { reindexTranscriptEntries, trimSingleTrailingNewline } from "./transcript-text.js"

export type QueuedPromptTranscriptSyncResult = {
  entries: TranscriptEntry[]
  changed: boolean
}

export function queuedPromptsForAgent(session: RuntimeSession, agentId: string): PromptQueueItem[] {
  const statePrompts = session.prompt_states?.[agentId]?.queued_prompts
  if (statePrompts) {
    return statePrompts
  }
  return session.queued_prompts.filter((prompt) => prompt.target_agent_id === agentId)
}

export function syncQueuedPromptEntriesForAgent(
  entries: readonly TranscriptEntry[],
  session: RuntimeSession,
  agentId: string,
): QueuedPromptTranscriptSyncResult {
  const queuedPrompts = queuedPromptsForAgent(session, agentId)
  const queuedIds = new Set(queuedPrompts.map((prompt) => prompt.id))
  let changed = false
  const retained = entries.filter((entry) => {
    if (!entry.queuedPrompt) {
      return true
    }
    const keep = entry.queuedPrompt.agentId === agentId && queuedIds.has(entry.queuedPrompt.promptId)
    if (!keep) {
      changed = true
    }
    return keep
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
  ])
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
): TranscriptEntry {
  return {
    id,
    role: "user",
    text: trimSingleTrailingNewline(prompt.prompt),
    queuedPrompt: {
      promptId: prompt.id,
      agentId,
      status: "queued",
    },
  }
}

function nextTranscriptEntryId(entries: readonly TranscriptEntry[]) {
  return entries.reduce((maxId, entry) => Math.max(maxId, entry.id), 0) + 1
}
