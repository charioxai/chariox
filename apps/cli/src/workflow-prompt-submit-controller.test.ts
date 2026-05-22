import assert from "node:assert/strict"
import test from "node:test"

import type { WorkflowQueuedPrompt, WorkflowRun } from "./cli-types.js"
import type { SubmittedPromptUiSnapshot } from "./prompt-submission-ui-controller.js"
import { createWorkflowPromptSubmitController } from "./workflow-prompt-submit-controller.js"
import type { WorkflowPromptState } from "./workflow-prompt-state.js"

test("workflow prompt submit reports disabled prompt state", async () => {
  const harness = createHarness({
    workflowPromptState: {
      ...enabledWorkflowPromptState(),
      enabled: false,
      disabledReason: "no active workflow run",
    },
  })

  await harness.controller.submit("hello")

  assert.equal(harness.footerMessages().at(-1)?.message, "prompt disabled: no active workflow run")
  assert.equal(harness.beginCount(), 0)
  assert.deepEqual(harness.invocations(), [])
})

test("workflow prompt submit rejects pending attachments", async () => {
  const harness = createHarness({ pendingAttachmentCount: 1 })

  await harness.controller.submit("hello")

  assert.equal(harness.footerMessages().at(-1)?.message, "workflow endpoint prompts do not support attachments")
  assert.equal(harness.beginCount(), 0)
})

test("workflow prompt submit invokes endpoint runs and records prompt history", async () => {
  const harness = createHarness({
    invokeWorkflowEndpoint: async () => ({ workflow_run: workflowRun("run-1", "Running") }),
  })

  await harness.controller.submit("hello")

  assert.deepEqual(harness.invocations(), [{
    workflowId: "workflow-1",
    endpointId: "endpoint-1",
    prompt: "hello\n",
  }])
  assert.equal(harness.beginCount(), 1)
  assert.equal(harness.footerMessages().at(-1)?.message, "started workflow run run-1 [running]")
  assert.deepEqual(harness.recordedHistory(), [{ sessionId: "session-1", rawPrompt: "hello" }])
})

test("workflow prompt submit reports queued prompts", async () => {
  const harness = createHarness({
    invokeWorkflowEndpoint: async () => ({ queued_prompt: queuedPrompt("queue-1") }),
  })

  await harness.controller.submit("hello\n")

  assert.equal(harness.invocations().at(-1)?.prompt, "hello\n")
  assert.equal(harness.footerMessages().at(-1)?.message, "queued workflow prompt queue-1")
  assert.deepEqual(harness.recordedHistory(), [{ sessionId: "session-1", rawPrompt: "hello\n" }])
})

test("workflow prompt submit restores UI after invocation failure", async () => {
  const harness = createHarness({
    invokeWorkflowEndpoint: async () => {
      throw new Error("endpoint unavailable")
    },
  })

  await harness.controller.submit("hello")

  assert.equal(harness.restoredSnapshots().at(-1)?.rawPrompt, "hello")
  assert.equal(harness.footerMessages().at(-1)?.message, "endpoint unavailable")
  assert.deepEqual(harness.recordedHistory(), [])
})

function createHarness(options: {
  workflowPromptState?: WorkflowPromptState
  pendingAttachmentCount?: number
  invokeWorkflowEndpoint?: (
    workflowId: string,
    endpointId: string,
    prompt: string,
  ) => Promise<{ workflow_run: WorkflowRun } | { queued_prompt: WorkflowQueuedPrompt }>
} = {}) {
  const invocations: Array<{ workflowId: string; endpointId: string; prompt: string }> = []
  const footerMessages: Array<{ message: string; tone: "info" | "error" }> = []
  const recordedHistory: Array<{ sessionId: string; rawPrompt: string }> = []
  const restoredSnapshots: SubmittedPromptUiSnapshot[] = []
  let beginCount = 0

  const controller = createWorkflowPromptSubmitController({
    getWorkflowPromptState: () => options.workflowPromptState ?? enabledWorkflowPromptState(),
    getPendingAttachmentCount: () => options.pendingAttachmentCount ?? 0,
    beginSubmittedPromptUi: (rawPrompt) => {
      beginCount += 1
      return { rawPrompt, attachments: [], sessionId: "session-1" }
    },
    restoreFailedPromptUi: (snapshot) => {
      if (snapshot) {
        restoredSnapshots.push(snapshot)
      }
      return Boolean(snapshot)
    },
    invokeWorkflowEndpoint: async (workflowId, endpointId, prompt) => {
      invocations.push({ workflowId, endpointId, prompt })
      return options.invokeWorkflowEndpoint
        ? options.invokeWorkflowEndpoint(workflowId, endpointId, prompt)
        : { workflow_run: workflowRun("run-1", "Running") }
    },
    getSessionId: () => "session-1",
    recordPromptAreaHistoryEntry: (sessionId, rawPrompt) => {
      recordedHistory.push({ sessionId, rawPrompt })
    },
    flashFooter: (message, tone) => {
      footerMessages.push({ message, tone })
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
  })

  return {
    controller,
    invocations: () => invocations,
    footerMessages: () => footerMessages,
    recordedHistory: () => recordedHistory,
    restoredSnapshots: () => restoredSnapshots,
    beginCount: () => beginCount,
  }
}

function enabledWorkflowPromptState(): WorkflowPromptState {
  return {
    workflow: {
      id: "workflow-1",
      alias: null,
      nodes: [],
      edges: [],
      endpoints: [{ id: "endpoint-1", alias: null, entry_node_id: "node-1" }],
    },
    workflowRun: workflowRun("run-active", "Running"),
    selectedNodeId: "node-1",
    endpoint: { id: "endpoint-1", alias: null, entry_node_id: "node-1" },
    enabled: true,
    disabledReason: null,
  }
}

function workflowRun(id: string, status: string): WorkflowRun {
  return {
    id,
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    entry_node_id: "node-1",
    status,
    invocation_prompt: "hello",
    active_node_run_id: null,
    node_runs: [],
    messages: [],
    created_at_ms: 1,
    started_at_ms: 1,
    completed_at_ms: null,
  }
}

function queuedPrompt(id: string): WorkflowQueuedPrompt {
  return {
    id,
    queue_id: "default",
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    prompt: "hello",
    source: "manual",
    status: "queued",
    created_at_ms: 1,
    updated_at_ms: 1,
  }
}
