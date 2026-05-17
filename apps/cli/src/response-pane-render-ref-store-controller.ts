export type ResponsePaneRenderRefSnapshot<TBox, TScrollbox, TText> = {
  layoutBox: TBox | undefined
  primaryPane: TBox | undefined
  primaryInteractionBox: TBox | undefined
  primaryFooterBox: TBox | undefined
  primaryScrollbox: TScrollbox | undefined
  historyLoadingBox: TBox | undefined
  auxiliaryPanes: readonly (TBox | undefined)[]
  auxiliaryInteractionBoxes: readonly (TBox | undefined)[]
  auxiliaryFooterBoxes: readonly (TBox | undefined)[]
  auxiliaryScrollboxes: readonly (TScrollbox | undefined)[]
  rowBoxes: readonly (TBox | undefined)[]
  borderRows: readonly (TBox | undefined)[]
  horizontalSegments: readonly (readonly (TBox | undefined)[] | undefined)[]
  verticalSegments: readonly (readonly (TBox | undefined)[] | undefined)[]
  junctionTexts: readonly (readonly (TText | undefined)[] | undefined)[]
  bottomBorderRow: TBox | undefined
  bottomHorizontalSegments: readonly (TBox | undefined)[]
  bottomJunctionTexts: readonly (TText | undefined)[]
}

export function createResponsePaneRenderRefStoreController<TBox, TScrollbox, TText>() {
  let layoutBox: TBox | undefined
  let primaryPane: TBox | undefined
  let primaryInteractionBox: TBox | undefined
  let primaryFooterBox: TBox | undefined
  let bottomBorderRow: TBox | undefined
  const auxiliaryPanes: Array<TBox | undefined> = []
  const auxiliaryInteractionBoxes: Array<TBox | undefined> = []
  const auxiliaryFooterBoxes: Array<TBox | undefined> = []
  const auxiliaryScrollboxes: Array<TScrollbox | undefined> = []
  const rowBoxes: Array<TBox | undefined> = []
  const borderRows: Array<TBox | undefined> = []
  const horizontalSegments: Array<Array<TBox | undefined> | undefined> = []
  const bottomHorizontalSegments: Array<TBox | undefined> = []
  const junctionTexts: Array<Array<TText | undefined> | undefined> = []
  const bottomJunctionTexts: Array<TText | undefined> = []
  const verticalSegments: Array<Array<TBox | undefined> | undefined> = []

  return {
    getLayoutBox() {
      return layoutBox
    },
    getPrimaryInteractionBox() {
      return primaryInteractionBox
    },
    getAuxiliaryInteractionBoxes() {
      return auxiliaryInteractionBoxes
    },
    getPrimaryFooterBox() {
      return primaryFooterBox
    },
    getAuxiliaryFooterBoxes() {
      return auxiliaryFooterBoxes
    },
    assignLayoutBox(value: TBox | undefined) {
      layoutBox = value
    },
    assignRowBox(index: number, value: TBox | undefined) {
      rowBoxes[index] = value
    },
    assignBorderRow(index: number, value: TBox | undefined) {
      borderRows[index] = value
    },
    assignBottomBorderRow(value: TBox | undefined) {
      bottomBorderRow = value
    },
    assignHorizontalSegment(rowIndex: number, segmentIndex: number, value: TBox | undefined) {
      horizontalSegments[rowIndex] ??= []
      horizontalSegments[rowIndex][segmentIndex] = value
    },
    assignBottomHorizontalSegment(segmentIndex: number, value: TBox | undefined) {
      bottomHorizontalSegments[segmentIndex] = value
    },
    assignJunctionText(rowIndex: number, junctionIndex: number, value: TText | undefined) {
      junctionTexts[rowIndex] ??= []
      junctionTexts[rowIndex][junctionIndex] = value
    },
    assignBottomJunctionText(junctionIndex: number, value: TText | undefined) {
      bottomJunctionTexts[junctionIndex] = value
    },
    assignVerticalSegment(rowIndex: number, segmentIndex: number, value: TBox | undefined) {
      verticalSegments[rowIndex] ??= []
      verticalSegments[rowIndex][segmentIndex] = value
    },
    assignPrimaryPane(value: TBox | undefined) {
      primaryPane = value
    },
    assignPrimaryInteractionBox(value: TBox | undefined) {
      primaryInteractionBox = value
    },
    assignPrimaryFooterBox(value: TBox | undefined) {
      primaryFooterBox = value
    },
    assignAuxiliaryPane(index: number, value: TBox | undefined) {
      auxiliaryPanes[index] = value
    },
    assignAuxiliaryScrollbox(index: number, value: TScrollbox | undefined) {
      auxiliaryScrollboxes[index] = value
    },
    assignAuxiliaryInteractionBox(index: number, value: TBox | undefined) {
      auxiliaryInteractionBoxes[index] = value
    },
    assignAuxiliaryFooterBox(index: number, value: TBox | undefined) {
      auxiliaryFooterBoxes[index] = value
    },
    snapshot(options: {
      primaryScrollbox: TScrollbox | undefined
      historyLoadingBox: TBox | undefined
    }): ResponsePaneRenderRefSnapshot<TBox, TScrollbox, TText> {
      return {
        layoutBox,
        primaryPane,
        primaryInteractionBox,
        primaryFooterBox,
        primaryScrollbox: options.primaryScrollbox,
        historyLoadingBox: options.historyLoadingBox,
        auxiliaryPanes,
        auxiliaryInteractionBoxes,
        auxiliaryFooterBoxes,
        auxiliaryScrollboxes,
        rowBoxes,
        borderRows,
        horizontalSegments,
        verticalSegments,
        junctionTexts,
        bottomBorderRow,
        bottomHorizontalSegments,
        bottomJunctionTexts,
      }
    },
  }
}
