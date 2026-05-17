export function createAgentPaneRuntimeStoreController<
  TScrollbox,
  TEntryRenderable,
  TEmptyRenderable,
  TToolUpdate,
>() {
  const currentAuxiliaryAgentIds: Array<string | null> = []
  const scrollboxes = new Map<string, TScrollbox>()
  const entryRenderables = new Map<string, Map<number, TEntryRenderable>>()
  const emptyRenderables = new Map<string, TEmptyRenderable>()
  const toolStates = new Map<string, Map<string, TToolUpdate>>()

  return {
    scrollboxes,
    entryRenderables,
    emptyRenderables,
    toolStates,
    toolUpdatesForAgent: (agentId: string) => toolStates.get(agentId)?.values(),
    unregisterScrollbox: (agentId: string) => {
      scrollboxes.delete(agentId)
    },
    registerScrollbox: (agentId: string, scrollbox: TScrollbox) => {
      scrollboxes.set(agentId, scrollbox)
    },
    getCurrentAuxiliaryAgentId: (auxiliaryIndex: number) => currentAuxiliaryAgentIds[auxiliaryIndex] ?? null,
    setCurrentAuxiliaryAgentId: (auxiliaryIndex: number, agentId: string | null) => {
      currentAuxiliaryAgentIds[auxiliaryIndex] = agentId
    },
    clearCurrentAuxiliaryAgentIds: () => {
      currentAuxiliaryAgentIds.length = 0
    },
  }
}
