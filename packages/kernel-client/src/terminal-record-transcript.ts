import {
  externalProviderObservedHistoryRefreshSignal,
  externalProviderObservedEntryIsPassiveTelemetry,
  externalProviderObservedProviderStatusShouldRender,
  historyEntryExternalProviderObservedMetadata,
  type ExternalProviderObservedTranscriptMetadata,
} from "./external-provider-observation.js"

export type TerminalRecordTranscriptFields = {
  readonly prompt_id?: string | null
  readonly source_attachment_id?: string | null
  readonly kind: string
  readonly source?: string | null
  readonly external_provider?: string | null
  readonly external_provider_session_id?: string | null
  readonly external_provider_turn_id?: string | null
  readonly observed_at_ms?: number | null
  readonly external_observation?: ExternalProviderObservedTranscriptMetadata["externalObservation"] | null
}

export type TerminalRecordTranscriptRole =
  | "assistant"
  | "error"
  | "reasoning"
  | "status"
  | "tool"
  | "user"

export type TerminalRecordTranscriptMetadata = {
  readonly promptId?: string | null
  readonly sourceAttachmentId?: string | null
  readonly source?: "external_provider_observed" | string | null
  readonly externalProvider?: string | null
  readonly externalProviderSessionId?: string | null
  readonly externalProviderTurnId?: string | null
  readonly observedAtMs?: number | null
  readonly externalObservation?: ExternalProviderObservedTranscriptMetadata["externalObservation"] | null
}

export type TerminalRecordTranscriptProjection = {
  readonly metadata: TerminalRecordTranscriptMetadata
  readonly historyRefreshSignal: boolean
  readonly passiveExternalTelemetry: boolean
  readonly startsStreaming: boolean
  readonly marksAgentBusy: boolean
  readonly providerStatusIdle: boolean
  readonly renderProviderStatus: boolean
  readonly transcriptRole: TerminalRecordTranscriptRole | null
  readonly transcriptText: string
  readonly mergeKey?: string | null
  readonly statusMergeKey: "__provider_status__" | null
}

export function terminalRecordTranscriptMetadata(
  record: TerminalRecordTranscriptFields,
): TerminalRecordTranscriptMetadata {
  const externalObservedMetadata = historyEntryExternalProviderObservedMetadata(record)
  return {
    ...(record.prompt_id !== undefined ? { promptId: record.prompt_id } : {}),
    ...(record.source_attachment_id !== undefined ? { sourceAttachmentId: record.source_attachment_id } : {}),
    ...(externalObservedMetadata ?? {}),
  }
}

export function transcriptEntryWithTerminalMetadata<TEntry extends TerminalTranscriptMetadataTarget>(
  entry: TEntry,
  metadata: TerminalRecordTranscriptMetadata,
): TEntry {
  const next = { ...entry }
  if (metadata.promptId !== undefined) next.promptId = metadata.promptId
  if (metadata.sourceAttachmentId !== undefined) next.sourceAttachmentId = metadata.sourceAttachmentId
  if (metadata.source !== undefined) next.source = metadata.source
  if (metadata.externalProvider !== undefined) next.externalProvider = metadata.externalProvider
  if (metadata.externalProviderSessionId !== undefined) next.externalProviderSessionId = metadata.externalProviderSessionId
  if (metadata.externalProviderTurnId !== undefined) next.externalProviderTurnId = metadata.externalProviderTurnId
  if (metadata.observedAtMs !== undefined) next.observedAtMs = metadata.observedAtMs
  if (metadata.externalObservation !== undefined) next.externalObservation = metadata.externalObservation
  return next
}

export function terminalRecordTranscriptProjection(
  record: TerminalRecordTranscriptFields & { readonly merge_key?: string | null },
  text: string,
  options: {
    readonly isProviderIdleStatus: (text: string) => boolean
    readonly shouldRenderProviderStatus: (text: string) => boolean
  },
): TerminalRecordTranscriptProjection {
  const metadata = terminalRecordTranscriptMetadata(record)
  const historyRefreshSignal = externalProviderObservedHistoryRefreshSignal(record, text)
  const passiveExternalTelemetry = terminalRecordIsPassiveExternalProviderTelemetry(record)
  const providerStatusIdle = record.kind === "provider_status" && options.isProviderIdleStatus(text)
  const renderProviderStatus = record.kind === "provider_status"
    ? terminalRecordProviderStatusShouldRender(record, text, options.shouldRenderProviderStatus)
    : false

  return {
    metadata,
    historyRefreshSignal,
    passiveExternalTelemetry,
    startsStreaming: record.kind !== "prompt_echo" && !historyRefreshSignal && !passiveExternalTelemetry,
    marksAgentBusy: record.kind !== "prompt_echo"
      && !historyRefreshSignal
      && !passiveExternalTelemetry
      && !providerStatusIdle,
    providerStatusIdle,
    renderProviderStatus,
    transcriptRole: terminalRecordTranscriptRole(record.kind),
    transcriptText: record.kind === "provider_error" ? normalizeTerminalRecordErrorText(text) : text,
    mergeKey: record.merge_key ?? null,
    statusMergeKey: record.kind === "provider_status" ? "__provider_status__" : null,
  }
}

export function terminalRecordProviderStatusShouldRender(
  record: TerminalRecordTranscriptFields,
  text: string,
  fallbackShouldRender: (text: string) => boolean,
): boolean {
  if (historyEntryExternalProviderObservedMetadata(record) !== null) {
    return externalProviderObservedProviderStatusShouldRender({ ...record, text })
  }
  return fallbackShouldRender(text)
}

export function terminalRecordIsPassiveExternalProviderTelemetry(
  record: TerminalRecordTranscriptFields,
): boolean {
  return externalProviderObservedEntryIsPassiveTelemetry(record)
}

export function normalizeTerminalRecordErrorText(text: string): string {
  return text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
}

function terminalRecordTranscriptRole(kind: string): TerminalRecordTranscriptRole | null {
  switch (kind) {
    case "prompt_echo":
      return "user"
    case "provider_reasoning":
      return "reasoning"
    case "provider_tool":
      return "tool"
    case "provider_error":
      return "error"
    case "provider_status":
      return "status"
    default:
      return "assistant"
  }
}

type TerminalTranscriptMetadataTarget = {
  promptId?: string | null
  sourceAttachmentId?: string | null
  source?: "external_provider_observed" | string | null
  externalProvider?: string | null
  externalProviderSessionId?: string | null
  externalProviderTurnId?: string | null
  observedAtMs?: number | null
  externalObservation?: ExternalProviderObservedTranscriptMetadata["externalObservation"] | null
}
