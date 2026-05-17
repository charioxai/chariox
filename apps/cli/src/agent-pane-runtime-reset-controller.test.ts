import assert from "node:assert/strict"
import test from "node:test"

import { createAgentPaneRuntimeResetController } from "./agent-pane-runtime-reset-controller.js"

test("agent pane runtime reset clears rendered panes and auxiliary ids", () => {
  const calls: string[] = []
  const controller = createAgentPaneRuntimeResetController({
    clearRenderedPanes: () => {
      calls.push("rendered-panes")
    },
    clearCurrentAuxiliaryAgentIds: () => {
      calls.push("auxiliary-ids")
    },
  })

  controller.reset()

  assert.deepEqual(calls, ["rendered-panes", "auxiliary-ids"])
})
