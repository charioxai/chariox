// MP-08 / MP-10: opt-in drill receipt at the first composed transcript frame.
import { appendFileSync } from "node:fs"
import type { CliRenderer } from "@opentui/core"
export function recordPaneFirstFrame(renderer: CliRenderer, hasMountedTranscript: () => boolean): () => void {
  const destination = process.env.CHARIOX_PANE_TIMING_FILE
  if (!destination) return () => {}
  let recorded = false
  const sample = () => {
    if (recorded || !hasMountedTranscript()) return
    recorded = true
    appendFileSync(destination, JSON.stringify({ event: "pane_first_frame", elapsedMs: process.uptime() * 1000, pid: process.pid }) + "\n", { mode: 0o600 })
    renderer.removePostProcessFn(sample)
  }
  renderer.addPostProcessFn(sample)
  return () => renderer.removePostProcessFn(sample)
}
