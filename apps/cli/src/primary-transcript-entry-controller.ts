import type { TranscriptEntry } from "./cli-types.js"
import { computeTranscriptRebuildScrollTop } from "./background-effects.js"
import {
  projectTranscriptDisplayState,
  type TranscriptDisplayProjection,
} from "@arroba/kernel-client/transcript-display-state"
import { stitchPrependedHistoryTranscript } from "@arroba/kernel-client/session-history-transcript"

export type PrimaryTranscriptEntryScrollbox = {
  scrollTop: number
  scrollLeft: number
  scrollHeight: number
  height: number
  scrollTo: (position: { x: number; y: number }) => unknown
  requestRender: () => unknown
}

export type PrimaryTranscriptEntryControllerDeps = {
  getScrollbox: () => PrimaryTranscriptEntryScrollbox | undefined
  getEntries: () => TranscriptEntry[]
  getVisibleTranscriptAgentId: () => string | null
  expandedTurnIdsForAgent: (agentId: string | null | undefined) => readonly number[]
  clearToolState: () => void
  setEntries: (entries: TranscriptEntry[]) => void
  setEntryCounter: (counter: number) => void
  setCurrentTurnId: (turnId: number | null) => void
  setNextTurnId: (turnId: number) => void
  setMountedTranscriptAgentId: (agentId: string | null) => void
  setLastScrollTop: (scrollTop: number) => void
  rebuildTranscript: () => void
  syncVisibleTranscriptPreview: (
    agentId: string | null | undefined,
    entries: readonly TranscriptEntry[],
  ) => void
  restorePrependedHistory: (options: {
    scrollbox: PrimaryTranscriptEntryScrollbox
    previousScrollTop: number
    previousScrollHeight: number
    previousViewportHeight: number
  }) => Promise<void>
}

export function createPrimaryTranscriptEntryController(deps: PrimaryTranscriptEntryControllerDeps) {
  const applyProjection = (projection: TranscriptDisplayProjection<TranscriptEntry>) => {
    deps.setCurrentTurnId(projection.currentTurnId)
    deps.setNextTurnId(projection.nextTurnId)
    deps.setEntries(projection.entries)
    deps.setEntryCounter(projection.entryCounter)
  }

  const replaceEntries = (
    nextEntries: TranscriptEntry[],
    transcriptAgentId: string | null = deps.getVisibleTranscriptAgentId(),
  ) => {
    const scrollbox = deps.getScrollbox()
    const previousScrollTop = scrollbox?.scrollTop ?? 0
    const previousScrollHeight = scrollbox?.scrollHeight ?? 0
    const previousViewportHeight = scrollbox?.height ?? 0
    const projection = projectTranscriptDisplayState(
      nextEntries.filter(Boolean),
      deps.expandedTurnIdsForAgent(transcriptAgentId),
    )
    deps.clearToolState()
    applyProjection(projection)
    deps.rebuildTranscript()
    deps.setMountedTranscriptAgentId(transcriptAgentId)

    if (scrollbox && deps.getScrollbox() === scrollbox) {
      const nextScrollTop = computeTranscriptRebuildScrollTop({
        previousScrollTop,
        previousScrollHeight,
        nextScrollHeight: scrollbox.scrollHeight,
        viewportHeight: previousViewportHeight,
      })
      scrollbox.scrollTo({ x: scrollbox.scrollLeft, y: nextScrollTop })
      scrollbox.requestRender()
      deps.setLastScrollTop(scrollbox.scrollTop)
    } else {
      deps.setLastScrollTop(deps.getScrollbox()?.scrollTop ?? 0)
    }
    deps.syncVisibleTranscriptPreview(transcriptAgentId, projection.entries)
  }

  const prependEntries = async (nextEntries: TranscriptEntry[]) => {
    const sanitizedEntries = nextEntries.filter(Boolean)
    if (sanitizedEntries.length === 0) {
      return
    }

    const currentEntries = deps.getEntries().filter(Boolean)
    const scrollbox = deps.getScrollbox()
    const previousScrollHeight = scrollbox?.scrollHeight ?? 0
    const previousScrollTop = scrollbox?.scrollTop ?? 0
    const previousViewportHeight = scrollbox?.height ?? 0
    const projection = projectTranscriptDisplayState(
      stitchPrependedHistoryTranscript(sanitizedEntries, currentEntries) as TranscriptEntry[],
      deps.expandedTurnIdsForAgent(deps.getVisibleTranscriptAgentId()),
    )
    applyProjection(projection)
    deps.rebuildTranscript()

    const nextScrollbox = deps.getScrollbox()
    if (nextScrollbox) {
      await deps.restorePrependedHistory({
        scrollbox: nextScrollbox,
        previousScrollTop,
        previousScrollHeight,
        previousViewportHeight,
      })
    }
  }

  return {
    replaceEntries,
    prependEntries,
  }
}
