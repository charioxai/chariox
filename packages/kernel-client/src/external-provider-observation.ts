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

export type ExternalProviderObservedTurnMarker = {
  provider: string
  providerSessionId: string
}

export type ExternalProviderImportMatchFields = {
  readonly external_provider_session_id: string
  readonly external_provider: string
  readonly external_provider_session_provider_id: string
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
  return normalizeExternalProviderObservedSource(entry.source) === EXTERNAL_PROVIDER_OBSERVED_SOURCE
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

export function externalProviderObservedEntryIsPassiveTelemetry(
  entry: ExternalProviderObservedObservationFields,
): boolean {
  return externalProviderObservedObservation(entry)?.passive_telemetry === true
}

export function externalProviderObservedStatusSettlesActivePrompt(
  entry: ExternalProviderObservedStatusSettlementFields,
): boolean {
  return externalProviderObservedEntryIsStatus(entry)
    && sessionHistoryEntryIsExternalProviderObserved(entry)
    && externalProviderObservedObservation(entry)?.settles_active_prompt === true
}

export function externalProviderObservedCompletionAtMs(
  entry: ExternalProviderObservedCompletionTimeFields,
  nowMs: () => number,
): number {
  return finiteNumber(entry.observedAtMs)
    ?? finiteNumber(entry.observed_at_ms)
    ?? finiteNumber(entry.createdAtMs)
    ?? finiteNumber(entry.created_at_ms)
    ?? nowMs()
}

export function externalProviderObservedEntryBelongsToImport(
  externalImport: ExternalProviderImportMatchFields | null | undefined,
  entry: ExternalProviderObservedImportScopedEntryFields,
): boolean {
  if (!sessionHistoryEntryIsExternalProviderObserved(entry)) {
    return true
  }
  const entryProvider = nonBlankString(entry.externalProvider)
  const entrySessionId = nonBlankString(entry.externalProviderSessionId)
  if (!entryProvider && !entrySessionId) {
    return true
  }
  if (!externalImport) {
    return true
  }
  const importProvider = nonBlankString(externalImport.external_provider)
  if (entryProvider && importProvider && entryProvider !== importProvider) {
    return false
  }
  if (!entrySessionId) {
    return true
  }
  return entrySessionId === externalImport.external_provider_session_id
    || entrySessionId === externalImport.external_provider_session_provider_id
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
  const normalizedExisting = existing ? normalizedExternalObservation(existing) : existing
  const normalizedIncoming = incoming ? normalizedExternalObservation(incoming) : incoming
  if (!normalizedExisting) {
    return normalizedIncoming
  }
  if (!normalizedIncoming) {
    return normalizedExisting
  }
  const settlesActivePrompt = normalizedExisting.settles_active_prompt === true
    || normalizedIncoming.settles_active_prompt === true
  return {
    settles_active_prompt: settlesActivePrompt,
    passive_telemetry: settlesActivePrompt
      ? false
      : normalizedExisting.passive_telemetry === true || normalizedIncoming.passive_telemetry === true,
  }
}

export function mergeExternalProviderObservedSource(
  existing: string | null | undefined,
  incoming: string | null | undefined,
): string | null | undefined {
  if (sessionHistoryEntryIsExternalProviderObserved({ source: incoming })) {
    return EXTERNAL_PROVIDER_OBSERVED_SOURCE
  }
  if (sessionHistoryEntryIsExternalProviderObserved({ source: existing })) {
    return EXTERNAL_PROVIDER_OBSERVED_SOURCE
  }
  return existing ?? incoming
}

export function mergeExternalProviderObservedHistoryFields<T extends ExternalProviderObservedMutableKernelFields>(
  target: T,
  incoming: ExternalProviderObservedKernelFields,
): T {
  if (incoming.external_observation === undefined && incoming.source === undefined) {
    return target
  }
  if (incoming.source !== undefined) {
    const source = mergeExternalProviderObservedSource(target.source, incoming.source)
    if (source !== undefined) {
      target.source = source
    }
  }
  if (target.external_provider === undefined && incoming.external_provider !== undefined) {
    target.external_provider = incoming.external_provider
  }
  if (target.external_provider_session_id === undefined && incoming.external_provider_session_id !== undefined) {
    target.external_provider_session_id = incoming.external_provider_session_id
  }
  if (target.external_provider_turn_id === undefined && incoming.external_provider_turn_id !== undefined) {
    target.external_provider_turn_id = incoming.external_provider_turn_id
  }
  if (target.observed_at_ms === undefined && incoming.observed_at_ms !== undefined) {
    target.observed_at_ms = incoming.observed_at_ms
  }
  const externalObservation = mergeExternalProviderObservation(
    target.external_observation,
    incoming.external_observation,
  )
  if (externalObservation !== undefined) {
    target.external_observation = externalObservation
  }
  return target
}

export function mergeExternalProviderObservedTranscriptFields<T extends ExternalProviderObservedMutableTranscriptFields>(
  target: T,
  older: ExternalProviderObservedTranscriptFields,
  newer: ExternalProviderObservedTranscriptFields = target,
): T {
  if (!sessionHistoryEntryIsExternalProviderObserved(older)) {
    return target
  }
  if (target.externalProvider === undefined && older.externalProvider !== undefined) {
    target.externalProvider = older.externalProvider
  }
  if (target.externalProviderSessionId === undefined && older.externalProviderSessionId !== undefined) {
    target.externalProviderSessionId = older.externalProviderSessionId
  }
  if (target.externalProviderTurnId === undefined && older.externalProviderTurnId !== undefined) {
    target.externalProviderTurnId = older.externalProviderTurnId
  }
  if (target.observedAtMs === undefined && older.observedAtMs !== undefined) {
    target.observedAtMs = older.observedAtMs
  }
  if (older.externalObservation !== undefined || newer.externalObservation !== undefined) {
    const externalObservation = mergeExternalProviderObservation(
      older.externalObservation,
      newer.externalObservation,
    )
    if (externalObservation !== undefined) {
      target.externalObservation = externalObservation
    }
  }
  return target
}

export function applyExternalProviderObservedTurnMetadata<T extends ExternalProviderObservedMutableTurnMetadataFields>(
  target: T,
  metadata: ExternalProviderObservedTurnMetadata | null | undefined,
): T {
  if (!metadata) {
    return target
  }
  if (target.source === undefined || target.source === null) {
    target.source = metadata.source
  }
  if (target.externalProvider === undefined && metadata.externalProvider !== null) {
    target.externalProvider = metadata.externalProvider
  }
  if (target.externalProviderSessionId === undefined && metadata.externalProviderSessionId !== null) {
    target.externalProviderSessionId = metadata.externalProviderSessionId
  }
  if (target.externalProviderTurnId === undefined && metadata.externalProviderTurnId !== null) {
    target.externalProviderTurnId = metadata.externalProviderTurnId
  }
  return target
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

export type ExternalProviderObservedMutableKernelFields = {
  source?: string | null | undefined
  external_provider?: string | null | undefined
  external_provider_session_id?: string | null | undefined
  external_provider_turn_id?: string | null | undefined
  observed_at_ms?: number | null | undefined
  external_observation?: SessionHistoryExternalObservation | null | undefined
}

export type ExternalProviderObservedTranscriptFields = {
  readonly source?: string | null | undefined
  readonly externalProvider?: string | null | undefined
  readonly externalProviderSessionId?: string | null | undefined
  readonly externalProviderTurnId?: string | null | undefined
  readonly observedAtMs?: number | null | undefined
  readonly externalObservation?: SessionHistoryExternalObservation | null | undefined
}

export type ExternalProviderObservedMutableTranscriptFields = {
  externalProvider?: string | null | undefined
  externalProviderSessionId?: string | null | undefined
  externalProviderTurnId?: string | null | undefined
  observedAtMs?: number | null | undefined
  externalObservation?: SessionHistoryExternalObservation | null | undefined
}

export type ExternalProviderObservedMutableTurnMetadataFields = {
  source?: string | null | undefined
  externalProvider?: string | null | undefined
  externalProviderSessionId?: string | null | undefined
  externalProviderTurnId?: string | null | undefined
}

export type ExternalProviderObservedStatusSignalFields = Pick<
  ExternalProviderObservedKernelFields,
  "kind" | "source"
>

export type ExternalProviderObservedProviderStatusFields = ExternalProviderObservedKernelFields & {
  readonly text: string
}

export type ExternalProviderObservedObservationFields = {
  readonly source?: string | null | undefined
  readonly external_observation?: SessionHistoryExternalObservation | null | undefined
  readonly externalObservation?: SessionHistoryExternalObservation | null | undefined
}

export type ExternalProviderObservedStatusSettlementFields =
  ExternalProviderObservedObservationFields & {
    readonly kind?: string | null | undefined
    readonly role?: string | null | undefined
  }

export type ExternalProviderObservedCompletionTimeFields = {
  readonly observed_at_ms?: number | null | undefined
  readonly observedAtMs?: number | null | undefined
  readonly created_at_ms?: number | null | undefined
  readonly createdAtMs?: number | null | undefined
}

export type ExternalProviderObservedImportScopedEntryFields = {
  readonly source?: string | null | undefined
  readonly externalProvider?: string | null | undefined
  readonly externalProviderSessionId?: string | null | undefined
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

export function transcriptExternalProviderObservedTurnMarker(
  entries: readonly ExternalProviderObservedTranscriptTurnFields[],
): ExternalProviderObservedTurnMarker | null {
  const metadata = entries
    .map(transcriptExternalProviderObservedTurnMetadata)
    .find((candidate) => candidate !== null)
  if (!metadata) {
    return null
  }
  return {
    provider: metadata.externalProvider ?? "provider",
    providerSessionId: metadata.externalProviderSessionId ?? "unknown",
  }
}

function nonBlankString(value: string | null | undefined): string | null {
  const trimmed = value?.trim()
  return trimmed ? trimmed : null
}

function normalizeExternalProviderObservedSource(value: string | null | undefined): string | null {
  return nonBlankString(value)?.toLowerCase() ?? null
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

function externalProviderObservedObservation(
  entry: ExternalProviderObservedObservationFields,
): SessionHistoryExternalObservation | null {
  if (!sessionHistoryEntryIsExternalProviderObserved(entry)) {
    return null
  }
  return normalizedExternalObservation(entry.externalObservation ?? entry.external_observation)
}

function externalProviderObservedEntryIsStatus(
  entry: ExternalProviderObservedStatusSettlementFields,
): boolean {
  return entry.kind === "provider_status" || entry.role === "status"
}
