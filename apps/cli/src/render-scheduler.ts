export type RenderTreeNode = {
  id?: string | number | undefined
  requestRender?: (() => void) | undefined
  requestRebuild?: (() => void) | undefined
  getChildren?: (() => RenderTreeNode[]) | undefined
}

type SchedulerTimer = ReturnType<typeof setTimeout>

export function createRenderScheduler(options: {
  schedule?: (callback: () => void) => SchedulerTimer
  clearSchedule?: (timer: SchedulerTimer) => void
  requestRootRender?: () => void
}) {
  const schedule = options.schedule ?? ((callback) => setTimeout(callback, 0))
  const clearSchedule = options.clearSchedule ?? clearTimeout
  const dirtyRenderables = new Set<RenderTreeNode>()
  const dirtyTrees = new Set<RenderTreeNode>()
  let rootDirty = false
  let timer: SchedulerTimer | null = null

  const ensureScheduled = () => {
    if (timer) {
      return
    }
    timer = schedule(() => {
      timer = null
      flush()
    })
  }

  const requestRenderable = (renderable: RenderTreeNode | null | undefined) => {
    if (!renderable) {
      return
    }
    dirtyRenderables.add(renderable)
    ensureScheduled()
  }

  const requestTree = (renderable: RenderTreeNode | null | undefined) => {
    if (!renderable) {
      return
    }
    dirtyTrees.add(renderable)
    ensureScheduled()
  }

  const requestRoot = () => {
    rootDirty = true
    ensureScheduled()
  }

  function flush() {
    if (timer) {
      clearSchedule(timer)
      timer = null
    }
    const seen = new Set<string | number>()
    for (const tree of [...dirtyTrees]) {
      requestRenderableTreeRender(tree, seen)
    }
    dirtyTrees.clear()
    for (const renderable of [...dirtyRenderables]) {
      const renderableId = renderable.id
      if (renderableId !== undefined && seen.has(renderableId)) {
        continue
      }
      renderable.requestRender?.()
    }
    dirtyRenderables.clear()
    if (rootDirty) {
      rootDirty = false
      options.requestRootRender?.()
    }
  }

  return {
    requestRenderable,
    requestTree,
    requestRoot,
    flush,
  }
}

export function requestRenderableTreeRender(
  renderable: RenderTreeNode | null | undefined,
  seen: Set<string | number> = new Set(),
) {
  if (!renderable) {
    return
  }
  const renderableId = renderable.id
  if (renderableId !== undefined) {
    if (seen.has(renderableId)) {
      return
    }
    seen.add(renderableId)
  }
  renderable.requestRebuild?.()
  renderable.requestRender?.()
  for (const child of renderable.getChildren?.() ?? []) {
    requestRenderableTreeRender(child, seen)
  }
}
