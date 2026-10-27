import assert from "node:assert/strict"
import test from "node:test"

import { createCliClosingStateController } from "./cli-closing-state-controller.js"

test("cli closing state controller tracks process shutdown state", () => {
  const controller = createCliClosingStateController()

  assert.equal(controller.isClosing(), false)

  controller.setClosing(true)
  assert.equal(controller.isClosing(), true)

  controller.setClosing(false)
  controller.markClosing()
  assert.equal(controller.isClosing(), true)
})
