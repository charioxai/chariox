import type { TranscriptEntry } from "./cli-types.js"

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
    const currentEntries = deps.entries().slice()
    let totalChars = currentEntries.reduce((sum, entry) => sum + entry.text.length, 0)
    let removeCount = 0

    while (
      currentEntries.length - removeCount > deps.maxEntries
      || (totalChars > deps.maxChars && removeCount < currentEntries.length - 1)
    ) {
      totalChars -= currentEntries[removeCount]?.text.length ?? 0
      removeCount += 1
    }

    if (removeCount === 0) {
      return
    }

    const removed = currentEntries.slice(0, removeCount)
    const kept = currentEntries.slice(removeCount)
    for (const entry of removed) {
      removeRenderable(entry.id)
      if (entry.mergeKey) {
        deps.deleteTool(entry.mergeKey)
      }
    }
    deps.setEntries(kept)
    deps.requestScrollboxRender()
  }

  return {
    enforce,
    removeRenderable,
  }
}
