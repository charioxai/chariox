import { sessionHistoryEntryIsExternalProviderObserved } from "./external-provider-observation.js"

export type TranscriptLineageEntry = {
  readonly role: string
  readonly text: string
  readonly turnId?: number | null | undefined
  readonly source?: string | null | undefined
  readonly externalProvider?: string | null | undefined
  readonly externalProviderSessionId?: string | null | undefined
  readonly externalProviderTurnId?: string | null | undefined
  readonly historyBlobId?: string | null | undefined
  readonly historyBlobAgentId?: string | null | undefined
  readonly historyBlobSourceId?: string | null | undefined
  readonly historyBlobSourceAgentId?: string | null | undefined
}

export type TranscriptRoleEntry = {
  readonly role: string
}

export type TranscriptTurnDisplayEntry = TranscriptRoleEntry & {
  readonly id: number
  readonly text?: string
  readonly turnId?: number | null | undefined
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
  const externalProvider = entry.externalProvider ?? ""
  const externalProviderSessionId = entry.externalProviderSessionId ?? ""
  const externalProviderTurnId = entry.externalProviderTurnId ?? ""
  if (
    sessionHistoryEntryIsExternalProviderObserved(entry)
    && (externalProvider || externalProviderSessionId || externalProviderTurnId)
  ) {
    keys.push([
      "external",
      externalProvider,
      externalProviderSessionId,
      externalProviderTurnId,
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
  if (typeof entry.turnId === "number") {
    keys.push([
      "turn",
      entry.source ?? "",
      entry.turnId,
      entry.role,
    ].join(":"))
  }
  if (text) {
    keys.push([
      "text",
      entry.source ?? "",
      entry.turnId ?? "",
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
  const refreshedKeys = new Set(renderableLineageKeys(refreshedEntries))
  if (refreshedKeys.size === 0) {
    return true
  }
  return renderableEntries(currentEntries)
    .some((entry) => transcriptEntryLineageKeys(entry).some((key) => refreshedKeys.has(key)))
}

export function transcriptEntriesContainRenderableLineage<TEntry extends TranscriptLineageEntry>(
  containingEntries: readonly TEntry[],
  candidateEntries: readonly TEntry[],
): boolean {
  const containingKeys = new Set(renderableLineageKeys(containingEntries))
  return renderableEntries(candidateEntries)
    .every((entry) => transcriptEntryLineageKeys(entry).some((key) => containingKeys.has(key)))
}

export function prependTranscriptEntriesWithoutDuplicateRenderableLineage<TEntry extends TranscriptLineageEntry>(
  olderEntries: readonly TEntry[],
  currentEntries: readonly TEntry[],
): TEntry[] {
  const admittedKeys = new Set(renderableLineageKeys(currentEntries))
  const prepend: TEntry[] = []
  for (const entry of olderEntries) {
    if (transcriptEntryIsRenderable(entry)) {
      const keys = transcriptEntryLineageKeys(entry)
      if (keys.some((key) => admittedKeys.has(key))) {
        continue
      }
      for (const key of keys) {
        admittedKeys.add(key)
      }
    }
    prepend.push(entry)
  }
  return [...prepend, ...currentEntries]
}

function renderableLineageKeys<TEntry extends TranscriptLineageEntry>(
  entries: readonly TEntry[],
): string[] {
  return renderableEntries(entries).flatMap(transcriptEntryLineageKeys)
}

function renderableEntries<TEntry extends TranscriptLineageEntry>(
  entries: readonly TEntry[],
): TEntry[] {
  return stripTranscriptDisplayOnlyEntries(entries)
}
