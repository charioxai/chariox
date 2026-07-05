import type {
  RuntimeSession,
  TranscriptEntry,
} from "./cli-types.js"
import {
  queuedPromptStripItemsForAgent as sharedQueuedPromptStripItemsForAgent,
  queuedPromptStripItemToTranscriptEntry as sharedQueuedPromptStripItemToTranscriptEntry,
  type QueuedPromptStripItem,
  type QueuedPromptStripStatusOverride,
} from "@arroba/kernel-client/queued-prompt-strip-state"

export type {
  QueuedPromptStripItem,
  QueuedPromptStripStatusOverride,
}

export function queuedPromptStripItemsForAgent(
  session: RuntimeSession,
  entries: readonly TranscriptEntry[],
  agentId: string | null | undefined,
  statusOverrides: readonly QueuedPromptStripStatusOverride[] = [],
): QueuedPromptStripItem[] {
  return sharedQueuedPromptStripItemsForAgent(
    session as Parameters<typeof sharedQueuedPromptStripItemsForAgent>[0],
    entries,
    agentId,
    statusOverrides,
  )
}

export function queuedPromptStripItemToTranscriptEntry(item: QueuedPromptStripItem): TranscriptEntry {
  return sharedQueuedPromptStripItemToTranscriptEntry(item) as TranscriptEntry
}
