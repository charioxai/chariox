import assert from "node:assert/strict"
import test from "node:test"

import { createPromptSubmissionAgentStateController } from "./prompt-submission-agent-state-controller.js"

test("prompt submission agent state controller tracks and clears target agent", () => {
  const controller = createPromptSubmissionAgentStateController()

  assert.equal(controller.getSubmittingAgentId(), null)

  controller.setSubmittingAgentId("agent-1")
  assert.equal(controller.getSubmittingAgentId(), "agent-1")

  controller.clearSubmittingAgentId()
  assert.equal(controller.getSubmittingAgentId(), null)
})
