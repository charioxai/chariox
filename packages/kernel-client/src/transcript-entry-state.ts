import type { TranscriptEntry as KernelTranscriptEntry } from "./kernel-types.js"
import {
  externalProviderObservedExactIdentityConflicts,
  externalProviderObservedExactIdentityMatches,
  type ExternalProviderObservedIdentityFields,
} from "./external-provider-observation.js"

type TranscriptEntryStateKernelFields = Pick<KernelTranscriptEntry, "id" | "text"> & {
  readonly turnId?: KernelTranscriptEntry["turnId"] | null
}

export type TranscriptEntryStateEntry = {
  readonly id: TranscriptEntryStateKernelFields["id"]
  readonly role: string
  readonly text: TranscriptEntryStateKernelFields["text"]
  readonly turnId?: TranscriptEntryStateKernelFields["turnId"]
  readonly promptId?: string | null
}

export type TranscriptTurnAssignmentId = string | number

export type TranscriptTurnAssignmentEntry<TTurnId extends TranscriptTurnAssignmentId = number> = {
  readonly role: string
  readonly text?: string
  turnId?: TTurnId | null
  readonly turnTracking?: "none"
  promptId?: string | null
  promptOrigin?: string | null
  sourceAttachmentId?: string | null
  providerRunId?: string | null
  externalProvider?: string | null
  externalProviderSessionId?: string | null
  externalProviderTurnId?: string | null
  readonly outputIdentity?: string | null
  readonly createdAtMs?: number | null
}

export type TranscriptTurnAssignmentOptions<
  TTurnId extends TranscriptTurnAssignmentId = number,
  TEntry extends TranscriptTurnAssignmentEntry<TTurnId> = TranscriptTurnAssignmentEntry<TTurnId>,
> = {
  readonly turnId: TTurnId
  readonly promptId?: string | null
  readonly promptOrigin?: string | null
  readonly sourceAttachmentId?: string | null
  readonly providerRunId?: string | null
  readonly externalProvider?: string | null
  readonly externalProviderSessionId?: string | null
  readonly externalProviderTurnId?: string | null
  readonly nowMs?: () => number
  readonly onAssigned?: (turnId: TTurnId, entry: TEntry, assignedAtMs: number | null) => void
}

export type TranscriptEquivalentOutput<TEntry extends TranscriptTurnAssignmentEntry<TTurnId>, TTurnId extends TranscriptTurnAssignmentId = number> = {
  readonly entry: TEntry
  readonly previousTurnId?: TTurnId | null
}

export type TranscriptTurnSiblingRetargetOptions<
  TTurnId extends TranscriptTurnAssignmentId = number,
  TEntry extends TranscriptTurnAssignmentEntry<TTurnId> = TranscriptTurnAssignmentEntry<TTurnId>,
> = {
  readonly nowMs?: () => number
  readonly onRetargeted?: (turnId: TTurnId, entry: TEntry, retargetedAtMs: number | null) => void
}

export type TranscriptPromptMetadata = {
  readonly promptId?: string | null | undefined
  readonly sourceAttachmentId?: string | null | undefined
  readonly promptOrigin?: string | null | undefined
}

export type TranscriptPromptMetadataTarget = {
  promptId?: string | null | undefined
  promptOrigin?: string | null | undefined
  sourceAttachmentId?: string | null | undefined
}

export function applyTranscriptPromptMetadata<TEntry extends object>(
  entry: TEntry,
  metadata: TranscriptPromptMetadata,
  options: { readonly preserveExisting?: boolean } = {},
): TEntry {
  const target = entry as TEntry & TranscriptPromptMetadataTarget
  if (metadata.promptId !== undefined && (!options.preserveExisting || target.promptId === undefined)) {
    target.promptId = metadata.promptId
  }
  if (metadata.promptOrigin !== undefined && (!options.preserveExisting || target.promptOrigin === undefined)) {
    target.promptOrigin = metadata.promptOrigin
  }
  if (
    metadata.sourceAttachmentId !== undefined
    && (!options.preserveExisting || target.sourceAttachmentId === undefined)
  ) {
    target.sourceAttachmentId = metadata.sourceAttachmentId
  }
  return entry
}

export type TranscriptUserPromptTurn = {
  readonly entry: {
    readonly role: "user"
    readonly text: string
    readonly turnId: number
    readonly promptId?: string | null
    readonly sourceAttachmentId?: string | null
    readonly promptOrigin?: string | null
  }
  readonly currentTurnId: number
  readonly nextTurnId: number
}

export type TranscriptSteeredPromptEntry = {
  readonly role: "user"
  readonly text: string
  readonly turnTracking: "none"
  readonly promptId?: string | null
  readonly sourceAttachmentId?: string | null
  readonly promptOrigin?: string | null
}

export type TranscriptEntryRuntimeState = {
  readonly entryCounter: number
  readonly currentTurnId: number | null
}

export type TranscriptEntryRuntimeOptions = {
  readonly nextEntryId: number
  readonly currentTurnId: number | null
}

export type TranscriptRetentionSlice<TEntry extends Pick<TranscriptEntryStateEntry, "text">> = {
  readonly removed: TEntry[]
  readonly kept: TEntry[]
  readonly changed: boolean
}

export function trimSingleTrailingNewline(text: string): string {
  return text.endsWith("\n") ? text.slice(0, -1) : text
}

export function reindexTranscriptEntries<TEntry extends { readonly id: number }>(
  entries: readonly TEntry[],
  startingId: number,
): TEntry[] {
  return entries.map((entry, index) => ({
    ...entry,
    id: startingId + index + 1,
  }))
}

export function computeCurrentTranscriptTurnId<TEntry extends Pick<TranscriptEntryStateEntry, "role" | "turnId">>(
  entries: readonly TEntry[],
): number | null {
  return entries.reduce<number | null>((latest, entry) => {
    if (!entry || entry.role !== "user" || entry.turnId === undefined || entry.turnId === null) {
      return latest
    }
    return entry.turnId
  }, null)
}

export function computeNextTranscriptTurnId<TEntry extends Pick<TranscriptEntryStateEntry, "turnId">>(
  entries: readonly TEntry[],
): number {
  return entries.reduce((max, entry) => Math.max(max, entry?.turnId ?? 0), 0) + 1
}

export function computeNextTranscriptEntryId<TEntry extends Pick<TranscriptEntryStateEntry, "id">>(
  entries: readonly TEntry[],
): number {
  return computeMaxTranscriptEntryId(entries) + 1
}

export function computeMaxTranscriptEntryId<TEntry extends Pick<TranscriptEntryStateEntry, "id">>(
  entries: readonly TEntry[],
): number {
  return entries.reduce((max, entry) => Math.max(max, entry.id), 0)
}

export function transcriptRetentionSlice<TEntry extends Pick<TranscriptEntryStateEntry, "text">>(
  entries: readonly TEntry[],
  options: {
    readonly maxEntries: number
    readonly maxChars: number
  },
): TranscriptRetentionSlice<TEntry> {
  const currentEntries = entries.map((entry) => ({ ...entry })) as TEntry[]
  const maxEntries = finiteIntegerOrZero(options.maxEntries)
  const maxChars = finiteIntegerOrZero(options.maxChars)
  let totalChars = currentEntries.reduce((sum, entry) => sum + entry.text.length, 0)
  let removeCount = 0

  while (
    currentEntries.length - removeCount > maxEntries
    || (totalChars > maxChars && removeCount < currentEntries.length - 1)
  ) {
    totalChars -= currentEntries[removeCount]?.text.length ?? 0
    removeCount += 1
  }

  if (removeCount === 0) {
    return {
      removed: [],
      kept: currentEntries,
      changed: false,
    }
  }

  return {
    removed: currentEntries.slice(0, removeCount),
    kept: currentEntries.slice(removeCount),
    changed: true,
  }
}

export function transcriptHasTrailingUserPrompt<TEntry extends Pick<TranscriptEntryStateEntry, "role" | "text" | "promptId">>(
  entries: readonly TEntry[],
  text: string,
  promptId?: string | null,
): boolean {
  const lastEntry = entries.at(-1)
  if (lastEntry?.role !== "user") {
    return false
  }
  if (hasTranscriptPromptIdentity(lastEntry.promptId) && hasTranscriptPromptIdentity(promptId)) {
    return lastEntry.promptId === promptId
  }
  return trimSingleTrailingNewline(lastEntry.text) === trimSingleTrailingNewline(text)
}

export function createTranscriptUserPromptTurn(
  text: string,
  turnId: number,
  metadata: TranscriptPromptMetadata = {},
): TranscriptUserPromptTurn {
  const entry = {
    role: "user" as const,
    text: trimSingleTrailingNewline(text),
    turnId,
  }
  applyTranscriptPromptMetadata(entry, metadata)
  return {
    entry,
    currentTurnId: turnId,
    nextTurnId: turnId + 1,
  }
}

export function createTranscriptSteeredPromptEntry(
  text: string,
  metadata: TranscriptPromptMetadata = {},
): TranscriptSteeredPromptEntry | null {
  const normalized = trimSingleTrailingNewline(text)
  if (!normalized) {
    return null
  }
  const entry = {
    role: "user",
    text: normalized,
    turnTracking: "none",
  } satisfies TranscriptSteeredPromptEntry
  return applyTranscriptPromptMetadata(entry, metadata)
}

export function shouldSkipConsecutiveTranscriptEntry(
  previous: { readonly role: string; readonly text: string; readonly emphasis?: string | undefined } | null | undefined,
  next: { readonly role: string; readonly text: string; readonly emphasis?: string | undefined },
) {
  if (!previous) {
    return false
  }
  if (next.role !== "error" && next.role !== "notice") {
    return false
  }
  return previous.role === next.role
    && previous.text === next.text
    && previous.emphasis === next.emphasis
}

export function transcriptEntryRuntimeOptions(
  state: TranscriptEntryRuntimeState,
): TranscriptEntryRuntimeOptions {
  return {
    nextEntryId: state.entryCounter + 1,
    currentTurnId: state.currentTurnId,
  }
}

export function createNextTranscriptEntry<
  TEntry extends TranscriptEntryStateEntry,
  TDraft extends Omit<TEntry, "id">,
>(
  currentEntries: readonly TEntry[],
  entry: TDraft,
  options: {
    readonly nextEntryId?: number
    readonly currentTurnId?: number | null
  } = {},
): TEntry {
  const nextEntry = {
    id: options.nextEntryId ?? computeNextTranscriptEntryId(currentEntries),
    ...entry,
  } as unknown as TEntry
  if (nextEntry.turnId === undefined) {
    const activeTurnId = options.currentTurnId !== undefined
      ? options.currentTurnId
      : computeCurrentTranscriptTurnId(currentEntries)
    if (activeTurnId !== null) {
      return {
        ...nextEntry,
        turnId: activeTurnId,
      }
    }
  }
  return nextEntry
}

export function assignMatchingUntrackedTranscriptEntriesToTurn<
  TTurnId extends TranscriptTurnAssignmentId,
  TEntry extends TranscriptTurnAssignmentEntry<TTurnId>,
>(
  entries: readonly TEntry[],
  promptEntry: TEntry,
  options: TranscriptTurnAssignmentOptions<TTurnId, TEntry>,
): number {
  const promptId = promptEntry.promptId ?? options.promptId
  const promptOrigin = promptEntry.promptOrigin ?? options.promptOrigin
  const sourceAttachmentId = promptEntry.sourceAttachmentId ?? options.sourceAttachmentId
  const providerRunId = promptEntry.providerRunId ?? options.providerRunId
  const externalIdentity = transcriptTurnAssignmentExternalIdentity(promptEntry, options)
  const hasPromptId = hasTranscriptPromptIdentity(promptId)
  const hasProviderRunId = Boolean(providerRunId)
  const hasExternalIdentity = transcriptTurnAssignmentHasExactExternalIdentity(externalIdentity)
  if (!hasPromptId && !hasProviderRunId && !hasExternalIdentity) {
    return 0
  }
  let assigned = 0
  for (const entry of entries) {
    if (
      entry === promptEntry
      || entry.role === "user"
      || entry.turnId !== undefined && entry.turnId !== null
      || entry.turnTracking === "none"
    ) {
      continue
    }
    const matchesPrompt = hasPromptId && entry.promptId === promptId
    const matchesProviderRun = hasProviderRunId && (
      entry.providerRunId === providerRunId
      || entry.outputIdentity?.startsWith(`${providerRunId}:`) === true
    )
    const matchesExternalIdentity = hasExternalIdentity
      && externalProviderObservedExactIdentityMatches(entry, externalIdentity)
    if (externalProviderObservedExactIdentityConflicts(entry, externalIdentity)) {
      continue
    }
    if (!matchesPrompt && !matchesProviderRun && !matchesExternalIdentity) {
      continue
    }
    entry.turnId = options.turnId
    applyTranscriptTurnAssignmentMetadata(entry, {
      promptId,
      promptOrigin,
      sourceAttachmentId,
      providerRunId,
      externalProvider: externalIdentity.externalProvider,
      externalProviderSessionId: externalIdentity.externalProviderSessionId,
      externalProviderTurnId: externalIdentity.externalProviderTurnId,
    })
    options.onAssigned?.(options.turnId, entry, transcriptAssignmentTimestamp(entry, options.nowMs))
    assigned += 1
  }
  return assigned
}

function hasTranscriptPromptIdentity(value: string | null | undefined): value is string {
  return value !== null && value !== undefined
}

export function retargetEquivalentTranscriptTurnSiblings<
  TTurnId extends TranscriptTurnAssignmentId,
  TEntry extends TranscriptTurnAssignmentEntry<TTurnId>,
>(
  entries: readonly TEntry[],
  equivalentOutput: TranscriptEquivalentOutput<TEntry, TTurnId>,
  canonicalEntry: TEntry,
  options: TranscriptTurnSiblingRetargetOptions<TTurnId, TEntry> = {},
): number {
  if (
    canonicalEntry.turnId === undefined
    || canonicalEntry.turnId === null
    || equivalentOutput.previousTurnId === undefined
    || equivalentOutput.previousTurnId === null
    || canonicalEntry.turnId === equivalentOutput.previousTurnId
  ) {
    return 0
  }
  let retargeted = 0
  for (const sibling of entries) {
    if (
      sibling === equivalentOutput.entry
      || sibling.role === "user"
      || sibling.turnId !== equivalentOutput.previousTurnId
      || sibling.turnTracking === "none"
    ) {
      continue
    }
    sibling.turnId = canonicalEntry.turnId
    applyTranscriptTurnAssignmentMetadata(sibling, canonicalEntry)
    options.onRetargeted?.(canonicalEntry.turnId, sibling, transcriptAssignmentTimestamp(sibling, options.nowMs))
    retargeted += 1
  }
  return retargeted
}

function applyTranscriptTurnAssignmentMetadata<TTurnId extends TranscriptTurnAssignmentId>(
  entry: TranscriptTurnAssignmentEntry<TTurnId>,
  metadata: {
    readonly promptId?: string | null | undefined
    readonly promptOrigin?: string | null | undefined
    readonly sourceAttachmentId?: string | null | undefined
    readonly providerRunId?: string | null | undefined
    readonly externalProvider?: string | null | undefined
    readonly externalProviderSessionId?: string | null | undefined
    readonly externalProviderTurnId?: string | null | undefined
  },
): void {
  if (entry.promptId === undefined && metadata.promptId !== undefined) {
    entry.promptId = metadata.promptId
  }
  if (entry.promptOrigin === undefined && metadata.promptOrigin !== undefined) {
    entry.promptOrigin = metadata.promptOrigin
  }
  if (entry.sourceAttachmentId === undefined && metadata.sourceAttachmentId !== undefined) {
    entry.sourceAttachmentId = metadata.sourceAttachmentId
  }
  if (entry.providerRunId === undefined && metadata.providerRunId !== undefined) {
    entry.providerRunId = metadata.providerRunId
  }
  if (entry.externalProvider === undefined && metadata.externalProvider !== undefined) {
    entry.externalProvider = metadata.externalProvider
  }
  if (entry.externalProviderSessionId === undefined && metadata.externalProviderSessionId !== undefined) {
    entry.externalProviderSessionId = metadata.externalProviderSessionId
  }
  if (entry.externalProviderTurnId === undefined && metadata.externalProviderTurnId !== undefined) {
    entry.externalProviderTurnId = metadata.externalProviderTurnId
  }
}

function transcriptTurnAssignmentExternalIdentity<
  TTurnId extends TranscriptTurnAssignmentId,
  TEntry extends TranscriptTurnAssignmentEntry<TTurnId>,
>(
  promptEntry: TEntry,
  options: TranscriptTurnAssignmentOptions<TTurnId, TEntry>,
): ExternalProviderObservedIdentityFields {
  const externalProvider = promptEntry.externalProvider ?? options.externalProvider
  const externalProviderSessionId = promptEntry.externalProviderSessionId ?? options.externalProviderSessionId
  const externalProviderTurnId = promptEntry.externalProviderTurnId ?? options.externalProviderTurnId
  return {
    ...(externalProvider !== undefined ? { externalProvider } : {}),
    ...(externalProviderSessionId !== undefined ? { externalProviderSessionId } : {}),
    ...(externalProviderTurnId !== undefined ? { externalProviderTurnId } : {}),
  }
}

function transcriptTurnAssignmentHasExactExternalIdentity(
  identity: ExternalProviderObservedIdentityFields,
): boolean {
  return externalProviderObservedExactIdentityMatches(identity, identity)
}

function transcriptAssignmentTimestamp<TTurnId extends TranscriptTurnAssignmentId>(
  entry: TranscriptTurnAssignmentEntry<TTurnId>,
  nowMs: (() => number) | undefined,
): number | null {
  return entry.createdAtMs ?? nowMs?.() ?? null
}

function finiteIntegerOrZero(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.trunc(value)) : 0
}
