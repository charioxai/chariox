// MP-08 / MP-10: opt-in receipts from composed frames, never screen scraping.
import { appendFileSync } from "node:fs"
import type { CliRenderer } from "@opentui/core"
import type { TranscriptEntry } from "./cli-types.js"

// Private drill receipt: allowlisted metadata only, before client projection.
export function recordPaneKernelEvent(event: { event?: string; session?: Record<string, unknown> }): void {
  const destination = process.env.CHARIOX_PANE_TIMING_FILE
  const prefix = process.env.CHARIOX_PANE_TIMING_PREFIX
  if (!destination || !prefix || event.event !== "session_snapshot" || !event.session) return
  const states = event.session.prompt_states as Record<string, { active_prompt?: unknown }> | undefined
  const prompts = [event.session.active_prompt, ...Object.values(states ?? {}).map(state => state.active_prompt)]
  const seen = new Set<string>()
  for (const value of prompts) {
    if (!value || typeof value !== "object") continue
    const prompt = value as { id?: unknown; prompt?: unknown; created_at_ms?: unknown }
    if (typeof prompt.id !== "string" || typeof prompt.prompt !== "string" || seen.has(prompt.id)) continue
    const tag = prompt.prompt.match(new RegExp(prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "[0-9]{2}(?![A-Za-z0-9-])"))?.[0]
    if (!tag) continue
    seen.add(prompt.id)
    appendFileSync(destination, JSON.stringify({ event: "pane_kernel_prompt_received", tag,
      promptId: prompt.id, kernelCreatedAtMs: typeof prompt.created_at_ms === "number" ? prompt.created_at_ms : null,
      epochMs: Date.now(), pid: process.pid }) + "\n", { mode: 0o600 })
  }
}
export function recordPaneFirstFrame(
  renderer: CliRenderer,
  hasMountedTranscript: () => boolean,
  mountedEntries: () => Iterable<TranscriptEntry> = () => [],
  cached: boolean | "preview" = false,
  onFirstFrame?: () => void,
  hasMountedSurface: () => boolean = hasMountedTranscript,
  requestStats?: () => { count: number; kinds: Record<string, number>; timeline?: Array<[string, number]> },
): () => void {
  const destination = process.env.CHARIOX_PANE_TIMING_FILE
  if (!destination && !onFirstFrame) return () => {}
  const prefix = destination ? process.env.CHARIOX_PANE_TIMING_PREFIX : undefined
  const tagPattern = prefix ? new RegExp(prefix.replace(/[.*+?^${}()|[\]\\]/g, "\\$&") + "[0-9]{2}(?![A-Za-z0-9-])") : null
  const seen = new Set<string>()
  const seenUsers = new Set<string>()
  let recorded = false
  let handedOff = false
  const write = (value: Record<string, unknown>) => {
    if (destination) appendFileSync(destination,
      JSON.stringify({ ...value, epochMs: Date.now(), elapsedMs: process.uptime() * 1000, pid: process.pid }) + "\n", { mode: 0o600 })
  }
  const sample = () => {
    if (!handedOff && hasMountedSurface()) {
      handedOff = true
      onFirstFrame?.()
      if (!destination) renderer.removePostProcessFn(sample)
    }
    if (!recorded && hasMountedTranscript()) {
      recorded = true
      write({ event: "pane_first_frame", ...(cached === "preview" ? { preview: true } : cached ? { cached: true } : {}), ...(requestStats ? { kernelRequests: requestStats() } : {}) })
      // Requests the open still issues after the first frame (provider launch, subscriptions).
      if (requestStats) setTimeout(() => write({ event: "pane_open_requests", kernelRequests: requestStats() }), 3000).unref()
      if (destination && !prefix) renderer.removePostProcessFn(sample)
    }
    if (!tagPattern) return
    const entries = [...mountedEntries()]
    for (const prompt of entries) {
      if (prompt.role !== "user" || prompt.turnTracking === "none") continue
      const tag = prompt.text.match(tagPattern)?.[0]
      if (!tag) continue
      if (!seenUsers.has(tag)) {
        seenUsers.add(tag)
        write({ event: "pane_user_frame", tag, entryId: prompt.id,
          promptId: prompt.promptId ?? null, turnId: prompt.turnId ?? null })
      }
      if (seen.has(tag)) continue
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
