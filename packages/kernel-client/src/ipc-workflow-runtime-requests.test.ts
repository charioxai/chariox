import assert from "node:assert/strict"
import test from "node:test"

import { listWorkflowRunsRequest, updateQueuedWorkflowPromptRequest } from "./ipc-workflow-runtime-requests.js"

test("workflow run listing exposes bounded cursor pagination", () => {
  assert.deepEqual(
    listWorkflowRunsRequest("session-1", "workflow-1", {
      cursor: "v1:20:run-2",
      limit: 25,
    }),
    {
      ListWorkflowRuns: {
        session_id: "session-1",
        workflow_ref: "workflow-1",
        cursor: "v1:20:run-2",
        limit: 25,
      },
    },
  )
})

test("queued workflow prompt update carries only the prompt and queue", () => {
  assert.deepEqual(updateQueuedWorkflowPromptRequest("s", "q", { prompt: "x" }), {
    UpdateQueuedWorkflowPrompt: { session_id: "s", queue_item_ref: "q", prompt: "x", queue_ref: null },
  })
})
