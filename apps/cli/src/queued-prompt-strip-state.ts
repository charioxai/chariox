import {
  queuedPromptProjectionForAgent,
  type ProjectedQueuedPrompt,
} from "@arroba/kernel-client/queued-prompt-controls"
import type {
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import { trimSingleTrailingNewline } from "./transcript-text.js"

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

export function queuedPromptStripItemsForAgent(
  session: RuntimeSession,
  entries: readonly TranscriptEntry[],
  agentId: string | null | undefined,
): QueuedPromptStripItem[] {
  if (!agentId) {
    return []
  }
  const optimisticByPromptId = queuedPromptItemsFromEntries(entries, agentId)
  const projection = queuedPromptProjectionForAgent(
    session as Parameters<typeof queuedPromptProjectionForAgent>[0],
    agentId,
  )
  if (projection.action === "preserve") {
    return optimisticByPromptId
  }
  return projection.prompts.map((prompt) => {
    const optimistic = optimisticByPromptId.find((candidate) => candidate.promptId === prompt.id)
    return queuedPromptItemFromProjection(prompt, agentId, optimistic)
  })
}

export function queuedPromptStripItemToTranscriptEntry(item: QueuedPromptStripItem): TranscriptEntry {
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
  entries: readonly TranscriptEntry[],
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
): QueuedPromptStripItem {
  if (optimistic) {
    return {
      ...optimistic,
      prompt: trimSingleTrailingNewline(prompt.prompt),
      sourceAttachmentId: prompt.sourceAttachmentId,
      attachmentCount: prompt.attachmentCount,
    }
  }
  return {
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
  }
}
