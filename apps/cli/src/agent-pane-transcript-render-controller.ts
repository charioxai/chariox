import type { TranscriptEntry } from "./cli-types.js"
import { reconcileMountedTranscriptPane } from "./transcript-pane-reconcile.js"

export type AgentPaneTranscriptChild = {
  id: string
  destroyRecursively: () => void
}

export type AgentPaneTranscriptScrollbox<TChild extends AgentPaneTranscriptChild> = {
  scrollTop: number
  scrollLeft: number
  scrollHeight: number
  height: number
  getChildren: () => AgentPaneTranscriptChild[]
  add: (renderable: TChild) => unknown
  remove: (id: string) => unknown
  scrollTo: (position: { x: number; y: number }) => unknown
  requestRender: () => unknown
}

export type AgentPaneTranscriptEntryRenderable<TChild extends AgentPaneTranscriptChild> = {
  entry: TranscriptEntry
  wrapper: TChild
  update: (entry: TranscriptEntry) => void
}

export type AgentPaneTranscriptRenderControllerDeps<
  TChild extends AgentPaneTranscriptChild,
  TScrollbox extends AgentPaneTranscriptScrollbox<TChild>,
  TEntryRenderable extends AgentPaneTranscriptEntryRenderable<TChild>,
  TToolUpdate,
  TSession,
> = {
  scrollboxes: Map<string, TScrollbox>
  entryRenderables: Map<string, Map<number, TEntryRenderable>>
  emptyRenderables: Map<string, TChild>
  toolStates: Map<string, Map<string, TToolUpdate>>
  paneEntries: (agentId: string) => TranscriptEntry[]
  buildEmptyRenderable: () => TChild
  buildEntryRenderable: (agentId: string, entry: TranscriptEntry) => TEntryRenderable
  renderMode: (entry: TranscriptEntry) => unknown
  requestRenderable: (renderable: TScrollbox | undefined) => void
  clampScrollTop: (scrollTop: number, scrollHeight: number, viewportHeight: number) => number
  activeAgentIdsForSession: (session: TSession) => Iterable<string>
}

export function createAgentPaneTranscriptRenderController<
  TChild extends AgentPaneTranscriptChild,
  TScrollbox extends AgentPaneTranscriptScrollbox<TChild>,
  TEntryRenderable extends AgentPaneTranscriptEntryRenderable<TChild>,
  TToolUpdate,
  TSession,
>(
  deps: AgentPaneTranscriptRenderControllerDeps<TChild, TScrollbox, TEntryRenderable, TToolUpdate, TSession>,
) {
  const entryRenderablesForAgent = (agentId: string) => {
    let renderables = deps.entryRenderables.get(agentId)
    if (!renderables) {
      renderables = new Map<number, TEntryRenderable>()
      deps.entryRenderables.set(agentId, renderables)
    }
    return renderables
  }

  const toolStateForAgent = (agentId: string) => {
    let toolState = deps.toolStates.get(agentId)
    if (!toolState) {
      toolState = new Map<string, TToolUpdate>()
      deps.toolStates.set(agentId, toolState)
    }
    return toolState
  }

  const clearPane = (agentId: string) => {
    const scrollbox = deps.scrollboxes.get(agentId)
    if (scrollbox) {
      for (const child of [...scrollbox.getChildren()]) {
        scrollbox.remove(child.id)
        child.destroyRecursively()
      }
      scrollbox.requestRender()
    }
    deps.entryRenderables.delete(agentId)
    deps.emptyRenderables.delete(agentId)
  }

  const rebuildPane = (agentId: string) => {
    const scrollbox = deps.scrollboxes.get(agentId)
    if (!scrollbox) {
      return
    }

    clearPane(agentId)

    const paneEntries = deps.paneEntries(agentId)
    if (paneEntries.length === 0) {
      const empty = deps.buildEmptyRenderable()
      deps.emptyRenderables.set(agentId, empty)
      scrollbox.add(empty)
      scrollbox.requestRender()
      return
    }

    const renderables = entryRenderablesForAgent(agentId)
    for (const entry of paneEntries.filter((candidate) => !candidate.historyDeferred)) {
      const renderable = deps.buildEntryRenderable(agentId, entry)
      renderables.set(entry.id, renderable)
      scrollbox.add(renderable.wrapper)
    }
    scrollbox.requestRender()
  }

  const mountEntry = (agentId: string, entry: TranscriptEntry, requestRender = true) => {
    const scrollbox = deps.scrollboxes.get(agentId)
    if (!scrollbox) {
      return
    }

    const empty = deps.emptyRenderables.get(agentId)
    if (empty) {
      scrollbox.remove(empty.id)
      empty.destroyRecursively()
      deps.emptyRenderables.delete(agentId)
    }

    const renderable = deps.buildEntryRenderable(agentId, entry)
    entryRenderablesForAgent(agentId).set(entry.id, renderable)
    scrollbox.add(renderable.wrapper)
    if (requestRender) {
      scrollbox.requestRender()
    }
  }

  const updateEntry = (agentId: string, nextEntry: TranscriptEntry) => {
    const renderable = entryRenderablesForAgent(agentId).get(nextEntry.id)
    if (!renderable) {
      rebuildPane(agentId)
      return
    }
    const previousMode = deps.renderMode(renderable.entry)
    if (deps.renderMode(nextEntry) !== previousMode) {
      rebuildPane(agentId)
      return
    }
    renderable.entry = nextEntry
    renderable.update(nextEntry)
    deps.requestRenderable(deps.scrollboxes.get(agentId))
  }

  const removeEmptyRenderable = (agentId: string) => {
    const empty = deps.emptyRenderables.get(agentId)
    if (!empty) {
      return
    }
    deps.scrollboxes.get(agentId)?.remove(empty.id)
    empty.destroyRecursively()
    deps.emptyRenderables.delete(agentId)
  }

  const reconcileMountedTranscript = (
    agentId: string,
    currentEntries: TranscriptEntry[],
    nextEntries: TranscriptEntry[],
  ) => {
    reconcileMountedTranscriptPane({
      scrollbox: deps.scrollboxes.get(agentId),
      currentEntries,
      nextEntries,
      renderables: entryRenderablesForAgent(agentId),
      clampScrollTop: deps.clampScrollTop,
      rebuild: () => rebuildPane(agentId),
      removeEmptyRenderable: () => removeEmptyRenderable(agentId),
      mountEntry: (entry, requestRender) => mountEntry(agentId, entry, requestRender),
    })
  }

  const prunePanes = (session: TSession) => {
    const activeAgentIds = new Set(deps.activeAgentIdsForSession(session))
    for (const agentId of deps.scrollboxes.keys()) {
      if (!activeAgentIds.has(agentId)) {
        deps.scrollboxes.delete(agentId)
      }
    }
    for (const agentId of deps.entryRenderables.keys()) {
      if (!activeAgentIds.has(agentId)) {
        deps.entryRenderables.delete(agentId)
      }
    }
    for (const agentId of deps.emptyRenderables.keys()) {
      if (!activeAgentIds.has(agentId)) {
        deps.emptyRenderables.delete(agentId)
      }
    }
    for (const agentId of deps.toolStates.keys()) {
      if (!activeAgentIds.has(agentId)) {
        deps.toolStates.delete(agentId)
      }
    }
  }

  const clearAll = () => {
    deps.scrollboxes.clear()
    deps.entryRenderables.clear()
    deps.emptyRenderables.clear()
    deps.toolStates.clear()
  }

  return {
    clearAll,
    clearPane,
    rebuildPane,
    mountEntry,
    updateEntry,
    reconcileMountedTranscript,
    prunePanes,
    toolStateForAgent,
  }
}
