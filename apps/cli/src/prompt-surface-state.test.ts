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

test("derivePromptPlaceholder reflects workflow agent eligibility while attached", () => {
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
        selectedAgent: {
          id: "agent-1",
          agent_ref: "agent-1",
          session_id: "session-1",
          alias: "Builder",
          provider: "codex",
          model: "gpt-5",
          worktree_id: null,
          state: "Idle",
          is_processing: false,
          grid_row: 0,
          grid_col: 0,
          grid_row_span: 1,
          grid_col_span: 1,
          created_at_ms: 1,
          last_activity_at_ms: 1,
        },
      }),
      attachedPlaceholder: "Write your next prompt here",
      detachedPlaceholder: "Choose a session",
    }),
    "Prompt workflow agent agent-1 (Builder)",
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
    selectedAgent: null,
    enabled: false,
    disabledReason: null,
    ...overrides,
  }
}
