import type { TranscriptEntry } from "./cli-types.js"
import { reconcileMountedTranscriptPane } from "@chariox/kernel-client/transcript-pane-reconcile"

export type PrimaryTranscriptChild = {
  id: string
  destroyRecursively: () => void
}

export type PrimaryTranscriptScrollbox<TChild extends PrimaryTranscriptChild> = {
  scrollTop: number
  scrollLeft: number
  scrollHeight: number
  height: number
  getChildren: () => PrimaryTranscriptChild[]
  add: (renderable: TChild) => unknown
  remove: (id: string) => unknown
  scrollTo: (position: { x: number; y: number }) => unknown
  requestRender: () => unknown
}

export type PrimaryTranscriptEntryRenderable<TChild extends PrimaryTranscriptChild> = {
  entry: TranscriptEntry
  wrapper: TChild
  update: (entry: TranscriptEntry) => void
}

export type PrimaryTranscriptRenderControllerDeps<
  TChild extends PrimaryTranscriptChild,
  TScrollbox extends PrimaryTranscriptScrollbox<TChild>,
  TEntryRenderable extends PrimaryTranscriptEntryRenderable<TChild>,
> = {
  getScrollbox: () => TScrollbox | undefined
  getEmptyRenderable: () => TChild | undefined
  setEmptyRenderable: (renderable: TChild | undefined) => void
  renderables: Map<number, TEntryRenderable>
  visibleEntries: () => TranscriptEntry[]
  workflowScreenActive: () => boolean
  showWorkflowOutline: () => boolean
  buildWorkflowRenderable: () => TChild
  buildEmptyRenderable: () => TChild
  buildEntryRenderable: (entry: TranscriptEntry) => TEntryRenderable
  renderMode: (entry: TranscriptEntry) => unknown
  requestTranscriptRender: () => void
  requestRendererRender: () => void
  shouldResetEmptyScrollTop: () => boolean
  clampScrollTop: (scrollTop: number, scrollHeight: number, viewportHeight: number) => number
  setLastScrollTop: (scrollTop: number) => void
  logViewDebug: (phase: string, fields?: Record<string, unknown>) => void
}

export function createPrimaryTranscriptRenderController<
  TChild extends PrimaryTranscriptChild,
  TScrollbox extends PrimaryTranscriptScrollbox<TChild>,
  TEntryRenderable extends PrimaryTranscriptEntryRenderable<TChild>,
>(
  deps: PrimaryTranscriptRenderControllerDeps<TChild, TScrollbox, TEntryRenderable>,
) {
  const removeEmptyRenderable = () => {
    const empty = deps.getEmptyRenderable()
    const scrollbox = deps.getScrollbox()
    if (!empty || !scrollbox) {
      return
    }
    scrollbox.remove(empty.id)
    empty.destroyRecursively()
    deps.setEmptyRenderable(undefined)
  }

  const mountEntry = (entry: TranscriptEntry, requestRender = true) => {
    const scrollbox = deps.getScrollbox()
    if (!scrollbox) {
      return
    }

    removeEmptyRenderable()

    const renderable = deps.buildEntryRenderable(entry)
    deps.renderables.set(entry.id, renderable)
    scrollbox.add(renderable.wrapper)
    if (requestRender) {
      deps.requestTranscriptRender()
    }
  }

  const rebuildTranscript = () => {
    deps.logViewDebug("rebuild transcript:start", {
      visible_entries: deps.visibleEntries().length,
    })
    const scrollbox = deps.getScrollbox()
    if (!scrollbox) {
      deps.logViewDebug("rebuild transcript:missing scrollbox")
      return
    }

    for (const child of [...scrollbox.getChildren()]) {
      scrollbox.remove(child.id)
      child.destroyRecursively()
    }
    deps.renderables.clear()
    deps.setEmptyRenderable(undefined)

    const visibleEntries = deps.visibleEntries()
    if (deps.showWorkflowOutline()) {
      const empty = deps.buildWorkflowRenderable()
      deps.setEmptyRenderable(empty)
      scrollbox.add(empty)
      scrollbox.scrollTo({ x: scrollbox.scrollLeft, y: 0 })
    } else if (visibleEntries.length === 0) {
      const empty = deps.buildEmptyRenderable()
      deps.setEmptyRenderable(empty)
      scrollbox.add(empty)
      if (deps.shouldResetEmptyScrollTop()) {
        scrollbox.scrollTo({ x: scrollbox.scrollLeft, y: 0 })
      }
    } else {
      for (const entry of visibleEntries.filter((candidate) => !candidate.historyDeferred)) {
        mountEntry(entry, false)
      }
    }

    scrollbox.requestRender()
    deps.requestRendererRender()
    deps.logViewDebug("rebuild transcript:complete", {
      scroll_height: scrollbox.scrollHeight,
      scroll_top: scrollbox.scrollTop,
    })
  }

  const reconcileMountedTranscript = (currentEntries: TranscriptEntry[], nextEntries: TranscriptEntry[]) => {
    if (deps.workflowScreenActive()) {
      rebuildTranscript()
      return
    }
    reconcileMountedTranscriptPane({
      scrollbox: deps.getScrollbox(),
      currentEntries,
      nextEntries,
      renderables: deps.renderables,
      clampScrollTop: deps.clampScrollTop,
      rebuild: rebuildTranscript,
      removeEmptyRenderable,
      mountEntry,
      onScrollTop: deps.setLastScrollTop,
    })
  }

  const updateEntry = (entryId: number, text: string, sourceText?: string) => {
    const renderable = deps.renderables.get(entryId)
    if (!renderable) {
      rebuildTranscript()
      return
    }
    const previousMode = deps.renderMode(renderable.entry)
    renderable.entry.text = text
    if (sourceText !== undefined) {
      renderable.entry.sourceText = sourceText
    }
    if (deps.renderMode(renderable.entry) !== previousMode) {
      rebuildTranscript()
      return
    }
    renderable.update(renderable.entry)
    deps.requestTranscriptRender()
  }

  return {
    mountEntry,
    reconcileMountedTranscript,
    updateEntry,
    rebuildTranscript,
  }
}
