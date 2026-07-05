import {
  queuedPromptProjectionForAgent,
  type ProjectedQueuedPrompt,
} from "./queued-prompt-controls.js"
import type { RuntimeSession, TranscriptEntry } from "./kernel-types.js"
import { formatTranscriptPreview } from "./session-history-preview.js"
import {
  reindexTranscriptEntries,
  trimSingleTrailingNewline,
} from "./transcript-entry-state.js"
import { sessionHasAgentRuntimeProjection } from "./session-prompt-work.js"

export type QueuedPromptStripItem = {
  promptId: string
  agentId: string
  sourceAttachmentId: string | null
  prompt: string
  status: string
  attachmentCount: number
  steerDisabled: boolean
  canSteer: boolean
  canCancel: boolean
  steerDisabledReason: string | null
  cancelDisabledReason: string | null
}

export type QueuedPromptStripStatusOverride = {
  promptId: string
  agentId: string
  status: string
  steerDisabled: boolean
  canSteer: boolean
  canCancel: boolean
  steerDisabledReason: string | null
  cancelDisabledReason: string | null
}

export type QueuedPromptTranscriptMetadata = {
  promptId: string
  agentId: string
  status: string
  attachmentCount: number
  steerDisabled: boolean
  canSteer: boolean
  canCancel: boolean
  steerDisabledReason: string | null
  cancelDisabledReason: string | null
}

export type QueuedPromptStripTranscriptEntry = {
  id: number
  role: "user"
  text: string
  sourceAttachmentId?: string | null
  queuedPrompt?: QueuedPromptTranscriptMetadata
}

export type QueuedPromptStripSourceEntry = {
  readonly id?: number
  readonly role?: string
  readonly text: string
  readonly sourceAttachmentId?: string | null
  readonly queuedPrompt?: Readonly<QueuedPromptTranscriptMetadata>
}

export type QueuedPromptTranscriptSyncEntry = QueuedPromptStripSourceEntry & {
  readonly id: number
}

export type QueuedPromptTranscriptSyncResult<TEntry extends QueuedPromptTranscriptSyncEntry> = {
  entries: TEntry[]
  changed: boolean
}

export type QueuedPromptTranscriptByAgentSyncResult<TEntry extends QueuedPromptTranscriptSyncEntry> = {
  entriesByAgent: Record<string, TEntry[]>
  changedAgentIds: string[]
  changed: boolean
}

export type QueuedPromptTranscriptPreviewEntry = QueuedPromptTranscriptSyncEntry
  & Pick<TranscriptEntry, "role" | "text" | "hidden">

export type QueuedPromptTranscriptByAgentPreviewSyncResult<TEntry extends QueuedPromptTranscriptPreviewEntry> = {
  entriesByAgent: Record<string, TEntry[]>
  previews: Record<string, string>
  changed: boolean
}

export function queuedPromptStripItemsForAgent(
  session: RuntimeSession,
  entries: readonly QueuedPromptStripSourceEntry[],
  agentId: string | null | undefined,
  statusOverrides: readonly QueuedPromptStripStatusOverride[] = [],
): QueuedPromptStripItem[] {
  if (!agentId) {
    return []
  }
  const agentStatusOverrides = statusOverrides.filter((override) => override.agentId === agentId)
  const optimisticByPromptId = overlayQueuedPromptStatus(
    queuedPromptItemsFromEntries(entries, agentId),
    agentStatusOverrides,
  )
  const projection = queuedPromptProjectionForAgent(session, agentId)
  if (projection.action === "preserve") {
    return optimisticByPromptId
  }
  return projection.prompts.map((prompt) => {
    const optimistic = optimisticByPromptId.find((candidate) => candidate.promptId === prompt.id)
    const override = agentStatusOverrides.find((candidate) => candidate.promptId === prompt.id)
    return queuedPromptItemFromProjection(prompt, agentId, optimistic, override)
  })
}

export function queuedPromptStripItemToTranscriptEntry(
  item: QueuedPromptStripItem,
): QueuedPromptStripTranscriptEntry {
  return {
    id: 0,
    role: "user",
    text: item.prompt,
    sourceAttachmentId: item.sourceAttachmentId,
    queuedPrompt: {
      promptId: item.promptId,
      agentId: item.agentId,
      status: item.status,
      attachmentCount: item.attachmentCount,
      steerDisabled: item.steerDisabled,
      canSteer: item.canSteer,
      canCancel: item.canCancel,
      steerDisabledReason: item.steerDisabledReason,
      cancelDisabledReason: item.cancelDisabledReason,
    },
  }
}

export function syncQueuedPromptTranscriptEntriesForAgent<TEntry extends QueuedPromptTranscriptSyncEntry>(
  entries: readonly TEntry[],
  session: RuntimeSession,
  agentId: string,
): QueuedPromptTranscriptSyncResult<TEntry> {
  const projection = queuedPromptProjectionForAgent(session, agentId)
  if (projection.action === "preserve") {
    return { entries: entries.map((entry) => ({ ...entry })) as TEntry[], changed: false }
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
    return { entries: entries.map((entry) => ({ ...entry })) as TEntry[], changed: false }
  }
  return {
    entries: reindexTranscriptEntries(retained.map((entry) => ({ ...entry })), 0) as TEntry[],
    changed: true,
  }
}

export function syncQueuedPromptTranscriptEntriesByAgent<TEntry extends QueuedPromptTranscriptSyncEntry>(
  entriesByAgent: Record<string, TEntry[]>,
  session: RuntimeSession,
): QueuedPromptTranscriptByAgentSyncResult<TEntry> {
  const entriesByAgentNext: Record<string, TEntry[]> = { ...entriesByAgent }
  const changedAgentIds: string[] = []
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
    const synced = syncQueuedPromptTranscriptEntriesForAgent(entriesByAgentNext[agentId] ?? [], session, agentId)
    if (synced.changed) {
      entriesByAgentNext[agentId] = synced.entries
      changedAgentIds.push(agentId)
    }
  }
  return {
    entriesByAgent: entriesByAgentNext,
    changedAgentIds,
    changed: changedAgentIds.length > 0,
  }
}

export function syncQueuedPromptTranscriptEntriesByAgentWithPreviews<TEntry extends QueuedPromptTranscriptPreviewEntry>(
  entriesByAgent: Record<string, TEntry[]>,
  session: RuntimeSession,
): QueuedPromptTranscriptByAgentPreviewSyncResult<TEntry> {
  const synced = syncQueuedPromptTranscriptEntriesByAgent(entriesByAgent, session)
  const previews: Record<string, string> = {}
  for (const agentId of synced.changedAgentIds) {
    previews[agentId] = formatTranscriptPreview(synced.entriesByAgent[agentId] ?? [])
  }
  return {
    entriesByAgent: synced.entriesByAgent,
    previews,
    changed: synced.changed,
  }
}

function queuedPromptItemsFromEntries(
  entries: readonly QueuedPromptStripSourceEntry[],
  agentId: string,
): QueuedPromptStripItem[] {
  return entries.flatMap((entry) => {
    const queuedPrompt = entry.queuedPrompt
    if (!queuedPrompt || queuedPrompt.agentId !== agentId) {
      return []
    }
    return [{
      promptId: queuedPrompt.promptId,
      agentId: queuedPrompt.agentId,
      sourceAttachmentId: entry.sourceAttachmentId ?? null,
      prompt: trimSingleTrailingNewline(entry.text),
      status: queuedPrompt.status,
      attachmentCount: queuedPrompt.attachmentCount,
      steerDisabled: queuedPrompt.steerDisabled,
      canSteer: queuedPrompt.canSteer,
      canCancel: queuedPrompt.canCancel,
      steerDisabledReason: queuedPrompt.steerDisabledReason,
      cancelDisabledReason: queuedPrompt.cancelDisabledReason,
    }]
  })
}

function queuedPromptItemFromProjection(
  prompt: ProjectedQueuedPrompt,
  agentId: string,
  optimistic: QueuedPromptStripItem | undefined,
  statusOverride: QueuedPromptStripStatusOverride | undefined,
): QueuedPromptStripItem {
  const applyOverride = (item: QueuedPromptStripItem): QueuedPromptStripItem => {
    if (!statusOverride) {
      return item
    }
    return {
      ...item,
      status: statusOverride.status,
      steerDisabled: statusOverride.steerDisabled,
      canSteer: statusOverride.canSteer,
      canCancel: statusOverride.canCancel,
      steerDisabledReason: statusOverride.steerDisabledReason,
      cancelDisabledReason: statusOverride.cancelDisabledReason,
    }
  }
  if (optimistic) {
    return applyOverride({
      ...optimistic,
      prompt: trimSingleTrailingNewline(prompt.prompt),
      sourceAttachmentId: prompt.sourceAttachmentId,
      attachmentCount: prompt.attachmentCount,
    })
  }
  return applyOverride({
    promptId: prompt.id,
    agentId,
    sourceAttachmentId: prompt.sourceAttachmentId,
    prompt: trimSingleTrailingNewline(prompt.prompt),
    status: prompt.status,
    attachmentCount: prompt.attachmentCount,
    steerDisabled: prompt.steerDisabled,
    canSteer: prompt.canSteer,
    canCancel: prompt.canCancel,
    steerDisabledReason: prompt.steerDisabledReason,
    cancelDisabledReason: prompt.cancelDisabledReason,
  })
}

function overlayQueuedPromptStatus(
  items: QueuedPromptStripItem[],
  statusOverrides: readonly QueuedPromptStripStatusOverride[],
): QueuedPromptStripItem[] {
  if (statusOverrides.length === 0) {
    return items
  }
  return items.map((item) => {
    const statusOverride = statusOverrides.find((override) => override.promptId === item.promptId)
    if (!statusOverride) {
      return item
    }
    return {
      ...item,
      status: statusOverride.status,
      steerDisabled: statusOverride.steerDisabled,
      canSteer: statusOverride.canSteer,
      canCancel: statusOverride.canCancel,
      steerDisabledReason: statusOverride.steerDisabledReason,
      cancelDisabledReason: statusOverride.cancelDisabledReason,
    }
  })
}
