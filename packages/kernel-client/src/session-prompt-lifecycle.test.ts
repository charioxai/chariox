import assert from "node:assert/strict"
import test from "node:test"

import {
  sessionActivePromptLifecycleRecords,
  sessionPromptLifecycleTransition,
} from "./session-prompt-lifecycle.js"
import {
  makeSession,
} from "./shell-executor.test-support.js"

test("session prompt lifecycle records normalize active prompt status", () => {
  assert.deepEqual(sessionActivePromptLifecycleRecords(makeSession({
    active_prompt: {
      id: "prompt-1",
      source_attachment_id: "attachment-1",
      target_agent_id: "agent-1",
      prompt: "hello",
      status: " Running ",
    },
  })), [{
    id: "prompt-1",
    source_attachment_id: "attachment-1",
    target_agent_id: "agent-1",
    prompt: "hello",
    status: "running",
    promptOrigin: null,
  }])
})

test("session prompt lifecycle transition settles normalized cancelling prompts", () => {
  assert.deepEqual(sessionPromptLifecycleTransition(
    makeSession({
      active_prompt: {
        id: "prompt-1",
        source_attachment_id: "attachment-1",
        target_agent_id: "agent-1",
        prompt: "hello",
        status: " Cancelling ",
      },
    }),
    makeSession(),
  ), {
    activePromptChanged: true,
    cancelledPromptSettled: true,
    settledAgentIds: ["agent-1"],
  })
})
