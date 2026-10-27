import type { TranscriptEntry } from "./cli-types.js"

type MaybeTranscriptEntry = TranscriptEntry | null | undefined | false

type TranscriptEntryProjectionControllerDeps = {
  getEntries: () => readonly MaybeTranscriptEntry[]
}

export function createTranscriptEntryProjectionController(
  deps: TranscriptEntryProjectionControllerDeps,
) {
  const renderableEntries = (): TranscriptEntry[] =>
    deps.getEntries().filter((entry): entry is TranscriptEntry => Boolean(entry))

  const visibleEntries = (): TranscriptEntry[] =>
    renderableEntries().filter((entry) => !entry.hidden)

  return {
    renderableEntries,
    visibleEntries,
    visibleEntryCount: () => visibleEntries().length,
  }
}
