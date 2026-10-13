import type { SessionHistoryBlobContent, TranscriptEntry } from "./cli-types.js"
import { cloneCompactTranscriptDisplayEntries, projectCompactTranscriptDisplayState } from "@chariox/kernel-client/transcript-display-state"
import { replaceSessionHistoryBlobPlaceholders, resolveSessionHistoryBlobLoadTarget } from "@chariox/kernel-client/session-history-transcript"

// MP-08 / MP-10: both primary and auxiliary Ctrl+O use one projection per
// action, with bounded history batches below the controller.
export function toggleTranscriptDetails(deps: {
  entries: () => TranscriptEntry[]
  collapsedTurns: () => readonly number[]
  commit: (current: TranscriptEntry[], next: TranscriptEntry[]) => void
  load?: ((agentId: string, blobIds: readonly string[]) => Promise<SessionHistoryBlobContent[]>) | undefined
  formatError?: ((error: unknown) => string) | undefined
}): void {
  const current = cloneCompactTranscriptDisplayEntries(deps.entries())
  const blobs = current.filter((entry) => entry.blobCollapsible && entry.role !== "turn_toggle" && !entry.hidden)
  if (!blobs.length) return
  const expand = blobs.some((entry) => entry.blobCollapsed !== false)
  const targets = new Map<string, Set<string>>()
  const selected = new Set(blobs.map((entry) => entry.id))
  const next = current.map((entry) => {
    if (!selected.has(entry.id)) return entry
    const target = resolveSessionHistoryBlobLoadTarget(entry, !expand)
    if (target && deps.load) {
      const ids = targets.get(target.agentId) ?? new Set<string>()
      ids.add(target.blobId)
      targets.set(target.agentId, ids)
      return { ...entry, historyBlobLoading: true, blobSummary: "loading..." }
    }
    return { ...entry, blobCollapsed: !expand }
  })
  deps.commit(current, projectCompactTranscriptDisplayState(next, deps.collapsedTurns()).entries)
  if (!targets.size || !deps.load) return
  void (async () => {
    const contents: { agentId: string; content: SessionHistoryBlobContent }[] = []
    try {
      for (const [agentId, ids] of targets) contents.push(...(await deps.load!(agentId, [...ids])).map(content => ({ agentId, content })))
      const latest = cloneCompactTranscriptDisplayEntries(deps.entries())
      deps.commit(latest, replaceSessionHistoryBlobPlaceholders(latest, contents, deps.collapsedTurns()) as TranscriptEntry[])
    } catch (error) {
      const latest = cloneCompactTranscriptDisplayEntries(deps.entries())
      deps.commit(latest, latest.map((entry) => targets.get(entry.historyBlobAgentId ?? "")?.has(entry.historyBlobId ?? "")
        ? { ...entry, historyBlobLoading: false, historyBlobError: deps.formatError?.(error) ?? String(error), blobSummary: "failed to load" }
        : entry))
    }
  })()
}

export function toggleFocusedTranscriptDetails(deps: {
  focusedAgentId: () => string | null | undefined
  visibleAgentId: () => string | null | undefined
  primary: () => void
  auxiliary: (agentId: string) => void
}): void {
  const focused = deps.focusedAgentId()
  if (focused && focused !== deps.visibleAgentId()) deps.auxiliary(focused)
  else deps.primary()
}
