import assert from "node:assert/strict"
import test from "node:test"

import { createResponsePaneRenderRefStoreController } from "./response-pane-render-ref-store-controller.js"

test("response pane render ref store owns mounted pane layout refs", () => {
  const store = createResponsePaneRenderRefStoreController<string, string, string>()

  store.assignLayoutBox("layout")
  store.assignPrimaryPane("primary-pane")
  store.assignPrimaryInteractionBox("primary-interaction")
  store.assignPrimaryFooterBox("primary-footer")
  store.assignAuxiliaryPane(1, "aux-pane")
  store.assignAuxiliaryScrollbox(1, "aux-scroll")
  store.assignAuxiliaryInteractionBox(1, "aux-interaction")
  store.assignAuxiliaryFooterBox(1, "aux-footer")
  store.assignRowBox(2, "row")
  store.assignBorderRow(2, "border")
  store.assignBottomBorderRow("bottom-border")
  store.assignHorizontalSegment(2, 3, "horizontal")
  store.assignBottomHorizontalSegment(3, "bottom-horizontal")
  store.assignJunctionText(2, 3, "junction")
  store.assignBottomJunctionText(3, "bottom-junction")
  store.assignVerticalSegment(2, 3, "vertical")

  assert.equal(store.getLayoutBox(), "layout")
  assert.equal(store.getPrimaryInteractionBox(), "primary-interaction")
  assert.equal(store.getPrimaryFooterBox(), "primary-footer")
  assert.equal(store.getAuxiliaryInteractionBoxes()[1], "aux-interaction")
  assert.equal(store.getAuxiliaryFooterBoxes()[1], "aux-footer")

  const snapshot = store.snapshot({
    primaryScrollbox: "primary-scroll",
    historyLoadingBox: "history-loading",
  })

  assert.equal(snapshot.layoutBox, "layout")
  assert.equal(snapshot.primaryPane, "primary-pane")
  assert.equal(snapshot.primaryScrollbox, "primary-scroll")
  assert.equal(snapshot.historyLoadingBox, "history-loading")
  assert.equal(snapshot.auxiliaryPanes[1], "aux-pane")
  assert.equal(snapshot.auxiliaryScrollboxes[1], "aux-scroll")
  assert.equal(snapshot.rowBoxes[2], "row")
  assert.equal(snapshot.borderRows[2], "border")
  assert.equal(snapshot.bottomBorderRow, "bottom-border")
  assert.equal(snapshot.horizontalSegments[2]?.[3], "horizontal")
  assert.equal(snapshot.bottomHorizontalSegments[3], "bottom-horizontal")
  assert.equal(snapshot.junctionTexts[2]?.[3], "junction")
  assert.equal(snapshot.bottomJunctionTexts[3], "bottom-junction")
  assert.equal(snapshot.verticalSegments[2]?.[3], "vertical")
})
