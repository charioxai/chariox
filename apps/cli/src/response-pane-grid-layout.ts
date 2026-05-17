import type { PaneGridModel, PaneGridTone } from "./response-pane-grid.js"

export type ResponsePaneLayoutBox = {
  id?: unknown
  visible?: unknown
  flexDirection?: unknown
  flexGrow?: unknown
  flexShrink?: unknown
  flexBasis?: unknown
  width?: unknown
  minWidth?: unknown
  maxWidth?: unknown
  minHeight?: unknown
  height?: unknown
  gap?: unknown
  paddingLeft?: unknown
  paddingRight?: unknown
  paddingTop?: unknown
  paddingBottom?: unknown
  border?: unknown
  borderColor?: unknown
  backgroundColor?: unknown
  getChildren: () => readonly unknown[]
  requestRender?: (() => unknown) | undefined
}

export type ResponsePaneLayoutText = {
  content?: unknown
  fg?: unknown
  attributes?: unknown
}

export type ResponsePaneLayoutScrollbox = {
  backgroundColor?: unknown
  requestRender?: (() => unknown) | undefined
}

export type ResponsePaneGridLayoutTheme = {
  primary: unknown
  borderSubtle: unknown
  backgroundPanel: unknown
  backgroundElement: unknown
}

export function applyResponsePaneGridLayout(options: {
  layoutBox: ResponsePaneLayoutBox | undefined
  primaryPane: ResponsePaneLayoutBox | undefined
  primaryInteractionBox: ResponsePaneLayoutBox | undefined
  primaryFooterBox: ResponsePaneLayoutBox | undefined
  primaryScrollbox: ResponsePaneLayoutScrollbox | undefined
  historyLoadingBox: ResponsePaneLayoutBox | undefined
  auxiliaryPanes: readonly (ResponsePaneLayoutBox | undefined)[]
  auxiliaryInteractionBoxes: readonly (ResponsePaneLayoutBox | undefined)[]
  auxiliaryFooterBoxes: readonly (ResponsePaneLayoutBox | undefined)[]
  auxiliaryScrollboxes: readonly (ResponsePaneLayoutScrollbox | undefined)[]
  rowBoxes: readonly (ResponsePaneLayoutBox | undefined)[]
  borderRows: readonly (ResponsePaneLayoutBox | undefined)[]
  horizontalSegments: readonly (readonly (ResponsePaneLayoutBox | undefined)[] | undefined)[]
  verticalSegments: readonly (readonly (ResponsePaneLayoutBox | undefined)[] | undefined)[]
  junctionTexts: readonly (readonly (ResponsePaneLayoutText | undefined)[] | undefined)[]
  bottomBorderRow: ResponsePaneLayoutBox | undefined
  bottomHorizontalSegments: readonly (ResponsePaneLayoutBox | undefined)[]
  bottomJunctionTexts: readonly (ResponsePaneLayoutText | undefined)[]
  paneRows: readonly (readonly number[])[]
  paneGrid: PaneGridModel
  split: boolean
  showWorkflowScreen: boolean
  theme: ResponsePaneGridLayoutTheme
  emptyTextAttributes: unknown
  panelBackgroundForFocus: (focused: boolean) => unknown
  onMissingRefs?: (details: {
    hasLayoutBox: boolean
    hasPrimaryPane: boolean
    auxiliaryPaneCount: number
  }) => void
}) {
  const primaryPane = options.primaryPane
  if (!options.layoutBox || !primaryPane) {
    options.onMissingRefs?.({
      hasLayoutBox: Boolean(options.layoutBox),
      hasPrimaryPane: Boolean(primaryPane),
      auxiliaryPaneCount: options.auxiliaryPanes.filter(Boolean).length,
    })
    return false
  }

  options.layoutBox.flexDirection = "column"
  options.layoutBox.gap = 0

  options.paneGrid.rows.forEach((gridRow, rowIndex) => {
    const rowBox = options.rowBoxes[rowIndex]
    if (!rowBox) {
      return
    }
    const borderRow = options.paneGrid.borderRows[rowIndex]
    if (borderRow) {
      applyBorderRowBox(options.borderRows[rowIndex], borderRow.visible)
      borderRow.horizontals.forEach((segment, segmentIndex) => {
        applyHorizontalSegment(
          options.horizontalSegments[rowIndex]?.[segmentIndex],
          segment.visible,
          segment.tone,
          options.theme,
        )
      })
      borderRow.junctions.forEach((junction, junctionIndex) => {
        applyJunctionText(
          options.junctionTexts[rowIndex]?.[junctionIndex],
          junction.visible,
          junction.char,
          junction.tone,
          options.theme,
          options.emptyTextAttributes,
        )
      })
    }

    rowBox.visible = rowIndex === 0 || gridRow.visible
    rowBox.flexDirection = "row"
    rowBox.gap = 0
    rowBox.flexGrow = rowBox.visible ? 1 : 0
    rowBox.flexBasis = 0
    rowBox.border = false
    rowBox.requestRender?.()

    gridRow.verticals.forEach((segment, segmentIndex) => {
      applyVerticalSegment(
        options.verticalSegments[rowIndex]?.[segmentIndex],
        segment.visible,
        segment.tone,
        options.theme,
      )
    })

    for (const slot of gridRow.slots) {
      if (slot.paneIndex === 0) {
        layoutPane({
          pane: primaryPane,
          interactionBox: options.primaryInteractionBox,
          footerBox: options.primaryFooterBox,
          scrollbox: options.primaryScrollbox,
          focused: slot.focused,
          visible: true,
          showFooter: !options.showWorkflowScreen,
          defaultBackground: options.theme.backgroundPanel,
          split: options.split,
          theme: options.theme,
          panelBackgroundForFocus: options.panelBackgroundForFocus,
        })
        if (options.historyLoadingBox) {
          options.historyLoadingBox.backgroundColor = primaryPane.backgroundColor
          options.historyLoadingBox.borderColor = options.split && slot.focused ? options.theme.primary : options.theme.borderSubtle
          options.historyLoadingBox.requestRender?.()
        }
        continue
      }

      const auxiliaryIndex = slot.paneIndex - 1
      layoutPane({
        pane: options.auxiliaryPanes[auxiliaryIndex],
        interactionBox: options.auxiliaryInteractionBoxes[auxiliaryIndex],
        footerBox: options.auxiliaryFooterBoxes[auxiliaryIndex],
        scrollbox: options.auxiliaryScrollboxes[auxiliaryIndex],
        focused: slot.focused,
        visible: true,
        showFooter: Boolean(slot.agentId),
        defaultBackground: options.theme.backgroundElement,
        split: options.split,
        theme: options.theme,
        panelBackgroundForFocus: options.panelBackgroundForFocus,
      })
    }

    for (const paneIndex of options.paneRows[rowIndex] ?? []) {
      if (gridRow.slots.some((slot) => slot.paneIndex === paneIndex)) {
        continue
      }
      if (paneIndex === 0) {
        layoutPane({
          pane: primaryPane,
          interactionBox: options.primaryInteractionBox,
          footerBox: options.primaryFooterBox,
          scrollbox: options.primaryScrollbox,
          focused: false,
          visible: false,
          showFooter: false,
          defaultBackground: options.theme.backgroundPanel,
          split: options.split,
          theme: options.theme,
          panelBackgroundForFocus: options.panelBackgroundForFocus,
        })
        continue
      }

      const auxiliaryIndex = paneIndex - 1
      layoutPane({
        pane: options.auxiliaryPanes[auxiliaryIndex],
        interactionBox: options.auxiliaryInteractionBoxes[auxiliaryIndex],
        footerBox: options.auxiliaryFooterBoxes[auxiliaryIndex],
        scrollbox: options.auxiliaryScrollboxes[auxiliaryIndex],
        focused: false,
        visible: false,
        showFooter: false,
        defaultBackground: options.theme.backgroundElement,
        split: options.split,
        theme: options.theme,
        panelBackgroundForFocus: options.panelBackgroundForFocus,
      })
    }
  })

  const bottomBorderRow = options.paneGrid.borderRows[options.paneGrid.rows.length]
  if (bottomBorderRow) {
    applyBorderRowBox(options.bottomBorderRow, bottomBorderRow.visible)
    bottomBorderRow.horizontals.forEach((segment, segmentIndex) => {
      applyHorizontalSegment(
        options.bottomHorizontalSegments[segmentIndex],
        segment.visible,
        segment.tone,
        options.theme,
      )
    })
    bottomBorderRow.junctions.forEach((junction, junctionIndex) => {
      applyJunctionText(
        options.bottomJunctionTexts[junctionIndex],
        junction.visible,
        junction.char,
        junction.tone,
        options.theme,
        options.emptyTextAttributes,
      )
    })
  }

  return true
}

function layoutPane(options: {
  pane: ResponsePaneLayoutBox | undefined
  interactionBox: ResponsePaneLayoutBox | undefined
  footerBox: ResponsePaneLayoutBox | undefined
  scrollbox: ResponsePaneLayoutScrollbox | undefined
  focused: boolean
  visible: boolean
  showFooter: boolean
  defaultBackground: unknown
  split: boolean
  theme: ResponsePaneGridLayoutTheme
  panelBackgroundForFocus: (focused: boolean) => unknown
}) {
  if (!options.pane) {
    return
  }

  options.pane.visible = options.visible
  options.pane.flexDirection = "column"
  options.pane.flexGrow = options.visible ? 1 : 0
  options.pane.flexBasis = options.visible ? 0 : 0
  options.pane.width = options.visible ? "auto" : 0
  options.pane.minWidth = options.visible && options.split ? 0 : null
  options.pane.maxWidth = null
  options.pane.paddingLeft = 0
  options.pane.paddingRight = 0
  options.pane.paddingTop = 0
  options.pane.paddingBottom = 0
  options.pane.border = false
  options.pane.borderColor = options.theme.borderSubtle
  options.pane.backgroundColor = options.visible && options.split
    ? options.panelBackgroundForFocus(options.focused)
    : options.defaultBackground

  if (options.interactionBox) {
    options.interactionBox.visible = options.visible && Boolean(options.interactionBox.getChildren().length)
    options.interactionBox.requestRender?.()
  }
  if (options.footerBox) {
    options.footerBox.visible = options.visible && options.showFooter
    options.footerBox.requestRender?.()
  }
  if (options.scrollbox) {
    options.scrollbox.backgroundColor = options.pane.backgroundColor
    options.scrollbox.requestRender?.()
  }
  options.pane.requestRender?.()
}

function applyBorderRowBox(box: ResponsePaneLayoutBox | undefined, visible: boolean) {
  if (!box) {
    return
  }
  box.visible = visible
  box.height = visible ? 1 : 0
  box.minHeight = visible ? 1 : 0
  box.flexGrow = 0
  box.flexShrink = 0
  box.flexDirection = "row"
  box.gap = 0
  box.requestRender?.()
}

function applyHorizontalSegment(
  segmentBox: ResponsePaneLayoutBox | undefined,
  visible: boolean,
  tone: PaneGridTone,
  theme: ResponsePaneGridLayoutTheme,
) {
  if (!segmentBox) {
    return
  }
  segmentBox.visible = visible
  segmentBox.height = 1
  segmentBox.minHeight = 1
  segmentBox.flexGrow = visible ? 1 : 0
  segmentBox.flexBasis = 0
  segmentBox.border = visible ? ["top"] : false
  segmentBox.borderColor = borderColor(tone, theme)
  segmentBox.requestRender?.()
}

function applyVerticalSegment(
  segmentBox: ResponsePaneLayoutBox | undefined,
  visible: boolean,
  tone: PaneGridTone,
  theme: ResponsePaneGridLayoutTheme,
) {
  if (!segmentBox) {
    return
  }
  segmentBox.visible = visible
  segmentBox.width = visible ? 1 : 0
  segmentBox.minWidth = visible ? 1 : 0
  segmentBox.flexGrow = 0
  segmentBox.flexShrink = 0
  segmentBox.border = visible ? ["left"] : false
  segmentBox.borderColor = borderColor(tone, theme)
  segmentBox.requestRender?.()
}

function applyJunctionText(
  text: ResponsePaneLayoutText | undefined,
  visible: boolean,
  char: string,
  tone: PaneGridTone,
  theme: ResponsePaneGridLayoutTheme,
  emptyTextAttributes: unknown,
) {
  if (!text) {
    return
  }
  text.content = visible ? char : ""
  text.fg = borderColor(tone, theme)
  text.attributes = emptyTextAttributes
}

function borderColor(tone: PaneGridTone, theme: ResponsePaneGridLayoutTheme) {
  return tone === "focused" ? theme.primary : theme.borderSubtle
}
