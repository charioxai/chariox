import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession, WorkflowDefinition } from "./cli-types.js"
import { createWorkflowSettingsController } from "./workflow-settings-controller.js"

test("workflow settings controller updates launch policy", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const nextSession = session("session-updated")
  const harness = createHarness({
    SetWorkflowLaunchPolicy: {
      WorkflowLaunchPolicyUpdated: {
        session: nextSession,
      },
    },
  }, refreshedSessions)

  const payload = await harness.controller.setWorkflowLaunchPolicy("queue")

  assert.equal(payload.session, nextSession)
  assert.deepEqual(refreshedSessions, [nextSession])
  assert.deepEqual(harness.requests.at(-1), {
    SetWorkflowLaunchPolicy: {
      session_id: "session-1",
      policy: "queue",
    },
  })
})

test("workflow settings controller updates workflow flush context", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const nextSession = session("session-updated")
  const harness = createHarness({
    SetWorkflowFlushContext: {
      WorkflowFlushContextUpdated: {
        workflow: workflow(),
        session: nextSession,
      },
    },
  }, refreshedSessions)

  const payload = await harness.controller.setWorkflowFlushContext("workflow-1", true)

  assert.equal(payload.workflow.id, "workflow-1")
  assert.deepEqual(refreshedSessions, [nextSession])
  assert.deepEqual(harness.requests.at(-1), {
    SetWorkflowFlushContext: {
      session_id: "session-1",
      workflow_ref: "workflow-1",
      flush_agent_context_before_run: true,
    },
  })
})

test("workflow settings controller updates run output schema refs", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const nextSession = session("session-updated")
  const harness = createHarness({
    SetWorkflowRunOutputSchema: {
      WorkflowRunOutputSchemaUpdated: {
        workflow: workflow(),
        session: nextSession,
      },
    },
  }, refreshedSessions)

  const payload = await harness.controller.setWorkflowRunOutputSchema("workflow-1", "schema-1")

  assert.equal(payload.workflow.id, "workflow-1")
  assert.deepEqual(refreshedSessions, [nextSession])
  assert.deepEqual(harness.requests.at(-1), {
    SetWorkflowRunOutputSchema: {
      session_id: "session-1",
      workflow_ref: "workflow-1",
      run_output_schema_ref: "schema-1",
    },
  })
})

function createHarness(
  responses: Record<string, Record<string, unknown>>,
  refreshedSessions: RuntimeSession[],
) {
  const requests: Record<string, unknown>[] = []
  const controller = createWorkflowSettingsController({
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
