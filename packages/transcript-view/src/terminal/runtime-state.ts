export type FreeformTranscriptRole = "user" | "agent" | "reasoning" | "tool" | "status" | "error"
import type { TerminalPromptAttachmentView } from "./prompt-attachment-state.js"
export type TerminalOutputRole = "user" | "assistant" | "reasoning" | "tool" | "status" | "error"
export type TerminalOutputEntry = {
  readonly key: string
  readonly role: TerminalOutputRole
  readonly agentId: string | null
  turnId?: string
  turnTracking?: "none"
  createdAtMs?: number
  text: string
  sourceText?: string
  source?: "external_provider_observed" | string | null
  promptOrigin?: string | null
  promptRole?: "steering"
  promptId?: string | null
  providerRunId?: string | null
  sourceAttachmentId?: string | null
  externalProvider?: string | null
  externalProviderSessionId?: string | null
  externalProviderTurnId?: string | null
  observedAtMs?: number | null
  externalObservation?: TerminalExternalObservation | null
  sourceEntryIds?: Set<string>
  outputIdentity?: string
  historyTurnSummary?: boolean
  hydratedHistoryBlobIds?: Set<string>
  attachments?: readonly TerminalPromptAttachmentView[]
  historyBlob?: TerminalHistoryBlobState
}

export type TerminalExternalObservation = {
  readonly settles_active_prompt: boolean
  readonly passive_telemetry: boolean
}


export type TerminalHistoryBlobState = {
  readonly blobId: string
  readonly blobIds?: readonly string[]
  readonly agentId: string
  readonly title: string
  readonly summary: string
  /** Tool kind icon and call id from the blob's preview (protocol ≥508). */
  readonly icon?: string
  readonly callId?: string
  /** Past-tense title for a call still running when its turn ended. */
  readonly endedTitle?: string
  readonly kind: string
  readonly totalChars: number
  loaded: boolean
  loading: boolean
  reconcilingLiveEntries?: boolean
  error: string | null
}


export type TerminalTurnState = {
  readonly id: string
  readonly agentId: string
  startedAtMs: number
  lastEntryAtMs: number
  completedAtMs?: number
  kernelObservedActive: boolean
  kernelSettled?: boolean
  kernelSettlementStatus?: "completed" | "cancelled" | "failed"
  historyObserved?: boolean
  kernelPromptId?: string
  kernelProviderRunId?: string
  kernelSourceAttachmentId?: string | null
  kernelPromptOrigin?: string
  kernelExternalProvider?: string
  kernelExternalProviderSessionId?: string
  kernelExternalProviderTurnId?: string
  undoAccepted?: boolean
  expanded: boolean
}


export function terminalTurnExpansionEntryKey(turnId: string): string {
  return `terminal-turn-expanded:${turnId}`
}
