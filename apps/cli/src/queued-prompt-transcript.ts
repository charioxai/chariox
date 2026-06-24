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
  const steerDisabled = activePromptOrigin(session, agentId) === "external"
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
    if (entry.queuedPrompt.steerDisabled !== steerDisabled) {
      changed = true
      return [{
        ...entry,
        queuedPrompt: {
          ...entry.queuedPrompt,
          steerDisabled,
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
      queuedPromptTranscriptEntry(prompt, agentId, nextTranscriptEntryId(nextEntries), steerDisabled),
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
  steerDisabled: boolean,
): TranscriptEntry {
  return {
    id,
    role: "user",
    text: trimSingleTrailingNewline(prompt.prompt),
    queuedPrompt: {
      promptId: prompt.id,
      agentId,
      status: "queued",
      steerDisabled,
    },
  }
}

function activePromptOrigin(session: RuntimeSession, agentId: string): string | null {
  const activeTurnOrigin = session.agent_activity?.[agentId]?.active_turn?.prompt_origin
  if (activeTurnOrigin) {
    return activeTurnOrigin
  }
  const stateActivePrompt = session.prompt_states?.[agentId]?.active_prompt
  if (stateActivePrompt) {
    return stateActivePrompt.prompt_origin ?? "arroba"
  }
  if (session.active_prompt?.target_agent_id === agentId) {
    return session.active_prompt.prompt_origin ?? "arroba"
  }
  return null
}

function nextTranscriptEntryId(entries: readonly TranscriptEntry[]) {
  return entries.reduce((maxId, entry) => Math.max(maxId, entry.id), 0) + 1
}
