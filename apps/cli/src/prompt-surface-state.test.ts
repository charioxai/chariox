import assert from "node:assert/strict"
import test from "node:test"

import type { WorkflowPromptState } from "./workflow-prompt-state.js"
import {
  createPromptPlaceholderSyncController,
  derivePromptAreaBackground,
  derivePromptInputMaxHeight,
  derivePromptPlaceholder,
} from "./prompt-surface-state.js"

test("derivePromptPlaceholder uses waiting-room placeholder while detached", () => {
  assert.equal(
    derivePromptPlaceholder({
      attached: false,
      workflowScreenActive: false,
      workflowPromptState: workflowPromptState({ enabled: false }),
      attachedPlaceholder: "Write your next prompt here",
      detachedPlaceholder: "Choose a session",
    }),
    "Choose a session",
  )
})

test("derivePromptPlaceholder reflects workflow endpoint eligibility while attached", () => {
  assert.equal(
    derivePromptPlaceholder({
      attached: true,
      workflowScreenActive: false,
      workflowPromptState: workflowPromptState({ enabled: false }),
      attachedPlaceholder: "Write your next prompt here",
      detachedPlaceholder: "Choose a session",
    }),
    "Write your next prompt here",
  )

  assert.equal(
    derivePromptPlaceholder({
      attached: true,
      workflowScreenActive: true,
      workflowPromptState: workflowPromptState({
        enabled: true,
        endpoint: { id: "endpoint-1", alias: "start", entry_node_id: "node-1" },
      }),
      attachedPlaceholder: "Write your next prompt here",
      detachedPlaceholder: "Choose a session",
    }),
    "Send prompt to endpoint endpoint-1 (start)",
  )
})

test("derivePromptAreaBackground follows attached and workflow modes", () => {
  assert.equal(
    derivePromptAreaBackground({
      attached: false,
      workflowScreenActive: false,
      attachedBackground: "panel",
      detachedBackground: "element",
      workflowBackground: "element",
    }),
    "element",
  )
  assert.equal(
    derivePromptAreaBackground({
      attached: true,
      workflowScreenActive: false,
      attachedBackground: "panel",
      detachedBackground: "element",
      workflowBackground: "element",
    }),
    "panel",
  )
  assert.equal(
    derivePromptAreaBackground({
      attached: true,
      workflowScreenActive: true,
      attachedBackground: "panel",
      detachedBackground: "element",
      workflowBackground: "element",
    }),
    "element",
  )
})

test("derivePromptInputMaxHeight keeps attached prompt bounded and detached prompt compact", () => {
  assert.equal(derivePromptInputMaxHeight({ attached: false, terminalHeight: 40 }), 6)
  assert.equal(derivePromptInputMaxHeight({ attached: true, terminalHeight: 20 }), 9)
  assert.equal(derivePromptInputMaxHeight({ attached: true, terminalHeight: 12 }), 6)
})

test("prompt placeholder sync controller updates the mounted input only", () => {
  const input = { placeholder: "old" }
  const controller = createPromptPlaceholderSyncController({
    getPromptInput: () => input,
    getPlaceholder: () => "new",
  })

  controller.sync()

  assert.equal(input.placeholder, "new")

  createPromptPlaceholderSyncController({
    getPromptInput: () => null,
    getPlaceholder: () => "ignored",
  }).sync()
})

function workflowPromptState(overrides: Partial<WorkflowPromptState>): WorkflowPromptState {
  return {
    workflow: null,
    workflowRun: null,
    selectedNodeId: null,
    endpoint: null,
    enabled: false,
    disabledReason: null,
    ...overrides,
  }
}
