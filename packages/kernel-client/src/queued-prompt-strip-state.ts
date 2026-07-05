import {
  queuedPromptProjectionForAgent,
  type ProjectedQueuedPrompt,
} from "./queued-prompt-controls.js"
import type { RuntimeSession } from "./kernel-types.js"
import { trimSingleTrailingNewline } from "./transcript-entry-state.js"

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

export type QueuedPromptStripTranscriptEntry = {
  id: number
  role: "user"
  text: string
  sourceAttachmentId?: string | null
  queuedPrompt?: {
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
}

export type QueuedPromptStripSourceEntry = {
  readonly id?: number
  readonly role?: string
  readonly text: string
  readonly sourceAttachmentId?: string | null
  readonly queuedPrompt?: {
    readonly promptId: string
    readonly agentId: string
    readonly status: string
    readonly attachmentCount: number
    readonly steerDisabled: boolean
    readonly canSteer: boolean
    readonly canCancel: boolean
    readonly steerDisabledReason: string | null
    readonly cancelDisabledReason: string | null
  }
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
