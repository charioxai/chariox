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

export type ExternalProviderObservedId = {
  provider: string
  providerSessionId: string
  providerTurnId: string
}

export type ExternalProviderObservedTurnMarker = {
  provider: string
  providerSessionId: string
}

export type ExternalProviderObservedTranscriptMetadata = {
  source: typeof EXTERNAL_PROVIDER_OBSERVED_SOURCE
  externalProvider: string | null
  externalProviderSessionId: string | null
  externalProviderTurnId: string | null
  observedAtMs: number | null
  externalObservation: SessionHistoryExternalObservation | null
}

export type ExternalProviderObservedIdentityFields = {
  readonly promptId?: string | null
  readonly externalProvider?: string | null
  readonly externalProviderSessionId?: string | null
  readonly externalProviderTurnId?: string | null
}

export type ExternalProviderObservedIdentityKey = {
  readonly provider: string
  readonly providerSessionId: string
  readonly providerTurnId: string
}

export function sessionHistoryEntryIsExternalProviderObserved(
  entry: { readonly source?: string | null | undefined },
): boolean {
  return normalizeExternalProviderObservedSource(entry.source) === EXTERNAL_PROVIDER_OBSERVED_SOURCE
}

export function parseExternalProviderObservedId(
  value: string | null | undefined,
): ExternalProviderObservedId | null {
  const parts = value?.split(":")
  if (!parts || parts.length < 4 || parts[0] !== "external") {
    return null
  }
  const provider = normalizeExternalProviderId(parts[1]) ?? ""
  const providerSessionId = parts[2]?.trim() ?? ""
  const providerTurnId = parts.slice(3).join(":").trim()
  if (!provider || !providerSessionId || !providerTurnId) {
    return null
  }
  return {
    provider,
    providerSessionId,
    providerTurnId,
  }
}

export function externalProviderObservedIdentityIsPresent(
  value: ExternalProviderObservedIdentityFields,
): boolean {
  return externalProviderObservedIdentityKey(value) !== null
}

export function externalProviderObservedIdentityKey(
  value: ExternalProviderObservedIdentityFields,
): ExternalProviderObservedIdentityKey | null {
  const promptIdentity = parseExternalProviderObservedId(value.promptId)
  const provider = normalizeExternalProviderId(value.externalProvider)
    ?? (promptIdentity ? normalizeExternalProviderId(promptIdentity.provider) : null)
    ?? ""
  const providerSessionId = nonBlankString(value.externalProviderSessionId)
    ?? promptIdentity?.providerSessionId
    ?? ""
  const providerTurnId = nonBlankString(value.externalProviderTurnId)
    ?? promptIdentity?.providerTurnId
    ?? ""
  if (!providerSessionId && !providerTurnId) {
    return null
  }
  return {
    provider,
    providerSessionId,
    providerTurnId,
  }
}

export function externalProviderObservedExactIdentityKey(
  value: ExternalProviderObservedIdentityFields,
): ExternalProviderObservedIdentityKey | null {
  const key = externalProviderObservedIdentityKey(value)
  if (!key?.provider || !key.providerSessionId || !key.providerTurnId) {
    return null
  }
  return key
}

export function externalProviderObservedIdentityMatches(
  candidate: ExternalProviderObservedIdentityFields,
  expected: ExternalProviderObservedIdentityFields,
): boolean {
  const expectedKey = externalProviderObservedIdentityKey(expected)
  if (!expectedKey) {
    return false
  }

  const candidateKey = externalProviderObservedIdentityKey(candidate)
  if (!candidateKey) {
    return false
  }
  if (expectedKey.provider && candidateKey.provider && candidateKey.provider !== expectedKey.provider) {
    return false
  }
  if (expectedKey.providerSessionId && candidateKey.providerSessionId !== expectedKey.providerSessionId) {
    return false
  }
  if (expectedKey.providerTurnId && candidateKey.providerTurnId !== expectedKey.providerTurnId) {
    return false
  }
  return true
}

export function historyEntryExternalProviderObservedMetadata(
  entry: ExternalProviderObservedKernelFields,
): ExternalProviderObservedTranscriptMetadata | null {
  if (!sessionHistoryEntryIsExternalProviderObserved(entry)) {
    return null
  }
  const mergeKeyIdentity = parseExternalProviderObservedId(entry.merge_key)
  return {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: normalizeExternalProviderId(entry.external_provider)
      ?? (mergeKeyIdentity ? normalizeExternalProviderId(mergeKeyIdentity.provider) : null),
    externalProviderSessionId:
      nonBlankString(entry.external_provider_session_id) ?? mergeKeyIdentity?.providerSessionId ?? null,
    externalProviderTurnId: nonBlankString(entry.external_provider_turn_id) ?? mergeKeyIdentity?.providerTurnId ?? null,
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
  if (!externalProviderObservedEntryCanBePassiveTelemetry(entry)) {
    return false
  }
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

export function promptOriginExternalProviderObservedMetadata(
  record: ExternalProviderObservedPromptOriginFields,
): ExternalProviderObservedTurnMetadata | null {
  if (!promptOriginIsExternal(promptOriginFromRecord(record))) {
    return null
  }
  const promptIdentity = parseExternalProviderObservedId(record.id ?? record.prompt_id)
  return {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: normalizeExternalProviderId(record.external_provider) ?? promptIdentity?.provider ?? null,
    externalProviderSessionId: nonBlankString(record.external_provider_session_id)
      ?? promptIdentity?.providerSessionId
      ?? null,
    externalProviderTurnId: nonBlankString(record.external_provider_turn_id) ?? promptIdentity?.providerTurnId ?? null,
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
  if (!hasExternalProviderObservedHistoryMetadata(incoming)) {
    return target
  }
  if (incoming.source !== undefined) {
    const source = mergeExternalProviderObservedSource(target.source, incoming.source)
    if (source !== undefined) {
      target.source = source
    }
  }
  if (fieldCanAcceptExternalProviderMetadata(target.external_provider) && incoming.external_provider != null) {
    target.external_provider = incoming.external_provider
  }
  if (
    fieldCanAcceptExternalProviderMetadata(target.external_provider_session_id)
    && incoming.external_provider_session_id != null
  ) {
    target.external_provider_session_id = incoming.external_provider_session_id
  }
  if (
    fieldCanAcceptExternalProviderMetadata(target.external_provider_turn_id)
    && incoming.external_provider_turn_id != null
  ) {
    target.external_provider_turn_id = incoming.external_provider_turn_id
  }
  if (fieldCanAcceptExternalProviderMetadata(target.observed_at_ms) && incoming.observed_at_ms != null) {
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

function hasExternalProviderObservedHistoryMetadata(incoming: ExternalProviderObservedKernelFields): boolean {
  return incoming.source !== undefined
    || incoming.external_provider !== undefined
    || incoming.external_provider_session_id !== undefined
    || incoming.external_provider_turn_id !== undefined
    || incoming.observed_at_ms !== undefined
    || incoming.external_observation !== undefined
}

export function mergeExternalProviderObservedTranscriptFields<T extends ExternalProviderObservedMutableTranscriptFields>(
  target: T,
  older: ExternalProviderObservedTranscriptFields,
  newer: ExternalProviderObservedTranscriptFields = target,
): T {
  const targetSource = (target as { readonly source?: string | null | undefined }).source
  if (
    !sessionHistoryEntryIsExternalProviderObserved(older)
    && !sessionHistoryEntryIsExternalProviderObserved(newer)
    && !sessionHistoryEntryIsExternalProviderObserved({ source: targetSource })
  ) {
    return target
  }
  if (fieldCanAcceptExternalProviderMetadata(target.externalProvider) && older.externalProvider != null) {
    target.externalProvider = older.externalProvider
  }
  if (
    fieldCanAcceptExternalProviderMetadata(target.externalProviderSessionId)
    && older.externalProviderSessionId != null
  ) {
    target.externalProviderSessionId = older.externalProviderSessionId
  }
  if (
    fieldCanAcceptExternalProviderMetadata(target.externalProviderTurnId)
    && older.externalProviderTurnId != null
  ) {
    target.externalProviderTurnId = older.externalProviderTurnId
  }
  if (fieldCanAcceptExternalProviderMetadata(target.observedAtMs) && older.observedAtMs != null) {
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

export function applyExternalProviderObservedTranscriptMetadata<
  T extends ExternalProviderObservedMutableTranscriptMetadataFields,
>(
  target: T,
  metadata: ExternalProviderObservedTranscriptFields,
): T {
  if (metadata.source !== undefined) {
    const source = mergeExternalProviderObservedSource(target.source, metadata.source)
    if (source !== undefined) {
      target.source = source
    }
  }
  if (metadata.externalProvider !== undefined) {
    target.externalProvider = metadata.externalProvider
  }
  if (metadata.externalProviderSessionId !== undefined) {
    target.externalProviderSessionId = metadata.externalProviderSessionId
  }
  if (metadata.externalProviderTurnId !== undefined) {
    target.externalProviderTurnId = metadata.externalProviderTurnId
  }
  if (metadata.observedAtMs !== undefined) {
    target.observedAtMs = metadata.observedAtMs
  }
  if (metadata.externalObservation !== undefined) {
    const externalObservation = mergeExternalProviderObservation(
      target.externalObservation,
      metadata.externalObservation,
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
  const source = mergeExternalProviderObservedSource(target.source, metadata.source)
  if (source !== undefined) {
    target.source = source
  }
  if (fieldCanAcceptExternalProviderMetadata(target.externalProvider) && metadata.externalProvider != null) {
    target.externalProvider = metadata.externalProvider
  }
  if (
    fieldCanAcceptExternalProviderMetadata(target.externalProviderSessionId)
    && metadata.externalProviderSessionId != null
  ) {
    target.externalProviderSessionId = metadata.externalProviderSessionId
  }
  if (
    fieldCanAcceptExternalProviderMetadata(target.externalProviderTurnId)
    && metadata.externalProviderTurnId != null
  ) {
    target.externalProviderTurnId = metadata.externalProviderTurnId
  }
  return target
}

export type ExternalProviderObservedKernelFields = {
  readonly kind?: string | null | undefined
  readonly merge_key?: string | null | undefined
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
  readonly observedAtMs?: number | null | undefined
  readonly externalObservation?: SessionHistoryExternalObservation | null | undefined
} & ExternalProviderObservedTranscriptIdentityFields

export type ExternalProviderObservedTranscriptIdentityFields = {
  readonly externalProvider?: string | null | undefined
  readonly externalProviderSessionId?: string | null | undefined
  readonly externalProviderTurnId?: string | null | undefined
}

export type ExternalProviderObservedMutableTranscriptIdentityFields = {
  externalProvider?: string | null | undefined
  externalProviderSessionId?: string | null | undefined
  externalProviderTurnId?: string | null | undefined
}

export type ExternalProviderObservedMutableTranscriptFields =
  ExternalProviderObservedMutableTranscriptIdentityFields & {
  observedAtMs?: number | null | undefined
  externalObservation?: SessionHistoryExternalObservation | null | undefined
}

export type ExternalProviderObservedMutableTranscriptMetadataFields =
  ExternalProviderObservedMutableTranscriptFields & {
  source?: string | null | undefined
}

export type ExternalProviderObservedMutableTurnMetadataFields = {
  source?: string | null | undefined
} & ExternalProviderObservedMutableTranscriptIdentityFields

export type ExternalProviderObservedStatusSignalFields = Pick<
  ExternalProviderObservedKernelFields,
  "kind" | "source"
>

export type ExternalProviderObservedProviderStatusFields = ExternalProviderObservedKernelFields & {
  readonly text: string
}

export type ExternalProviderObservedObservationFields = {
  readonly kind?: string | null | undefined
  readonly role?: string | null | undefined
  readonly merge_key?: string | null | undefined
  readonly source?: string | null | undefined
  readonly text?: string | null | undefined
  readonly external_provider?: string | null | undefined
  readonly externalProvider?: string | null | undefined
  readonly external_observation?: SessionHistoryExternalObservation | null | undefined
  readonly externalObservation?: SessionHistoryExternalObservation | null | undefined
}

export type ExternalProviderObservedStatusSettlementFields = ExternalProviderObservedObservationFields

export type ExternalProviderObservedCompletionTimeFields = {
  readonly observed_at_ms?: number | null | undefined
  readonly observedAtMs?: number | null | undefined
  readonly created_at_ms?: number | null | undefined
  readonly createdAtMs?: number | null | undefined
}

export type ExternalProviderObservedPromptOriginFields = PromptOriginRecord & {
  readonly id?: string | null | undefined
  readonly prompt_id?: string | null | undefined
  readonly external_provider?: string | null | undefined
  readonly external_provider_session_id?: string | null | undefined
  readonly external_provider_turn_id?: string | null | undefined
}

export type ExternalProviderObservedTranscriptTurnFields =
  ExternalProviderObservedTranscriptIdentityFields & {
  readonly source?: string | null | undefined
}

export function transcriptExternalProviderObservedTurnMetadata(
  entry: ExternalProviderObservedTranscriptTurnFields,
): ExternalProviderObservedTurnMetadata | null {
  if (!sessionHistoryEntryIsExternalProviderObserved(entry)) {
    return null
  }
  return {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: normalizeExternalProviderId(entry.externalProvider),
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

function normalizeExternalProviderId(value: string | null | undefined): string | null {
  return nonBlankString(value)?.toLowerCase() ?? null
}

function fieldCanAcceptExternalProviderMetadata(value: unknown): boolean {
  return value === undefined || value === null
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

function externalProviderObservedEntryCanBePassiveTelemetry(
  entry: ExternalProviderObservedObservationFields,
): boolean {
  return externalProviderObservedEntryIsStatus(entry)
    || entry.kind === "prompt_echo"
    || entry.kind === "user_prompt"
    || entry.role === "user"
}
