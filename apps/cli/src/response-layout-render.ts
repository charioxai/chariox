import type { SplitPaneGeometry } from "./response-panes.js"

type RenderRequester = {
  requestRender?: () => void
}

type RenderableTreeNode = RenderRequester & {
  id?: string | number | undefined
  requestRebuild?: (() => void) | undefined
  getChildren?: (() => RenderableTreeNode[]) | undefined
}

type LayoutBoxRenderable = RenderRequester & {
  visible?: boolean | undefined
  flexDirection?: string | null | undefined
  gap?: number | string | undefined
  flexGrow?: number | null | undefined
  width?: unknown
  flexBasis?: unknown
  minHeight?: number | string | null | undefined
  backgroundColor?: unknown
  borderColor?: unknown
}

type LayoutPaneRenderable = LayoutBoxRenderable & {
  border?: boolean | string[] | undefined
  minWidth?: number | string | null | undefined
  maxWidth?: number | string | null | undefined
  maxHeight?: number | string | null | undefined
  paddingLeft?: number | string | null | undefined
  paddingRight?: number | string | null | undefined
  paddingTop?: number | string | null | undefined
  paddingBottom?: number | string | null | undefined
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

export type ResponseLayoutRenderables = {
  responseLayoutBox: LayoutBoxRenderable
  responseTopRowBox: LayoutBoxRenderable
  responsePrimaryPane: LayoutPaneRenderable
  responseSecondaryPane: LayoutPaneRenderable
  responseTertiaryPane: LayoutPaneRenderable
  historyLoadingBox: LayoutPaneRenderable | undefined
  transcriptScrollbox: (RenderRequester & { backgroundColor?: unknown }) | undefined
  responseSecondaryScrollbox: AuxiliaryPaneScrollbox | undefined
  responseTertiaryScrollbox: AuxiliaryPaneScrollbox | undefined
  responsePrimaryFooterBox: LayoutBoxRenderable | undefined
  responseSecondaryFooterBox: LayoutBoxRenderable | undefined
  responseTertiaryFooterBox: LayoutBoxRenderable | undefined
}

export type ApplyResponseLayoutRenderablesOptions = {
  renderables: ResponseLayoutRenderables
  geometry: SplitPaneGeometry
  split: boolean
  primaryFocused: boolean
  secondaryFocused: boolean
  tertiaryFocused: boolean
  primaryBackground: unknown
  secondaryBackground: unknown
  tertiaryBackground: unknown
  primaryBorderColor: unknown
  secondaryBorderColor: unknown
  tertiaryBorderColor: unknown
  subtleBorderColor: unknown
}

function resolveLayoutValue<T>(value: T | "auto"): T | undefined {
  return value === "auto" ? undefined : value
}

export function requestRenderableTreeRender(
  renderable: RenderableTreeNode | null | undefined,
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

export function applyResponseLayoutRenderables(options: ApplyResponseLayoutRenderablesOptions) {
  const {
    renderables,
    geometry,
    split,
    primaryFocused,
    secondaryFocused,
    tertiaryFocused,
    primaryBackground,
    secondaryBackground,
    tertiaryBackground,
    primaryBorderColor,
    secondaryBorderColor,
    tertiaryBorderColor,
    subtleBorderColor,
  } = options

  renderables.responseLayoutBox.flexDirection = geometry.layoutDirection
  renderables.responseLayoutBox.gap = geometry.layoutGap
  renderables.responseTopRowBox.visible = geometry.topRowVisible
  renderables.responseTopRowBox.flexDirection = "row"
  renderables.responseTopRowBox.gap = geometry.topRowGap
  renderables.responseTopRowBox.flexGrow = 1
  renderables.responseTopRowBox.width = undefined
  renderables.responseTopRowBox.flexBasis = resolveLayoutValue(geometry.topRowFlexBasis)
  renderables.responseTopRowBox.minHeight = geometry.topRowMinHeight

  renderables.responsePrimaryPane.border = split ? ["left", "top", "bottom"] : ["left"]
  renderables.responsePrimaryPane.borderColor = split ? (primaryFocused ? primaryBorderColor : subtleBorderColor) : subtleBorderColor
  renderables.responsePrimaryPane.backgroundColor = primaryBackground
  renderables.responsePrimaryPane.flexGrow = geometry.primaryFlexGrow
  renderables.responsePrimaryPane.width = resolveLayoutValue(geometry.primaryWidth)
  renderables.responsePrimaryPane.flexBasis = resolveLayoutValue(geometry.primaryFlexBasis)
  renderables.responsePrimaryPane.minWidth = geometry.primaryMinWidth
  renderables.responsePrimaryPane.maxWidth = geometry.primaryMaxWidth

  renderables.responseSecondaryPane.visible = geometry.showSecondaryPane
  renderables.responseSecondaryPane.width = geometry.secondaryWidth
  renderables.responseSecondaryPane.flexBasis = geometry.secondaryFlexBasis
  renderables.responseSecondaryPane.minWidth = geometry.secondaryMinWidth
  renderables.responseSecondaryPane.maxWidth = geometry.secondaryMaxWidth
  renderables.responseSecondaryPane.border = geometry.showSecondaryPane ? ["left", "top", "bottom", "right"] : false
  renderables.responseSecondaryPane.borderColor = secondaryFocused ? secondaryBorderColor : subtleBorderColor
  renderables.responseSecondaryPane.backgroundColor = secondaryBackground
  renderables.responseSecondaryPane.paddingLeft = 0
  renderables.responseSecondaryPane.paddingRight = 0
  renderables.responseSecondaryPane.paddingTop = 0
  renderables.responseSecondaryPane.paddingBottom = 0

  renderables.responseTertiaryPane.visible = geometry.showTertiaryPane
  renderables.responseTertiaryPane.width = resolveLayoutValue(geometry.tertiaryWidth)
  renderables.responseTertiaryPane.flexGrow = geometry.tertiaryFlexGrow
  renderables.responseTertiaryPane.flexBasis = geometry.tertiaryFlexBasis
  renderables.responseTertiaryPane.minHeight = geometry.tertiaryMinHeight
  renderables.responseTertiaryPane.maxHeight = null
  renderables.responseTertiaryPane.border = geometry.showTertiaryPane ? ["left", "top", "bottom", "right"] : false
  renderables.responseTertiaryPane.borderColor = tertiaryFocused ? tertiaryBorderColor : subtleBorderColor
  renderables.responseTertiaryPane.backgroundColor = tertiaryBackground
  renderables.responseTertiaryPane.paddingLeft = 0
  renderables.responseTertiaryPane.paddingRight = 0
  renderables.responseTertiaryPane.paddingTop = 0
  renderables.responseTertiaryPane.paddingBottom = 0

  if (renderables.historyLoadingBox) {
    renderables.historyLoadingBox.backgroundColor = primaryBackground
    renderables.historyLoadingBox.borderColor = split && primaryFocused ? primaryBorderColor : subtleBorderColor
    renderables.historyLoadingBox.requestRender?.()
  }
  if (renderables.transcriptScrollbox) {
    renderables.transcriptScrollbox.backgroundColor = primaryBackground
    renderables.transcriptScrollbox.requestRender?.()
  }
  if (renderables.responseSecondaryScrollbox) {
    renderables.responseSecondaryScrollbox.backgroundColor = secondaryBackground
    renderables.responseSecondaryScrollbox.requestRender?.()
  }
  if (renderables.responseTertiaryScrollbox) {
    renderables.responseTertiaryScrollbox.backgroundColor = tertiaryBackground
    renderables.responseTertiaryScrollbox.requestRender?.()
  }
  if (renderables.responsePrimaryFooterBox) {
    renderables.responsePrimaryFooterBox.visible = split
    renderables.responsePrimaryFooterBox.backgroundColor = primaryBackground
    renderables.responsePrimaryFooterBox.requestRender?.()
  }
  if (renderables.responseSecondaryFooterBox) {
    renderables.responseSecondaryFooterBox.visible = geometry.showSecondaryPane
    renderables.responseSecondaryFooterBox.backgroundColor = secondaryBackground
    renderables.responseSecondaryFooterBox.requestRender?.()
  }
  if (renderables.responseTertiaryFooterBox) {
    renderables.responseTertiaryFooterBox.visible = geometry.showTertiaryPane
    renderables.responseTertiaryFooterBox.backgroundColor = tertiaryBackground
    renderables.responseTertiaryFooterBox.requestRender?.()
  }

  renderables.responseTopRowBox.requestRender?.()
  renderables.responsePrimaryPane.requestRender?.()
  renderables.responseSecondaryPane.requestRender?.()
  renderables.responseTertiaryPane.requestRender?.()
  renderables.responseLayoutBox.requestRender?.()
  renderables.transcriptScrollbox?.requestRender?.()
  renderables.responseSecondaryScrollbox?.requestRender?.()
  renderables.responseTertiaryScrollbox?.requestRender?.()

  return {
    splitPaneWidth: geometry.splitPaneWidth,
    secondaryVisible: renderables.responseSecondaryPane.visible,
    tertiaryVisible: renderables.responseTertiaryPane.visible,
    primaryWidth: renderables.responsePrimaryPane.width,
    secondaryWidth: renderables.responseSecondaryPane.width,
  }
}
