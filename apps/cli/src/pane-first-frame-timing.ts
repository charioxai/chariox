// MP-08 / MP-10: opt-in receipts from composed frames, never screen scraping.
import { appendFileSync } from "node:fs"
import type { CliRenderer } from "@opentui/core"
import type { TranscriptEntry } from "./cli-types.js"
export function recordPaneFirstFrame(
  renderer: CliRenderer,
  hasMountedTranscript: () => boolean,
  mountedEntries: () => Iterable<TranscriptEntry> = () => [],
): () => void {
  const destination = process.env.CHARIOX_PANE_TIMING_FILE
  if (!destination) return () => {}
  const prefix = process.env.CHARIOX_PANE_TIMING_PREFIX
  const tagPattern = prefix ? new RegExp(prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "[0-9]{2}(?![A-Za-z0-9-])") : null
  const seen = new Set<string>()
  let recorded = false
  const write = (value: Record<string, unknown>) => appendFileSync(destination,
    JSON.stringify({ ...value, epochMs: Date.now(), elapsedMs: process.uptime() * 1000, pid: process.pid }) + "\n", { mode: 0o600 })
  const sample = () => {
    if (!recorded && hasMountedTranscript()) {
      recorded = true
      write({ event: "pane_first_frame" })
      if (!prefix) renderer.removePostProcessFn(sample)
    }
    if (!tagPattern) return
    const entries = [...mountedEntries()]
    for (const prompt of entries) {
      if (prompt.role !== "user" || prompt.turnTracking === "none") continue
      const tag = prompt.text.match(tagPattern)?.[0]
      if (!tag || seen.has(tag)) continue
      const answer = entries.find(entry => entry.role === "assistant"
        && entry.text.trim().length > 0
        && !(entry.historyBlobId && !entry.historyBlobLoaded)
        && (prompt.promptId && entry.promptId
          ? prompt.promptId === entry.promptId
          : prompt.turnId !== undefined && prompt.turnId === entry.turnId))
      if (!answer) continue
      seen.add(tag)
      write({ event: "pane_assistant_frame", tag, entryId: answer.id,
        promptId: answer.promptId ?? prompt.promptId ?? null,
        turnId: answer.turnId ?? prompt.turnId ?? null })
    }
  }
  renderer.addPostProcessFn(sample)
  return () => renderer.removePostProcessFn(sample)
}
