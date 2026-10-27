export { requestRenderableTreeRender } from "./render-scheduler.js"

type RenderRequester = {
  requestRender?: () => void
}

type AuxiliaryPaneChild = {
  id: string | number
  destroyRecursively?: () => void
}

type AuxiliaryPaneScrollbox<TChild = any> = RenderRequester & {
  backgroundColor?: unknown
  getChildren: () => AuxiliaryPaneChild[]
  remove: (id: string) => unknown
  add: (child: TChild) => unknown
}

export function syncAuxiliaryPane<TChild extends AuxiliaryPaneChild, TScrollbox extends AuxiliaryPaneScrollbox<TChild>>(options: {
  scrollbox: TScrollbox | undefined
  nextAgentId: string | null
  currentAgentId: string | null
  splitMode: boolean
  clearAuxiliaryAgentPane: (agentId: string) => void
  unregisterAgentScrollbox: (agentId: string) => void
  assignCurrentAgentId: (value: string | null) => void
  registerAgentScrollbox: (agentId: string, scrollbox: TScrollbox) => void
  rebuildAuxiliaryAgentPane: (agentId: string) => void
  buildEmptyTranscriptRenderable: () => TChild
}) {
  const { scrollbox, nextAgentId, currentAgentId } = options
  if (!scrollbox) {
    return
  }

  if (currentAgentId && currentAgentId !== nextAgentId) {
    options.clearAuxiliaryAgentPane(currentAgentId)
    options.unregisterAgentScrollbox(currentAgentId)
    options.assignCurrentAgentId(null)
  }

  if (!nextAgentId) {
    for (const child of [...scrollbox.getChildren()]) {
      scrollbox.remove(String(child.id))
      child.destroyRecursively?.()
    }
    if (options.splitMode) {
      scrollbox.add(options.buildEmptyTranscriptRenderable())
    }
    scrollbox.requestRender?.()
    return
  }

  options.assignCurrentAgentId(nextAgentId)
  options.registerAgentScrollbox(nextAgentId, scrollbox)
  if (currentAgentId === nextAgentId) {
    return
  }
  options.rebuildAuxiliaryAgentPane(nextAgentId)
}
