import assert from "node:assert/strict"
import test from "node:test"

import type {
  QueuedWorkflowLaunch,
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowRun,
} from "./cli-types.js"
import { createWorkflowRuntimeController } from "./workflow-runtime-controller.js"

test("workflow runtime controller invokes endpoints and refreshes returned sessions", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const nextSession = session("session-updated")
  const harness = createHarness({
    InvokeWorkflowEndpoint: {
      WorkflowRunInvoked: {
        workflow_run: workflowRun("run-1"),
        workflow: workflow(),
        endpoint: endpoint(),
        session: nextSession,
      },
    },
  }, refreshedSessions)

  const payload = await harness.controller.invokeWorkflowEndpoint("workflow-1", "endpoint-1", "ship it")

  assert.ok("workflow_run" in payload)
  assert.equal(payload.workflow_run.id, "run-1")
  assert.deepEqual(refreshedSessions, [nextSession])
  assert.deepEqual(harness.requests.at(-1), {
    InvokeWorkflowEndpoint: {
      session_id: "session-1",
      workflow_ref: "workflow-1",
      endpoint_ref: "endpoint-1",
      prompt: "ship it",
    },
  })
})

test("workflow runtime controller lists and removes queued launches", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const queueItem = queuedLaunch("queue-1")
  const nextSession = session("session-updated")
  const harness = createHarness({
    ListQueuedWorkflowLaunches: {
      QueuedWorkflowLaunchesListed: {
        queued_launches: [queueItem],
      },
    },
    RemoveQueuedWorkflowLaunch: {
      QueuedWorkflowLaunchRemoved: {
        queued_launch: queueItem,
        session: nextSession,
      },
    },
  }, refreshedSessions)

  assert.deepEqual(await harness.controller.listQueuedWorkflowLaunches(), [queueItem])
  const payload = await harness.controller.removeQueuedWorkflowLaunch("queue-1")

  assert.equal(payload.queued_launch.id, "queue-1")
  assert.deepEqual(refreshedSessions, [nextSession])
})

test("workflow runtime controller cancels runs through the kernel request path", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const nextSession = session("session-updated")
  const harness = createHarness({
    CancelWorkflowRun: {
      WorkflowRunCancelled: {
        workflow_run: workflowRun("run-1"),
        session: nextSession,
      },
    },
  }, refreshedSessions)

  const payload = await harness.controller.cancelWorkflowRun("run-1")

  assert.equal(payload.workflow_run.id, "run-1")
  assert.deepEqual(refreshedSessions, [nextSession])
  assert.deepEqual(harness.requests.at(-1), {
    CancelWorkflowRun: {
      session_id: "session-1",
      workflow_run_ref: "run-1",
    },
  })
})

function createHarness(
  responses: Record<string, Record<string, unknown>>,
  refreshedSessions: RuntimeSession[],
) {
  const requests: Record<string, unknown>[] = []
  const controller = createWorkflowRuntimeController({
    sessionId: () => "session-1",
    applyWorkflowSessionRefresh: (nextSession) => {
      refreshedSessions.push(nextSession)
    },
    sendRequest: async (request) => {
      requests.push(request)
      const variant = Object.keys(request)[0] ?? ""
      return responses[variant] ?? {}
    },
  })
  return { controller, requests }
}

function session(id: string): RuntimeSession {
  return { id } as RuntimeSession
}

function workflow(): WorkflowDefinition {
  return {
    id: "workflow-1",
    alias: "Workflow",
    nodes: [],
  }
}

function endpoint(): WorkflowEndpointDefinition {
  return {
    id: "endpoint-1",
    alias: null,
    entry_node_id: "node-1",
  }
}

function workflowRun(id: string): WorkflowRun {
  return {
    id,
    workflow_id: "workflow-1",
    status: "Running",
  } as unknown as WorkflowRun
}

function queuedLaunch(id: string): QueuedWorkflowLaunch {
  return {
    id,
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    source: "manual",
    queued_at_ms: 1,
  }
}
