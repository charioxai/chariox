import {
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  externalProviderObservedExactIdentityConflicts,
  externalProviderObservedExactIdentityKey,
  sessionHistoryEntryIsExternalProviderObserved,
} from "./external-provider-observation.js"
import type { TranscriptEntry as KernelTranscriptEntry } from "./kernel-types.js"

type TranscriptEntryRole = KernelTranscriptEntry["role"] | string

type TranscriptLineageKernelFields = Pick<
  KernelTranscriptEntry,
  | "text"
  | "turnId"
  | "source"
  | "externalProvider"
  | "externalProviderSessionId"
  | "externalProviderTurnId"
>

export type TranscriptLineageEntry = TranscriptLineageKernelFields & {
  readonly role: TranscriptEntryRole
  readonly promptId?: string | null
  readonly historyBlobId?: string | null | undefined
  readonly historyBlobAgentId?: string | null | undefined
  readonly historyBlobSourceId?: string | null | undefined
  readonly historyBlobSourceAgentId?: string | null | undefined
}

export type TranscriptRoleEntry = {
  readonly role: TranscriptEntryRole
}

export type TranscriptTurnDisplayEntry = TranscriptRoleEntry & {
  readonly id: KernelTranscriptEntry["id"]
  readonly text?: KernelTranscriptEntry["text"]
  readonly turnId?: KernelTranscriptEntry["turnId"] | null | undefined
  readonly historyBlobId?: string | null | undefined
}

export function transcriptEntryIsDisplayOnly(entry: TranscriptRoleEntry): boolean {
  return entry.role === "turn_toggle"
}

export function transcriptEntryIsRenderable(entry: TranscriptRoleEntry): boolean {
  return !transcriptEntryIsDisplayOnly(entry)
}

export function stripTranscriptDisplayOnlyEntries<TEntry extends TranscriptRoleEntry>(
  entries: readonly TEntry[],
): TEntry[] {
  return entries.filter(transcriptEntryIsRenderable)
}

export function transcriptEntryIsBlobCollapsible(entry: TranscriptTurnDisplayEntry): boolean {
  if (entry.historyBlobId) {
    return true
  }
  switch (entry.role) {
    case "tool":
    case "reasoning":
    case "status":
    case "notice":
      return true
    default:
      return false
  }
}

export function transcriptTurnFinalAssistantEntry<TEntry extends TranscriptTurnDisplayEntry>(
  turnEntries: readonly TEntry[],
): TEntry | null {
  return [...turnEntries].reverse().find((entry) => entry.role === "assistant") ?? null
}

export function transcriptTurnHasCollapsibleBody<TEntry extends TranscriptTurnDisplayEntry>(
  turnEntries: readonly TEntry[],
  finalSummaryId: number,
): boolean {
  return turnEntries.some((entry) => entry.role !== "user" && entry.id !== finalSummaryId)
}

export function transcriptTurnIsCollapsible<TEntry extends TranscriptTurnDisplayEntry>(
  turnEntries: readonly TEntry[],
  activeTurnId: number | null | undefined = null,
): boolean {
  const finalSummary = transcriptTurnFinalAssistantEntry(turnEntries)
  const turnId = turnEntries.find((entry) => typeof entry.turnId === "number")?.turnId
  return Boolean(finalSummary)
    && turnId !== activeTurnId
    && transcriptTurnHasCollapsibleBody(turnEntries, finalSummary!.id)
}

export function transcriptEntryLineageKeys(entry: TranscriptLineageEntry): string[] {
  const keys: string[] = []
  const externalIdentityKey = externalProviderObservedExactIdentityKey(entry)
  const source = transcriptLineageSource(entry)
  if (
    sessionHistoryEntryIsExternalProviderObserved(entry)
    && externalIdentityKey
  ) {
    keys.push([
      "external",
      externalIdentityKey.provider,
      externalIdentityKey.providerSessionId,
      externalIdentityKey.providerTurnId,
      entry.role,
    ].join(":"))
  }

  const blobAgentId = entry.historyBlobSourceAgentId ?? entry.historyBlobAgentId
  const blobId = entry.historyBlobSourceId ?? entry.historyBlobId
  if (blobAgentId || blobId) {
    keys.push([
      "blob",
      blobAgentId ?? "",
      blobId ?? "",
      entry.turnId ?? "",
      entry.role,
    ].join(":"))
  }

  const text = entry.text.trim()
  const turnIdentity = transcriptTurnLineageIdentity(entry)
  if (typeof entry.turnId === "number") {
    keys.push([
      "turn",
      source,
      turnIdentity,
      entry.role,
    ].join(":"))
  }
  if (text) {
    keys.push([
      "text",
      source,
      turnIdentity,
      entry.role,
      text,
    ].join(":"))
  }
  return keys
}

export function transcriptEntryDeduplicationKeys(entry: TranscriptLineageEntry): string[] {
  const keys: string[] = []
  const source = transcriptLineageSource(entry)
  const blobAgentId = entry.historyBlobSourceAgentId ?? entry.historyBlobAgentId
  const blobId = entry.historyBlobSourceId ?? entry.historyBlobId
  if (blobAgentId || blobId) {
    keys.push([
      "blob",
      blobAgentId ?? "",
      blobId ?? "",
      entry.turnId ?? "",
      entry.role,
    ].join(":"))
  }

  const text = entry.text.trim()
  const turnIdentity = transcriptTurnLineageIdentity(entry)
  if (text) {
    keys.push([
      "text",
      source,
      turnIdentity,
      entry.role,
      text,
    ].join(":"))
  }
  return keys
}

export function transcriptEntriesShareRenderableLineage<TEntry extends TranscriptLineageEntry>(
  currentEntries: readonly TEntry[],
  refreshedEntries: readonly TEntry[],
): boolean {
  const refreshedRenderableEntries = renderableEntries(refreshedEntries)
  if (renderableEntriesLineageKeys(refreshedRenderableEntries).length === 0) {
    return true
  }
  return renderableEntries(currentEntries)
    .some((currentEntry) => refreshedRenderableEntries
      .some((refreshedEntry) => transcriptEntriesShareLineageKeys(
        currentEntry,
        refreshedEntry,
        transcriptEntryLineageKeys,
      )))
}

export function transcriptEntriesContainRenderableLineage<TEntry extends TranscriptLineageEntry>(
  containingEntries: readonly TEntry[],
  candidateEntries: readonly TEntry[],
): boolean {
  const containingRenderableEntries = renderableEntries(containingEntries)
  return renderableEntries(candidateEntries)
    .every((candidateEntry) => containingRenderableEntries
      .some((containingEntry) => transcriptEntriesShareLineageKeys(
        containingEntry,
        candidateEntry,
        transcriptEntryLineageKeys,
      )))
}

export function prependTranscriptEntriesWithoutDuplicateRenderableLineage<TEntry extends TranscriptLineageEntry>(
  olderEntries: readonly TEntry[],
  currentEntries: readonly TEntry[],
): TEntry[] {
  const admittedRenderableEntries = renderableEntries(currentEntries)
  const prepend: TEntry[] = []
  for (const entry of olderEntries) {
    if (transcriptEntryIsRenderable(entry)) {
      if (admittedRenderableEntries.some((admittedEntry) => transcriptEntriesShareLineageKeys(
        admittedEntry,
        entry,
        transcriptEntryDeduplicationKeys,
      ))) {
        continue
      }
      admittedRenderableEntries.push(entry)
    }
    prepend.push(entry)
  }
  return [...prepend, ...currentEntries]
}

function renderableEntriesLineageKeys<TEntry extends TranscriptLineageEntry>(
  entries: readonly TEntry[],
): string[] {
  return entries.flatMap(transcriptEntryLineageKeys)
}

function renderableEntries<TEntry extends TranscriptLineageEntry>(
  entries: readonly TEntry[],
): TEntry[] {
  return stripTranscriptDisplayOnlyEntries(entries)
}

function transcriptEntriesShareLineageKeys<TEntry extends TranscriptLineageEntry>(
  left: TEntry,
  right: TEntry,
  keyFactory: (entry: TEntry) => string[],
): boolean {
  if (transcriptEntriesExternalExactIdentityConflict(left, right)) {
    return false
  }
  const rightKeys = new Set(keyFactory(right))
  return keyFactory(left).some((key) => rightKeys.has(key))
}

function transcriptEntriesExternalExactIdentityConflict(
  left: TranscriptLineageEntry,
  right: TranscriptLineageEntry,
): boolean {
  if (
    !sessionHistoryEntryIsExternalProviderObserved(left)
    || !sessionHistoryEntryIsExternalProviderObserved(right)
  ) {
    return false
  }
  return externalProviderObservedExactIdentityConflicts(left, right)
}

function transcriptLineageSource(entry: TranscriptLineageEntry): string {
  if (sessionHistoryEntryIsExternalProviderObserved(entry)) {
    return EXTERNAL_PROVIDER_OBSERVED_SOURCE
  }
  return entry.source?.trim() ?? ""
}

function transcriptTurnLineageIdentity(entry: TranscriptLineageEntry): string {
  const externalIdentityKey = sessionHistoryEntryIsExternalProviderObserved(entry)
    ? externalProviderObservedExactIdentityKey(entry)
    : null
  if (externalIdentityKey) {
    return [
      "external",
      externalIdentityKey.provider,
      externalIdentityKey.providerSessionId,
      externalIdentityKey.providerTurnId,
    ].join(":")
  }
  const promptId = entry.promptId?.trim()
  if (promptId) {
    return `prompt:${promptId}`
  }
  return entry.turnId === undefined || entry.turnId === null ? "" : String(entry.turnId)
}
