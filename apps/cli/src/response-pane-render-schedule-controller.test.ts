import assert from "node:assert/strict"
import test from "node:test"

import { createResponsePaneRenderScheduleController } from "./response-pane-render-schedule-controller.js"

test("response pane render schedule controller requests pane trees and root", () => {
  const requestedTrees: Array<string | undefined> = []
  let rootRequests = 0
  let layoutBox: string | undefined = "layout"
  let historyBox: string | undefined = "history"
  const controller = createResponsePaneRenderScheduleController({
    responseLayoutBox: () => layoutBox,
    historyLoadingBox: () => historyBox,
    requestTree: (renderable) => {
      requestedTrees.push(renderable)
    },
    requestRoot: () => {
      rootRequests += 1
    },
  })

  controller.scheduleRepaint()
  layoutBox = undefined
  historyBox = "next-history"
  controller.scheduleRepaint()

  assert.deepEqual(requestedTrees, ["layout", "history", undefined, "next-history"])
  assert.equal(rootRequests, 2)
})
