import type {
  TerminalExternalObservation,
  TerminalOutputEntry,
} from "./runtime-state.js"
import {
  readNumberField,
  readStringField,
} from "./kernel-response-parsing.js"
import {
  terminalPromptOriginFromRecord,
  terminalPromptOriginIsExternal,
} from "./prompt-origin.js"
import {
  shouldRenderProviderStatus,
} from "@chariox/tool-display"

export const terminalExternalProviderObservedSource = "external_provider_observed"
export const terminalExternalProviderHistoryUpdatedStatus = "external_provider_history_updated"

export type ExternalProviderObservedTurnMarker = {
  readonly provider: string
  readonly providerSessionId: string
}

export type ExternalProviderObservedTurnMetadata = {
  readonly source: typeof terminalExternalProviderObservedSource
  readonly externalProvider: string | null
  readonly externalProviderSessionId: string | null
  readonly externalProviderTurnId: string | null
}

export type ExternalProviderObservedMetadata = {
  readonly source: typeof terminalExternalProviderObservedSource
  readonly externalProvider: string | null
  readonly externalProviderSessionId: string | null
  readonly externalProviderTurnId: string | null
  readonly observedAtMs: number | null
  readonly externalObservation: TerminalExternalObservation | null
}

export type ExternalProviderObservedIdentityFields = {
  readonly externalProvider?: string | null
  readonly externalProviderSessionId?: string | null
  readonly externalProviderTurnId?: string | null
}

export type ExternalProviderObservedIdentityKey = {
  readonly provider: string
  readonly providerSessionId: string
  readonly providerTurnId: string
}

export type ExternalProviderObservedTranscriptFields = {
  readonly source?: string | null | undefined
  readonly externalProvider?: string | null | undefined
  readonly externalProviderSessionId?: string | null | undefined
  readonly externalProviderTurnId?: string | null | undefined
  readonly observedAtMs?: number | null | undefined
  readonly externalObservation?: TerminalExternalObservation | null | undefined
}

export type ExternalProviderObservedMutableTranscriptFields = {
  source?: string | null | undefined
  externalProvider?: string | null | undefined
  externalProviderSessionId?: string | null | undefined
  externalProviderTurnId?: string | null | undefined
  observedAtMs?: number | null | undefined
  externalObservation?: TerminalExternalObservation | null | undefined
}

export type ExternalProviderObservedMutableTurnMetadataFields = {
  source?: string | null | undefined
  externalProvider?: string | null | undefined
  externalProviderSessionId?: string | null | undefined
  externalProviderTurnId?: string | null | undefined
}

export function isExternalProviderObservedSource(source: string | null | undefined): boolean {
  return normalizeExternalProviderObservedSource(source) === terminalExternalProviderObservedSource
}

export function externalProviderObservedIdentityIsPresent(
  value: ExternalProviderObservedIdentityFields,
): boolean {
  return externalProviderObservedIdentityKey(value) !== null
}

export function externalProviderObservedIdentityKey(
  value: ExternalProviderObservedIdentityFields,
): ExternalProviderObservedIdentityKey | null {
  const provider = normalizeExternalProviderId(value.externalProvider)
    ?? ""
  const providerSessionId = nonBlankString(value.externalProviderSessionId)
    ?? ""
  const providerTurnId = nonBlankString(value.externalProviderTurnId)
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

export function externalProviderObservedExactIdentityMatches(
  candidate: ExternalProviderObservedIdentityFields,
  expected: ExternalProviderObservedIdentityFields,
): boolean {
  const candidateKey = externalProviderObservedExactIdentityKey(candidate)
  const expectedKey = externalProviderObservedExactIdentityKey(expected)
  return Boolean(
    candidateKey
      && expectedKey
      && candidateKey.provider === expectedKey.provider
      && candidateKey.providerSessionId === expectedKey.providerSessionId
      && candidateKey.providerTurnId === expectedKey.providerTurnId,
  )
}

export function externalProviderObservedExactIdentityConflicts(
  candidate: ExternalProviderObservedIdentityFields,
  expected: ExternalProviderObservedIdentityFields,
): boolean {
  const candidateKey = externalProviderObservedExactIdentityKey(candidate)
  const expectedKey = externalProviderObservedExactIdentityKey(expected)
  if (!candidateKey || !expectedKey) {
    return false
  }
  return candidateKey.provider !== expectedKey.provider
    || candidateKey.providerSessionId !== expectedKey.providerSessionId
    || candidateKey.providerTurnId !== expectedKey.providerTurnId
}

export function externalProviderObservedTranscriptExactIdentityConflicts(
  left: ExternalProviderObservedTranscriptFields,
  right: ExternalProviderObservedTranscriptFields,
): boolean {
  if (
    !isExternalProviderObservedSource(left.source)
    && !isExternalProviderObservedSource(right.source)
  ) {
    return false
  }
  return externalProviderObservedExactIdentityConflicts(
    externalProviderObservedTranscriptIdentityFields(left),
    externalProviderObservedTranscriptIdentityFields(right),
  )
}

export function externalProviderObservedDisambiguatedKey(
  baseKey: string,
  identity: ExternalProviderObservedIdentityFields,
  keyExists: (key: string) => boolean,
): string {
  const exactIdentity = externalProviderObservedExactIdentityKey(identity)
  const disambiguationBase = exactIdentity
    ? [
        baseKey,
        "external",
        exactIdentity.provider,
        exactIdentity.providerSessionId,
        exactIdentity.providerTurnId,
      ].join(":")
    : `${baseKey}:external-conflict`
  let key = disambiguationBase
  let counter = 2
  while (keyExists(key)) {
    key = `${disambiguationBase}:${counter}`
    counter += 1
  }
  return key
}

export function externalProviderObservedTranscriptIdentityFields(
  entry: ExternalProviderObservedIdentityFields,
): ExternalProviderObservedIdentityFields {
  return {
    ...(entry.externalProvider !== undefined ? { externalProvider: entry.externalProvider } : {}),
    ...(entry.externalProviderSessionId !== undefined
      ? { externalProviderSessionId: entry.externalProviderSessionId }
      : {}),
    ...(entry.externalProviderTurnId !== undefined ? { externalProviderTurnId: entry.externalProviderTurnId } : {}),
  }
}

export function externalProviderObservedCoalescedTranscriptIdentityFields(
  primary: ExternalProviderObservedIdentityFields,
  fallback: ExternalProviderObservedIdentityFields = {},
): ExternalProviderObservedIdentityFields {
  const primaryFields = externalProviderObservedTranscriptIdentityFields(primary)
  const fallbackFields = externalProviderObservedTranscriptIdentityFields(fallback)
  return {
    ...(primaryFields.externalProvider !== undefined
      ? { externalProvider: primaryFields.externalProvider }
      : fallbackFields.externalProvider !== undefined
        ? { externalProvider: fallbackFields.externalProvider }
        : {}),
    ...(primaryFields.externalProviderSessionId !== undefined
      ? { externalProviderSessionId: primaryFields.externalProviderSessionId }
      : fallbackFields.externalProviderSessionId !== undefined
        ? { externalProviderSessionId: fallbackFields.externalProviderSessionId }
        : {}),
    ...(primaryFields.externalProviderTurnId !== undefined
      ? { externalProviderTurnId: primaryFields.externalProviderTurnId }
      : fallbackFields.externalProviderTurnId !== undefined
        ? { externalProviderTurnId: fallbackFields.externalProviderTurnId }
        : {}),
  }
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

export function isExternalProviderObservedRecord(
  value: unknown,
): boolean {
  return readExternalProviderObservedMetadata(value) !== null
}

export function externalProviderObservedEntryIsPassiveTelemetry(
  value: unknown,
): boolean {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return false
  }
  const record = value as Record<string, unknown>
  if (!isExternalProviderObservedSource(readStringField(record, "source"))) {
    return false
  }
  const kind = readStringField(record, "kind")
  const role = readStringField(record, "role")
  if (
    kind !== "provider_status"
    && kind !== "prompt_echo"
    && kind !== "user_prompt"
    && role !== "status"
    && role !== "user"
  ) {
    return false
  }
  return readExternalProviderObservation(record.externalObservation ?? record.external_observation)?.passive_telemetry === true
}

export function externalProviderObservedHistoryRefreshSignal(
  value: unknown,
  text: string,
): boolean {
  return readStringField(value, "kind") === "provider_status"
    && isExternalProviderObservedRecord(value)
    && text.trim() === terminalExternalProviderHistoryUpdatedStatus
}

export function externalProviderObservedProviderStatusShouldRender(
  value: {
    readonly kind?: string | null
    readonly source?: string | null
    readonly merge_key?: string | null
    readonly text?: string | null
    readonly external_provider?: string | null
    readonly externalProvider?: string | null
    readonly external_observation?: TerminalExternalObservation | null
    readonly externalObservation?: TerminalExternalObservation | null
  },
): boolean {
  if (value.kind !== "provider_status") {
    return false
  }
  if (!isExternalProviderObservedSource(value.source)) {
    return false
  }
  const observation = readExternalProviderObservation(value.externalObservation ?? value.external_observation)
  if (observation?.passive_telemetry === true) {
    return false
  }
  return value.text == null ? false : shouldRenderProviderStatus(value.text)
}

export function isExternalProviderObservedEntry(
  entry: Pick<TerminalOutputEntry, "source">,
): boolean {
  return isExternalProviderObservedSource(entry.source)
}

export function isExternalProviderObservedStatusEntry(
  entry: Pick<TerminalOutputEntry, "role" | "source">,
): boolean {
  return entry.role === "status" && isExternalProviderObservedEntry(entry)
}

export function isExternalProviderObservedPassiveTelemetryStatusEntry(
  entry: Pick<TerminalOutputEntry, "role" | "source" | "externalObservation">,
): boolean {
  return isExternalProviderObservedStatusEntry(entry)
    && entry.externalObservation?.passive_telemetry === true
}

export function mergeExternalProviderObservedSource(
  existing: string | null | undefined,
  incoming: string | null | undefined,
): string | null | undefined {
  if (isExternalProviderObservedSource(incoming)) {
    return terminalExternalProviderObservedSource
  }
  if (isExternalProviderObservedSource(existing)) {
    return terminalExternalProviderObservedSource
  }
  return existing ?? incoming
}

export function mergeExternalProviderObservation(
  existing: TerminalExternalObservation | null | undefined,
  incoming: TerminalExternalObservation | null | undefined,
): TerminalExternalObservation | null | undefined {
  const normalizedExisting = existing ? readExternalProviderObservation(existing) : existing
  const normalizedIncoming = incoming ? readExternalProviderObservation(incoming) : incoming
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

export function mergeExternalProviderObservedTranscriptFields<T extends ExternalProviderObservedMutableTranscriptFields>(
  target: T,
  older: ExternalProviderObservedTranscriptFields,
  newer: ExternalProviderObservedTranscriptFields = target,
): T {
  if (
    !isExternalProviderObservedSource(older.source)
    && !isExternalProviderObservedSource(newer.source)
    && !isExternalProviderObservedSource(target.source)
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

export function applyExternalProviderObservedTranscriptMetadata<T extends ExternalProviderObservedMutableTranscriptFields>(
  target: T,
  metadata: ExternalProviderObservedTranscriptFields,
): T {
  if (metadata.source !== undefined) {
    const source = mergeExternalProviderObservedSource(target.source, metadata.source)
    if (source !== undefined) {
      target.source = source
    }
  }
  if (fieldCanAcceptExternalProviderMetadata(target.externalProvider) && metadata.externalProvider !== undefined) {
    target.externalProvider = metadata.externalProvider
  }
  if (
    fieldCanAcceptExternalProviderMetadata(target.externalProviderSessionId)
    && metadata.externalProviderSessionId !== undefined
  ) {
    target.externalProviderSessionId = metadata.externalProviderSessionId
  }
  if (
    fieldCanAcceptExternalProviderMetadata(target.externalProviderTurnId)
    && metadata.externalProviderTurnId !== undefined
  ) {
    target.externalProviderTurnId = metadata.externalProviderTurnId
  }
  if (fieldCanAcceptExternalProviderMetadata(target.observedAtMs) && metadata.observedAtMs !== undefined) {
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

export function readExternalProviderObservation(
  value: unknown,
): TerminalExternalObservation | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null
  }
  const observation = value as Record<string, unknown>
  const settlesActivePrompt = observation.settles_active_prompt === true
  return {
    settles_active_prompt: settlesActivePrompt,
    passive_telemetry: settlesActivePrompt ? false : observation.passive_telemetry === true,
  }
}

export function readExternalProviderObservedMetadata(
  value: unknown,
): ExternalProviderObservedMetadata | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null
  }
  const source = readStringField(value, "source")
  if (!isExternalProviderObservedSource(source)) {
    return null
  }
  return {
    source: terminalExternalProviderObservedSource,
    externalProvider: normalizeExternalProviderId(readStringField(value, "external_provider"))
      ?? null,
    externalProviderSessionId:
      readNonBlankStringField(value, "external_provider_session_id") ?? null,
    externalProviderTurnId:
      readNonBlankStringField(value, "external_provider_turn_id") ?? null,
    observedAtMs: readNumberField(value, "observed_at_ms"),
    externalObservation: readExternalProviderObservation((value as Record<string, unknown>).external_observation),
  }
}

export function externalProviderObservedEntryFields(
  value: unknown,
): Partial<Pick<
  TerminalOutputEntry,
  | "source"
  | "externalProvider"
  | "externalProviderSessionId"
  | "externalProviderTurnId"
  | "observedAtMs"
  | "externalObservation"
>> {
  const metadata = readExternalProviderObservedMetadata(value)
  if (!metadata) {
    return {}
  }
  const fields: Partial<Pick<
    TerminalOutputEntry,
    | "source"
    | "externalProvider"
    | "externalProviderSessionId"
    | "externalProviderTurnId"
    | "observedAtMs"
    | "externalObservation"
  >> = {}
  fields.source = metadata.source
  fields.externalProvider = metadata.externalProvider
  fields.externalProviderSessionId = metadata.externalProviderSessionId
  fields.externalProviderTurnId = metadata.externalProviderTurnId
  fields.observedAtMs = metadata.observedAtMs
  fields.externalObservation = metadata.externalObservation
  return fields
}

export function externalProviderObservedIdentityFields(
  value: unknown,
): ExternalProviderObservedIdentityFields {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {}
  }
  const record = value as Record<string, unknown>
  const fields: ExternalProviderObservedIdentityFields = {
    ...(Object.prototype.hasOwnProperty.call(record, "external_provider")
      ? { externalProvider: normalizeExternalProviderId(readStringField(record, "external_provider")) }
      : {}),
    ...(Object.prototype.hasOwnProperty.call(record, "external_provider_session_id")
      ? { externalProviderSessionId: readNonBlankStringField(record, "external_provider_session_id") }
      : {}),
    ...(Object.prototype.hasOwnProperty.call(record, "external_provider_turn_id")
      ? { externalProviderTurnId: readNonBlankStringField(record, "external_provider_turn_id") }
      : {}),
  }
  if (
    fields.externalProvider !== undefined
    || fields.externalProviderSessionId !== undefined
    || fields.externalProviderTurnId !== undefined
  ) {
    return fields
  }
  return externalProviderObservedIdFields(
    readStringField(record, "prompt_id") ?? readStringField(record, "id"),
  ) ?? {}
}

export function externalProviderObservedExplicitIdentityFields(
  value: unknown,
): ExternalProviderObservedIdentityFields {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return {}
  }
  const record = value as Record<string, unknown>
  return {
    ...(Object.prototype.hasOwnProperty.call(record, "external_provider")
      ? { externalProvider: normalizeExternalProviderId(readStringField(record, "external_provider")) }
      : {}),
    ...(Object.prototype.hasOwnProperty.call(record, "external_provider_session_id")
      ? { externalProviderSessionId: readNonBlankStringField(record, "external_provider_session_id") }
      : {}),
    ...(Object.prototype.hasOwnProperty.call(record, "external_provider_turn_id")
      ? { externalProviderTurnId: readNonBlankStringField(record, "external_provider_turn_id") }
      : {}),
  }
}

export function externalProviderObservedPromptOriginMetadata(
  value: unknown,
): ExternalProviderObservedTurnMetadata | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return null
  }
  const record = value as Record<string, unknown>
  const promptOrigin = terminalPromptOriginFromRecord(record)
  const externalProvider = normalizeExternalProviderId(readStringField(record, "external_provider"))
  const externalProviderSessionId = readNonBlankStringField(record, "external_provider_session_id")
  const externalProviderTurnId = readNonBlankStringField(record, "external_provider_turn_id")
  if (!terminalPromptOriginIsExternal(promptOrigin)) {
    return null
  }
  return {
    source: terminalExternalProviderObservedSource,
    externalProvider: externalProvider ?? null,
    externalProviderSessionId: externalProviderSessionId ?? null,
    externalProviderTurnId: externalProviderTurnId ?? null,
  }
}

export function externalProviderObservedEntryTurnMetadata(
  entry: Pick<TerminalOutputEntry, "source" | "externalProvider" | "externalProviderSessionId" | "externalProviderTurnId">,
): ExternalProviderObservedTurnMetadata | null {
  if (!isExternalProviderObservedEntry(entry)) {
    return null
  }
  return {
    source: terminalExternalProviderObservedSource,
    externalProvider: normalizeExternalProviderId(entry.externalProvider),
    externalProviderSessionId: nonBlankString(entry.externalProviderSessionId),
    externalProviderTurnId: nonBlankString(entry.externalProviderTurnId),
  }
}

function readNonBlankStringField(value: unknown, field: string): string | null {
  const candidate = readStringField(value, field)
  const trimmed = candidate?.trim()
  return trimmed ? trimmed : null
}

function normalizeExternalProviderObservedSource(source: string | null | undefined): string | null {
  const trimmed = source?.trim()
  return trimmed ? trimmed.toLowerCase() : null
}

export function externalProviderObservedStatusSettlesActivePrompt(
  entry: {
    readonly kind?: string | null | undefined
    readonly role?: string | null | undefined
    readonly source?: string | null | undefined
    readonly externalObservation?: TerminalExternalObservation | null | undefined
    readonly external_observation?: TerminalExternalObservation | null | undefined
  },
): boolean {
  return (entry.kind === "provider_status" || entry.role === "status")
    && isExternalProviderObservedSource(entry.source)
    && readExternalProviderObservation(entry.externalObservation ?? entry.external_observation)?.settles_active_prompt === true
}

export function externalProviderObservedTurnMarker(
  entries: readonly Pick<TerminalOutputEntry, "source" | "externalProvider" | "externalProviderSessionId">[],
): ExternalProviderObservedTurnMarker | null {
  const metadata = entries
    .map((entry) => externalProviderObservedEntryTurnMetadata(entry))
    .find((candidate) => candidate !== null)
  if (metadata) {
    return {
      provider: metadata.externalProvider ?? "provider",
      providerSessionId: metadata.externalProviderSessionId ?? "unknown",
    }
  }
  return null
}

export function externalProviderObservedCompletionAtMs(
  entry: {
    readonly observedAtMs?: number | null | undefined
    readonly observed_at_ms?: number | null | undefined
    readonly createdAtMs?: number | null | undefined
    readonly created_at_ms?: number | null | undefined
  },
  nowMs: () => number,
): number {
  return finiteNumber(entry.observedAtMs)
    ?? finiteNumber(entry.observed_at_ms)
    ?? finiteNumber(entry.createdAtMs)
    ?? finiteNumber(entry.created_at_ms)
    ?? nowMs()
}

function finiteNumber(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null
}

function nonBlankString(value: string | null | undefined): string | null {
  const trimmed = value?.trim()
  return trimmed ? trimmed : null
}

function normalizeExternalProviderId(value: string | null | undefined): string | null {
  return nonBlankString(value)?.toLowerCase() ?? null
}

function externalProviderObservedIdFields(value: string | null | undefined): ExternalProviderObservedIdentityFields | null {
  const parts = value?.split(":") ?? []
  if (parts.length < 4 || parts[0] !== "external") {
    return null
  }
  const externalProvider = normalizeExternalProviderId(parts[1])
  const externalProviderSessionId = nonBlankString(parts[2])
  const externalProviderTurnId = nonBlankString(parts.slice(3).join(":"))
  if (!externalProvider || !externalProviderSessionId || !externalProviderTurnId) {
    return null
  }
  return {
    externalProvider,
    externalProviderSessionId,
    externalProviderTurnId,
  }
}

function fieldCanAcceptExternalProviderMetadata(value: unknown): boolean {
  return value === undefined || value === null
}
