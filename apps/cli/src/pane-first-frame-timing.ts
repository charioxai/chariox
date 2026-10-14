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
    if (!prefix) return
    for (const entry of mountedEntries()) {
      if (entry.role !== "assistant") continue
      const at = entry.text.indexOf(prefix)
      if (at < 0) continue
      const tag = entry.text.slice(at).match(/^[A-Za-z0-9-]+/)?.[0]
      // A complete controlled two-digit trial id prevents partial-prefix receipts.
      if (!tag || !new RegExp("^" + prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "[0-9]{2}$").test(tag) || seen.has(tag)) continue
      seen.add(tag)
      write({ event: "pane_assistant_frame", tag, entryId: entry.id })
    }
  }
  renderer.addPostProcessFn(sample)
  return () => renderer.removePostProcessFn(sample)
}
