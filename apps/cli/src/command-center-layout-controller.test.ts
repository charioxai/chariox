import assert from "node:assert/strict"
import test from "node:test"

import { createCommandCenterLayoutController } from "./command-center-layout-controller.js"

test("command center layout clamps visible rows to usable terminal space", () => {
  let terminalHeight = 24
  let promptHeight = 2
  const controller = createCommandCenterLayoutController({
    terminalHeight: () => terminalHeight,
    promptHeight: () => promptHeight,
  })

  assert.equal(controller.visibleRowCount(), 10)

  terminalHeight = 15
  assert.equal(controller.visibleRowCount(), 4)

  terminalHeight = 18
  promptHeight = 4
  assert.equal(controller.visibleRowCount(), 4)

  terminalHeight = 21
  promptHeight = 3
  assert.equal(controller.visibleRowCount(), 8)
})
