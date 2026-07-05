import assert from "node:assert/strict"
import test from "node:test"

import { createPromptChromeProjectionController } from "./prompt-chrome-projection-controller.js"
import type { WorkflowPromptState } from "@arroba/kernel-client/workflow-prompt-state"

test("prompt chrome projection derives status and footer from prompt state", () => {
  const controller = createPromptChromeProjectionController({
    daemonDisconnected: () => false,
    working: () => false,
    hasActivePrompt: () => true,
    submitting: () => false,
    queueDepth: () => 2,
    fatalError: () => null,
    activePromptId: () => "prompt-1",
    statusLine: () => "connected",
    isAttached: () => true,
    workflowScreenActive: () => false,
    workflowPromptState: workflowPromptState,
    attachedPlaceholder: "attached prompt",
    detachedPlaceholder: "detached prompt",
    attachedBackground: () => "attached",
    detachedBackground: () => "detached",
    workflowBackground: () => "workflow",
  })

  assert.equal(controller.sessionStatusMode(), "working")
  assert.equal(controller.footerHint(), "Processing prompt-1; 2 queued.")
})

test("prompt chrome projection derives placeholder and tracks prompt background theme", () => {
  let themeRevisionReads = 0
  const controller = createPromptChromeProjectionController({
    daemonDisconnected: () => false,
    working: () => false,
    hasActivePrompt: () => false,
    submitting: () => false,
    queueDepth: () => 0,
    fatalError: () => null,
    activePromptId: () => null,
    statusLine: () => "connected",
    isAttached: () => false,
    workflowScreenActive: () => false,
    workflowPromptState: workflowPromptState,
    attachedPlaceholder: "attached prompt",
    detachedPlaceholder: "detached prompt",
    trackThemeRevision: () => {
      themeRevisionReads += 1
    },
    attachedBackground: () => "attached",
    detachedBackground: () => "detached",
    workflowBackground: () => "workflow",
  })

  assert.equal(controller.promptPlaceholder(), "detached prompt")
  assert.equal(controller.promptAreaBackground(), "detached")
  assert.equal(themeRevisionReads, 1)
})

function workflowPromptState(): WorkflowPromptState {
  return {
    workflow: null,
    workflowRun: null,
    selectedNodeId: null,
    selectedAgent: null,
    enabled: false,
    disabledReason: null,
  }
}
