import assert from "node:assert/strict"
import test from "node:test"

import { createTranscriptTurnStateController } from "./transcript-turn-state-controller.js"

test("transcript turn state controller tracks current and next turn ids", () => {
  const controller = createTranscriptTurnStateController({
    initialCurrentTurnId: 3,
    initialNextTurnId: 4,
  })

  assert.equal(controller.getCurrentTurnId(), 3)
  assert.equal(controller.getNextTurnId(), 4)

  controller.setCurrentTurnId(null)
  controller.setNextTurnId(8)

  assert.equal(controller.getCurrentTurnId(), null)
  assert.equal(controller.getNextTurnId(), 8)
})
