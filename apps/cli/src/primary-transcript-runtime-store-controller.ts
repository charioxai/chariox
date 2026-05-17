export type PrimaryTranscriptRuntimeEntryRenderable = {
  wrapper: {
    y?: number
  }
}

export type PrimaryTranscriptRuntimeStoreOptions = {
  initialMountedTranscriptAgentId?: string | null
}

export function createPrimaryTranscriptRuntimeStoreController<
  TEntryRenderable extends PrimaryTranscriptRuntimeEntryRenderable,
  TEmptyRenderable,
  TToolUpdate,
>(options: PrimaryTranscriptRuntimeStoreOptions = {}) {
  const tools = new Map<string, TToolUpdate>()
  const activeToolLabels = new Map<string, string>()
  const transcriptRenderables = new Map<number, TEntryRenderable>()
  let emptyTranscriptRenderable: TEmptyRenderable | undefined
  let lastTranscriptScrollTop = 0
  let mountedTranscriptAgentId = options.initialMountedTranscriptAgentId ?? null

  const setEmptyRenderable = (renderable: TEmptyRenderable | undefined) => {
    emptyTranscriptRenderable = renderable
  }

  const setLastScrollTop = (scrollTop: number) => {
    lastTranscriptScrollTop = scrollTop
  }

  const entryWrapperY = (entryId: number) => transcriptRenderables.get(entryId)?.wrapper.y ?? null

  const setMountedTranscriptAgentId = (agentId: string | null) => {
    mountedTranscriptAgentId = agentId
  }

  return {
    tools,
    activeToolLabels,
    transcriptRenderables,
    getEmptyRenderable: () => emptyTranscriptRenderable,
    setEmptyRenderable,
    getLastScrollTop: () => lastTranscriptScrollTop,
    setLastScrollTop,
    getMountedTranscriptAgentId: () => mountedTranscriptAgentId,
    setMountedTranscriptAgentId,
    clearTools: () => {
      tools.clear()
    },
    deleteTool: (mergeKey: string) => {
      tools.delete(mergeKey)
    },
    activeToolLabelValues: () => activeToolLabels.values(),
    clearActiveToolLabels: () => {
      activeToolLabels.clear()
    },
    entryWrapperY,
  }
}
