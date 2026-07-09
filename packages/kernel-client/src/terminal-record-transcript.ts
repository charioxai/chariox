import {
  applyTranscriptPromptMetadata,
  kernelRecordTranscriptMetadata,
  presentTranscriptPromptMetadataFields,
  type TranscriptPromptMetadataTarget,
} from "./transcript-entry-state.js"
import {
  applyExternalProviderObservedTranscriptMetadata,
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  externalProviderObservedHistoryRefreshSignal,
  externalProviderObservedEntryIsPassiveTelemetry,
  externalProviderObservedProviderStatusShouldRender,
  historyEntryExternalProviderObservedMetadata,
  sessionHistoryEntryIsExternalProviderObserved,
  type ExternalProviderObservedMutableTranscriptMetadataFields,
  type ExternalProviderObservedTranscriptMetadata,
  type ExternalProviderObservedTranscriptFields,
} from "./external-provider-observation.js"
import { providerTranscriptRoleForKind } from "./transcript-kind-role.js"

export type TerminalRecordTranscriptFields = {
  readonly prompt_id?: string | null
  readonly prompt_origin?: string | null
  readonly source_attachment_id?: string | null
  readonly kind: string
  readonly source?: string | null
  readonly external_provider?: string | null
  readonly external_provider_session_id?: string | null
  readonly external_provider_turn_id?: string | null
  readonly observed_at_ms?: number | null
  readonly external_observation?: ExternalProviderObservedTranscriptMetadata["externalObservation"] | null
  readonly text?: string | null
}

export type TerminalRecordTranscriptRole =
  | "assistant"
  | "error"
  | "reasoning"
  | "status"
  | "tool"
  | "user"

export type TerminalRecordTranscriptMetadata = ExternalProviderObservedTranscriptFields & {
  readonly promptId?: string | null
  readonly promptOrigin?: string | null
  readonly sourceAttachmentId?: string | null
}

export type TerminalRecordTranscriptProjection = {
  readonly metadata: TerminalRecordTranscriptMetadata
  readonly role: TerminalRecordTranscriptRole | null
  readonly text: string
  readonly historyRefreshSignal: boolean
  readonly passiveExternalTelemetry: boolean
  readonly startsStreaming: boolean
  readonly marksAgentBusy: boolean
  readonly providerStatusIdle: boolean
  readonly updatesProviderActivity: boolean
  readonly appendsLiveTranscript: boolean
  readonly renderProviderStatus: boolean
  readonly transcriptRole: TerminalRecordTranscriptRole | null
  readonly transcriptText: string
  readonly mergeKey?: string | null
  readonly statusMergeKey: "__provider_status__" | null
  readonly renderInAgentPane: boolean
  readonly append: boolean
  readonly replace: boolean
}

export function terminalRecordTranscriptMetadata(
  record: TerminalRecordTranscriptFields,
): TerminalRecordTranscriptMetadata {
  const externalObservedMetadata = historyEntryExternalProviderObservedMetadata(record)
  return {
    ...presentTranscriptPromptMetadataFields(kernelRecordTranscriptMetadata(record)),
    ...(externalObservedMetadata ?? {}),
  }
}

export function transcriptEntryWithTerminalMetadata<TEntry extends TerminalTranscriptMetadataTarget>(
  entry: TEntry,
  metadata: TerminalRecordTranscriptMetadata,
): TEntry {
  const next = { ...entry }
  applyTranscriptPromptMetadata(next, metadata)
  applyExternalProviderObservedTranscriptMetadata(next, metadata)
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
  const passiveExternalTelemetry = terminalRecordIsPassiveExternalProviderTelemetry({ ...record, text })
  const providerStatusIdle = record.kind === "provider_status" && options.isProviderIdleStatus(text)
  const renderProviderStatus = record.kind === "provider_status"
    && !historyRefreshSignal
    && !providerStatusIdle
    ? terminalRecordProviderStatusShouldRender(record, text, options.shouldRenderProviderStatus)
    : false
  const transcriptRole = terminalRecordTranscriptRole(record.kind)
  const transcriptText = record.kind === "provider_error" ? normalizeTerminalRecordErrorText(text) : text
  const renderInAgentPane = !historyRefreshSignal
    && !passiveExternalTelemetry
    && !providerStatusIdle
    && terminalRecordShouldRenderInAgentPane(record.kind, text, {
      externalObserved: sessionHistoryEntryIsExternalProviderObserved(metadata),
      passiveExternalTelemetry,
    })
  const updatesProviderActivity = record.kind === "provider_status"
    && !historyRefreshSignal
    && !passiveExternalTelemetry
    && !providerStatusIdle
  const startsOrMarksLiveTurn = !terminalRecordKindIsUserPrompt(record.kind)
    && (renderInAgentPane || updatesProviderActivity)

  return {
    metadata,
    role: transcriptRole,
    text: transcriptText,
    historyRefreshSignal,
    passiveExternalTelemetry,
    startsStreaming: startsOrMarksLiveTurn,
    marksAgentBusy: startsOrMarksLiveTurn,
    providerStatusIdle,
    updatesProviderActivity,
    appendsLiveTranscript: renderInAgentPane,
    renderProviderStatus,
    transcriptRole,
    transcriptText,
    mergeKey: record.merge_key ?? null,
    statusMergeKey: record.kind === "provider_status" && !sessionHistoryEntryIsExternalProviderObserved(metadata)
      ? "__provider_status__"
      : null,
    renderInAgentPane,
    append: renderInAgentPane && terminalRecordRoleShouldAppend(transcriptRole),
    replace: renderInAgentPane && terminalRecordRoleShouldReplace(transcriptRole),
  }
}

export function terminalRecordPromptHistoryText(
  record: TerminalRecordTranscriptFields,
  text: string,
): string | null {
  if (terminalRecordTranscriptRole(record.kind) !== "user") {
    return null
  }
  if (externalProviderObservedHistoryRefreshSignal(record, text)) {
    return null
  }
  if (terminalRecordIsPassiveExternalProviderTelemetry(record)) {
    return null
  }
  return text
}

export function terminalRecordProviderStatusShouldRender(
  record: TerminalRecordTranscriptFields,
  text: string,
  fallbackShouldRender: (text: string) => boolean,
): boolean {
  if (record.kind !== "provider_status") {
    return false
  }
  if (historyEntryExternalProviderObservedMetadata(record) !== null) {
    return externalProviderObservedProviderStatusShouldRender({ ...record, text })
  }
  return fallbackShouldRender(text)
}

export function terminalRecordShouldRenderInAgentPane(
  kind: string,
  text: string,
  options: {
    readonly externalObserved?: boolean
    readonly passiveExternalTelemetry?: boolean
  } = {},
): boolean {
  if (kind === "provider_status") {
    return terminalRecordProviderStatusShouldRender({
      kind,
      source: options.externalObserved === true ? EXTERNAL_PROVIDER_OBSERVED_SOURCE : null,
      external_observation: options.passiveExternalTelemetry === true
        ? { settles_active_prompt: false, passive_telemetry: true }
        : null,
    }, text, () => false)
  }
  return kind !== "notice"
}

export function terminalRecordIsPassiveExternalProviderTelemetry(
  record: TerminalRecordTranscriptFields,
): boolean {
  return externalProviderObservedEntryIsPassiveTelemetry(record)
}

export function normalizeTerminalRecordErrorText(text: string): string {
  return text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").trim()
}

export function terminalRecordRoleShouldAppend(role: TerminalRecordTranscriptRole | null): boolean {
  return role === "assistant" || role === "reasoning"
}

export function terminalRecordRoleShouldReplace(role: TerminalRecordTranscriptRole | null): boolean {
  return role === "tool" || role === "status"
}

function terminalRecordTranscriptRole(kind: string): TerminalRecordTranscriptRole | null {
  if (terminalRecordKindIsUserPrompt(kind)) {
    return "user"
  }
  return providerTranscriptRoleForKind(kind) ?? "assistant"
}

function terminalRecordKindIsUserPrompt(kind: string): boolean {
  return kind === "prompt_echo" || kind === "user_prompt"
}

type TerminalTranscriptMetadataTarget =
  & ExternalProviderObservedMutableTranscriptMetadataFields
  & TranscriptPromptMetadataTarget
