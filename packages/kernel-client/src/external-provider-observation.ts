import type { SessionHistoryExternalObservation } from "./kernel-types.js"

export const EXTERNAL_PROVIDER_OBSERVED_SOURCE = "external_provider_observed"

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

export type ExternalProviderObservedKernelFields = {
  readonly source?: string | null | undefined
  readonly external_provider?: string | null | undefined
  readonly external_provider_session_id?: string | null | undefined
  readonly external_provider_turn_id?: string | null | undefined
  readonly observed_at_ms?: number | null | undefined
  readonly external_observation?: SessionHistoryExternalObservation | null | undefined
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
  return {
    settles_active_prompt: value.settles_active_prompt === true,
    passive_telemetry: value.passive_telemetry === true,
  }
}
