import assert from "node:assert/strict"
import test from "node:test"

import {
  createWorkflowNodeInstructionsEditorController,
  type WorkflowNodeInstructionsEditor,
} from "./workflow-node-instructions-editor-controller.js"

test("workflow node instructions editor opens on the workflow screen and focuses the editor", () => {
  const harness = createHarness({ workflowScreenShowing: false })
  harness.controller.setInputRef({ focus: () => harness.calls().push("focus:editor") })

  harness.controller.open("workflow-1", "node-1", "draft")

  assert.deepEqual(harness.editor(), {
    workflowId: "workflow-1",
    nodeId: "node-1",
    draft: "draft",
  })
  assert.deepEqual(harness.calls(), ["editor:workflow-1:node-1", "screen:workflow", "rebuild", "timer:0"])
  harness.fire()
  assert.deepEqual(harness.calls(), ["editor:workflow-1:node-1", "screen:workflow", "rebuild", "timer:0", "focus:editor"])
})

test("workflow node instructions editor closes, rebuilds, and returns prompt focus", () => {
  const harness = createHarness({
    editor: { workflowId: "workflow-1", nodeId: "node-1", draft: "draft" },
    workflowScreenShowing: true,
  })

  harness.controller.close()

  assert.equal(harness.editor(), null)
  assert.deepEqual(harness.calls(), ["editor:null", "rebuild", "focus:prompt"])
})

test("workflow node instructions editor updates draft and exposes command context", () => {
  const harness = createHarness({
    editor: { workflowId: "workflow-1", nodeId: "node-1", draft: "old" },
  })

  harness.controller.updateDraft("new")

  assert.equal(harness.controller.draft(), "new")
  assert.deepEqual(harness.controller.context(), {
    workflowId: "workflow-1",
    nodeId: "node-1",
  })
})

test("workflow node instructions editor clear drops editor state without UI side effects", () => {
  const harness = createHarness({
    editor: { workflowId: "workflow-1", nodeId: "node-1", draft: "old" },
    workflowScreenShowing: true,
  })

  harness.controller.clear()

  assert.equal(harness.editor(), null)
  assert.deepEqual(harness.calls(), ["editor:null"])
})

function createHarness(options: {
  editor?: WorkflowNodeInstructionsEditor | null
  workflowScreenShowing?: boolean
} = {}) {
  const calls: string[] = []
  let editor = options.editor ?? null
  let scheduled: (() => void) | null = null
  const controller = createWorkflowNodeInstructionsEditorController<string>({
    getEditor: () => editor,
    setEditor: (nextEditor) => {
      editor = nextEditor
      calls.push(nextEditor ? `editor:${nextEditor.workflowId}:${nextEditor.nodeId}` : "editor:null")
    },
    workflowScreenShowing: () => options.workflowScreenShowing ?? true,
    setWorkspaceScreenMode: (mode) => {
      calls.push(`screen:${mode}`)
    },
    rebuildTranscript: () => {
      calls.push("rebuild")
    },
    scheduleTimer: (callback, delayMs) => {
      scheduled = callback
      calls.push(`timer:${delayMs}`)
      return "timer-1"
    },
    focusPromptInput: () => {
      calls.push("focus:prompt")
    },
  })

  return {
    controller,
    calls: () => calls,
    editor: () => editor,
    fire: () => scheduled?.(),
  }
}
