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

test("held notification retry adds retry_delivery only when requested", () => {
  assert.deepEqual(updateQueuedWorkflowPromptRequest("s", "q", { retryDelivery: true }), {
    UpdateQueuedWorkflowPrompt: { session_id: "s", queue_item_ref: "q", prompt: null, queue_ref: null, retry_delivery: true },
  })
  assert.deepEqual(updateQueuedWorkflowPromptRequest("s", "q", { prompt: "x" }), {
    UpdateQueuedWorkflowPrompt: { session_id: "s", queue_item_ref: "q", prompt: "x", queue_ref: null },
  })
})
