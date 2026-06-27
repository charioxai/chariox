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
    externalProvider: entry.external_provider ?? null,
    externalProviderSessionId: entry.external_provider_session_id ?? null,
    externalProviderTurnId: entry.external_provider_turn_id ?? null,
    observedAtMs: entry.observed_at_ms ?? null,
    externalObservation: entry.external_observation ?? null,
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
