// MP-08 / MP-10 / MP-11: persist only the drawn transcript, outside the input/approval surfaces.
import type { CliRenderer, OptimizedBuffer, ScrollBoxRenderable } from "@opentui/core"
import { createEffect, onCleanup } from "solid-js"
import { captureTranscriptFrame, type CachedTranscriptFrame } from "./client-view-frame.js"
import type { LocalIpcClient } from "./ipc.js"
import type { RuntimeSession, TranscriptEntry } from "./cli-types.js"
import { cliViewScope, cliViewClient, clientViewEpoch, writeClientView } from "./client-view-cache.js"

export function createCliViewPersistence(deps: {
  client: LocalIpcClient
  renderer: CliRenderer
  viewport(): ScrollBoxRenderable | undefined
  covered(): boolean
  session(): RuntimeSession
  agentId(): string | null
  entries(): TranscriptEntry[]
  attached(): boolean
}): void {
  let timer: ReturnType<typeof setTimeout> | undefined
  let save: ((frame: CachedTranscriptFrame) => void) | undefined
  let previous = "", revision = 0, latestBuffer: OptimizedBuffer | undefined
  const sample = (buffer: OptimizedBuffer) => {
    latestBuffer = buffer
    if (!save || timer || deps.covered()) return
    const requestedRevision = revision
    timer = setTimeout(() => {
      timer = undefined
      const viewport = deps.viewport()
      if (requestedRevision !== revision || !save || !viewport || !latestBuffer || deps.covered()) return
      // Read the most recently composed buffer between frames. In particular,
      // asynchronous Markdown formatting can finish after the first mounted row.
      let frame: CachedTranscriptFrame | undefined
      try { frame = captureTranscriptFrame(latestBuffer, viewport) } catch { return }
      if (!frame) return
      const identity = JSON.stringify(frame)
      if (identity === previous) return
      previous = identity
      save(frame)
    }, 150)
  }
  deps.renderer.addPostProcessFn(sample)
  createEffect(() => {
    const capturedRevision = ++revision
    const session = deps.session(), agentId = deps.agentId(), entries = deps.entries(), attached = deps.attached()
    if (timer) clearTimeout(timer)
    timer = undefined; save = undefined; previous = ""
    if (!attached || !session.id || !agentId || !entries.length) return
    const generation = clientViewEpoch(), viewClient = cliViewClient(deps.client)
    const presentation = entries.filter(entry => ["user", "assistant", "tool", "reasoning", "turn_toggle"].includes(entry.role)).map(entry => ({ ...entry }))
    const sequence = Math.max(0, ...presentation.flatMap(entry => [entry.historyEntryIndex ?? 0, ...(entry.historySequenceRanges ?? []).map(range => range[1])]))
    save = frame => { void (async () => {
      const scope = await cliViewScope(viewClient)
      if (capturedRevision !== revision || !scope || cliViewClient(deps.client) !== viewClient
        || generation !== clientViewEpoch() || deps.session().id !== session.id || deps.agentId() !== agentId) return
      await writeClientView({ scope, sessionId: session.id, agentId, alias: session.alias ?? session.id,
        sequence, savedAt: Date.now(), entries: presentation, frame })
    })() }
    deps.renderer.requestRender()
  })
  onCleanup(() => {
    revision++; save = undefined; latestBuffer = undefined
    if (timer) clearTimeout(timer)
    deps.renderer.removePostProcessFn(sample)
  })
}
