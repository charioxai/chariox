import type { TranscriptEntry } from "./cli-types.js"
import { transcriptRetentionSlice } from "@chariox/kernel-client/transcript-entry-state"

export type TranscriptRenderableHandle = {
  wrapper: {
    id: string
    destroyRecursively: () => void
  }
}

export type TranscriptRetentionControllerDeps<TRenderable extends TranscriptRenderableHandle> = {
  entries: () => TranscriptEntry[]
  setEntries: (entries: TranscriptEntry[]) => void
  renderables: Map<number, TRenderable>
  removeFromScrollbox: (renderableId: string) => boolean
  requestScrollboxRender: () => void
  deleteTool: (mergeKey: string) => void
  maxEntries: number
  maxChars: number
}

export function createTranscriptRetentionController<TRenderable extends TranscriptRenderableHandle>(
  deps: TranscriptRetentionControllerDeps<TRenderable>,
) {
  const removeRenderable = (entryId: number) => {
    const renderable = deps.renderables.get(entryId)
    if (!renderable) {
      return
    }
    if (!deps.removeFromScrollbox(renderable.wrapper.id)) {
      return
    }
    renderable.wrapper.destroyRecursively()
    deps.renderables.delete(entryId)
  }

  const enforce = () => {
    const retention = transcriptRetentionSlice(deps.entries(), {
      maxEntries: deps.maxEntries,
      maxChars: deps.maxChars,
    })
    if (!retention.changed) {
      return
    }

    for (const entry of retention.removed) {
      removeRenderable(entry.id)
      if (entry.mergeKey) {
        deps.deleteTool(entry.mergeKey)
      }
    }
    deps.setEntries(retention.kept)
    deps.requestScrollboxRender()
  }

  return {
    enforce,
    removeRenderable,
  }
}
