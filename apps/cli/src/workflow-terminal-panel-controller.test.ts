import assert from "node:assert/strict"
import test from "node:test"

import { createWorkflowTerminalPanelController } from "./workflow-terminal-panel-controller.js"

test("workflow terminal panel opens on the workflow screen", () => {
  const harness = createHarness({ workflowScreenShowing: false })

  harness.controller.open("workflow-1")

  assert.deepEqual(harness.calls, [
    "clear:node-instructions",
    "inspector:logs",
    "selected:workflow-1",
    "screen:workflow",
    "rebuild",
  ])
})

test("workflow terminal panel does not switch screen when workflow screen is already visible", () => {
  const harness = createHarness({ workflowScreenShowing: true })

  harness.controller.open("workflow-2")

  assert.deepEqual(harness.calls, [
    "clear:node-instructions",
    "inspector:logs",
    "selected:workflow-2",
    "rebuild",
  ])
})

function createHarness(options: { workflowScreenShowing: boolean }) {
  const calls: string[] = []
  const controller = createWorkflowTerminalPanelController({
    clearNodeInstructionsEditor: () => {
      calls.push("clear:node-instructions")
    },
    setWorkflowInspectorMode: (mode) => {
      calls.push(`inspector:${mode}`)
    },
    setSelectedWorkflowId: (workflowId) => {
      calls.push(`selected:${workflowId}`)
    },
    workflowScreenShowing: () => options.workflowScreenShowing,
    setWorkspaceScreenMode: (mode) => {
      calls.push(`screen:${mode}`)
    },
    rebuildTranscript: () => {
      calls.push("rebuild")
    },
  })

  return { calls, controller }
}
