import {
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
