import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession, WorkflowDefinition } from "./cli-types.js"
import { createWorkflowSettingsController } from "./workflow-settings-controller.js"

test("workflow settings controller updates workflow flush context", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const nextSession = { ...session("session-updated"), workflows: [{ ...workflow(), flush_agent_context_before_run: true }] }
  const harness = createHarness({
    ResolveWorkflow: { WorkflowResolved: { workflow: workflow() } },
    ApplyWorkflowDesignOp: { WorkflowDesignOpAccepted: { session: nextSession } },
  }, refreshedSessions)

  const payload = await harness.controller.setWorkflowFlushContext("workflow-1", true)

  assert.equal(payload.workflow.id, "workflow-1")
  assert.deepEqual(refreshedSessions, [nextSession])
  assert.deepEqual(harness.designOps, [{
    kind: "workflow_update",
    workflow_id: "workflow-1",
    patch: { flush_agent_context_before_run: true },
  }])
})

test("workflow settings controller updates run output schema refs", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const nextSession = { ...session("session-updated"), workflows: [{ ...workflow(), run_output_schema_ref: "schema-1" }] }
  const harness = createHarness({
    ResolveWorkflow: { WorkflowResolved: { workflow: workflow() } },
    ApplyWorkflowDesignOp: { WorkflowDesignOpAccepted: { session: nextSession } },
  }, refreshedSessions)

  const payload = await harness.controller.setWorkflowRunOutputSchema("workflow-1", "schema-1")

  assert.equal(payload.workflow.id, "workflow-1")
  assert.deepEqual(refreshedSessions, [nextSession])
  assert.deepEqual(harness.designOps, [{
    kind: "workflow_update",
    workflow_id: "workflow-1",
    patch: { run_output_schema_ref: "schema-1" },
  }])
})

function createHarness(
  responses: Record<string, Record<string, unknown>>,
  refreshedSessions: RuntimeSession[],
) {
  const requests: Record<string, unknown>[] = []
  const designOps: unknown[] = []
  const controller = createWorkflowSettingsController({
    sessionId: () => "session-1",
    applyWorkflowDesignOp: async (op) => {
      designOps.push(op)
      const payload = responses.ApplyWorkflowDesignOp?.WorkflowDesignOpAccepted as { session: RuntimeSession } | undefined
      if (!payload) throw new Error("missing ApplyWorkflowDesignOp response")
      return payload
    },
    applyWorkflowSessionRefresh: (nextSession) => {
      refreshedSessions.push(nextSession)
    },
    sendRequest: async (request) => {
      requests.push(request)
      const variant = Object.keys(request)[0] ?? ""
      return responses[variant] ?? {}
    },
  })
  return { controller, requests, designOps }
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
