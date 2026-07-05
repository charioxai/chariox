import type {
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import {
  queuedPromptProjectionForAgent,
} from "@arroba/kernel-client/queued-prompt-controls"
import {
  sessionHasAgentRuntimeProjection,
} from "@arroba/kernel-client/session-prompt-work"
import { reindexTranscriptEntries } from "@arroba/kernel-client/transcript-entry-state"
import { formatTranscriptPreview } from "./transcript-preview.js"

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
  let changed = false
  const retained = entries.flatMap((entry) => {
    if (!entry.queuedPrompt) {
      return [entry]
    }
    if (entry.queuedPrompt.agentId === agentId) {
      changed = true
      return []
    }
    return [entry]
  })
  if (!changed) {
    return { entries: entries.map((entry) => ({ ...entry })), changed: false }
  }
  return {
    entries: reindexTranscriptEntries(retained.map((entry) => ({ ...entry })), 0),
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
  if (sessionHasAgentRuntimeProjection(session)) {
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
