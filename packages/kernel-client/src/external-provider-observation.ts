import type { SessionHistoryEntry, SessionHistoryExternalObservation } from "./kernel-types.js"

export const EXTERNAL_PROVIDER_OBSERVED_SOURCE = "external_provider_observed"

export type ExternalProviderObservedTranscriptMetadata = {
  source: typeof EXTERNAL_PROVIDER_OBSERVED_SOURCE
  externalProvider: string | null
  externalProviderSessionId: string | null
  externalProviderTurnId: string | null
  observedAtMs: number | null
  externalObservation: SessionHistoryExternalObservation | null
}

export function historyEntryExternalProviderObservedMetadata(
  entry: Pick<
    SessionHistoryEntry,
    | "source"
    | "external_provider"
    | "external_provider_session_id"
    | "external_provider_turn_id"
    | "observed_at_ms"
    | "external_observation"
  >,
): ExternalProviderObservedTranscriptMetadata | null {
  if (entry.source !== EXTERNAL_PROVIDER_OBSERVED_SOURCE) {
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
