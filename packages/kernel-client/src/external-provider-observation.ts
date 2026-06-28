import type { SessionHistoryExternalObservation } from "./kernel-types.js"
import {
  promptOriginFromRecord,
  promptOriginIsExternal,
  type PromptOriginRecord,
} from "./prompt-origin.js"
import { shouldRenderProviderStatus } from "./provider-status.js"

export const EXTERNAL_PROVIDER_OBSERVED_SOURCE = "external_provider_observed"
export const EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS = "external_provider_history_updated"

export type ExternalProviderObservedTurnMetadata = {
  source: typeof EXTERNAL_PROVIDER_OBSERVED_SOURCE
  externalProvider: string | null
  externalProviderSessionId: string | null
  externalProviderTurnId: string | null
}

export type ExternalProviderObservedTranscriptMetadata = {
  source: typeof EXTERNAL_PROVIDER_OBSERVED_SOURCE
  externalProvider: string | null
  externalProviderSessionId: string | null
  externalProviderTurnId: string | null
  observedAtMs: number | null
  externalObservation: SessionHistoryExternalObservation | null
}

export function sessionHistoryEntryIsExternalProviderObserved(
  entry: { readonly source?: string | null | undefined },
): boolean {
  return entry.source === EXTERNAL_PROVIDER_OBSERVED_SOURCE
}

export function historyEntryExternalProviderObservedMetadata(
  entry: ExternalProviderObservedKernelFields,
): ExternalProviderObservedTranscriptMetadata | null {
  if (!sessionHistoryEntryIsExternalProviderObserved(entry)) {
    return null
  }
  return {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: nonBlankString(entry.external_provider),
    externalProviderSessionId: nonBlankString(entry.external_provider_session_id),
    externalProviderTurnId: nonBlankString(entry.external_provider_turn_id),
    observedAtMs: finiteNumber(entry.observed_at_ms),
    externalObservation: normalizedExternalObservation(entry.external_observation),
  }
}

export function externalProviderObservedHistoryRefreshSignal(
  entry: ExternalProviderObservedStatusSignalFields,
  text: string,
): boolean {
  return entry.kind === "provider_status"
    && sessionHistoryEntryIsExternalProviderObserved(entry)
    && text.trim() === EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS
}

export function externalProviderObservedProviderStatusShouldRender(
  entry: ExternalProviderObservedProviderStatusFields,
): boolean {
  if (entry.kind !== "provider_status") {
    return false
  }
  const metadata = historyEntryExternalProviderObservedMetadata(entry)
  return metadata !== null
    && metadata.externalObservation?.passive_telemetry !== true
    && shouldRenderProviderStatus(entry.text)
}

export function promptOriginExternalProviderObservedMetadata(
  record: ExternalProviderObservedPromptOriginFields,
): ExternalProviderObservedTurnMetadata | null {
  if (!promptOriginIsExternal(promptOriginFromRecord(record))) {
    return null
  }
  return {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: nonBlankString(record.external_provider),
    externalProviderSessionId: nonBlankString(record.external_provider_session_id),
    externalProviderTurnId: nonBlankString(record.external_provider_turn_id),
  }
}

export function mergeExternalProviderObservation(
  existing: SessionHistoryExternalObservation | null | undefined,
  incoming: SessionHistoryExternalObservation | null | undefined,
): SessionHistoryExternalObservation | null | undefined {
  if (!existing) {
    return incoming
  }
  if (!incoming) {
    return existing
  }
  const settlesActivePrompt = existing.settles_active_prompt || incoming.settles_active_prompt
  return {
    settles_active_prompt: settlesActivePrompt,
    passive_telemetry: settlesActivePrompt
      ? false
      : existing.passive_telemetry === true || incoming.passive_telemetry === true,
  }
}

export type ExternalProviderObservedKernelFields = {
  readonly kind?: string | null | undefined
  readonly source?: string | null | undefined
  readonly external_provider?: string | null | undefined
  readonly external_provider_session_id?: string | null | undefined
  readonly external_provider_turn_id?: string | null | undefined
  readonly observed_at_ms?: number | null | undefined
  readonly external_observation?: SessionHistoryExternalObservation | null | undefined
}

export type ExternalProviderObservedStatusSignalFields = Pick<
  ExternalProviderObservedKernelFields,
  "kind" | "source"
>

export type ExternalProviderObservedProviderStatusFields = ExternalProviderObservedKernelFields & {
  readonly text: string
}

export type ExternalProviderObservedPromptOriginFields = PromptOriginRecord & {
  readonly external_provider_turn_id?: string | null | undefined
}

export type ExternalProviderObservedTranscriptTurnFields = {
  readonly source?: string | null | undefined
  readonly externalProvider?: string | null | undefined
  readonly externalProviderSessionId?: string | null | undefined
  readonly externalProviderTurnId?: string | null | undefined
}

export function transcriptExternalProviderObservedTurnMetadata(
  entry: ExternalProviderObservedTranscriptTurnFields,
): ExternalProviderObservedTurnMetadata | null {
  if (!sessionHistoryEntryIsExternalProviderObserved(entry)) {
    return null
  }
  return {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: nonBlankString(entry.externalProvider),
    externalProviderSessionId: nonBlankString(entry.externalProviderSessionId),
    externalProviderTurnId: nonBlankString(entry.externalProviderTurnId),
  }
}

function nonBlankString(value: string | null | undefined): string | null {
  const trimmed = value?.trim()
  return trimmed ? trimmed : null
}

function finiteNumber(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null
}

function normalizedExternalObservation(
  value: SessionHistoryExternalObservation | null | undefined,
): SessionHistoryExternalObservation | null {
  if (!value) {
    return null
  }
  const settlesActivePrompt = value.settles_active_prompt === true
  return {
    settles_active_prompt: settlesActivePrompt,
    passive_telemetry: settlesActivePrompt ? false : value.passive_telemetry === true,
  }
}
