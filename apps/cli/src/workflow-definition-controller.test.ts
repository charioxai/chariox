import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession, WorkflowDefinition } from "./cli-types.js"
import { createWorkflowDefinitionController } from "./workflow-definition-controller.js"

test("workflow definition controller creates workflows and selects the new canvas", async () => {
  const harness = createHarness({
    CreateWorkflow: {
      WorkflowCreated: {
        workflow: workflow("workflow-2", "New"),
        session: session("session-updated"),
      },
    },
  })

  const payload = await harness.controller.createWorkflow("New")

  assert.equal(payload.workflow.id, "workflow-2")
  assert.equal(harness.selectedWorkflowId, "workflow-2")
  assert.equal(harness.selectedNodeId, null)
  assert.equal(harness.rebuilds, 1)
  assert.equal(harness.layouts, 1)
  assert.deepEqual(harness.requests.at(-1), {
    CreateWorkflow: {
      session_id: "session-1",
      alias: "New",
    },
  })
})

test("workflow definition controller lists and resolves workflows", async () => {
  const item = workflow("workflow-1", "Main")
  const harness = createHarness({
    ListWorkflows: {
      WorkflowsListed: {
        workflows: [item],
      },
    },
    ResolveWorkflow: {
      WorkflowResolved: {
        workflow: item,
      },
    },
  })

  assert.deepEqual(await harness.controller.listWorkflows(), [item])
  assert.deepEqual(await harness.controller.resolveWorkflow("Main"), { workflow: item })
})

test("workflow definition controller aliases workflows and repaints", async () => {
  const nextSession = session("session-updated")
  const harness = createHarness({
    AliasWorkflow: {
      WorkflowAliased: {
        workflow: workflow("workflow-1", "Main"),
        session: nextSession,
      },
    },
  })

  const payload = await harness.controller.assignWorkflowAlias("workflow-1", "Main")

  assert.equal(payload.id, "workflow-1")
  assert.equal(harness.appliedSession, nextSession)
  assert.equal(harness.rebuilds, 1)
  assert.equal(harness.layouts, 1)
  assert.deepEqual(harness.requests.at(-1), {
    AliasWorkflow: {
      session_id: "session-1",
      workflow_ref: "workflow-1",
      alias: "Main",
    },
  })
})

function createHarness(responses: Record<string, Record<string, unknown>>) {
  const state: {
    selectedWorkflowId: string | null
    selectedNodeId: string | null
    appliedSession: RuntimeSession | null
    rebuilds: number
    layouts: number
  } = {
    selectedWorkflowId: null,
    selectedNodeId: "node-1",
    appliedSession: null,
    rebuilds: 0,
    layouts: 0,
  }
  const requests: Record<string, unknown>[] = []
  const controller = createWorkflowDefinitionController({
    sessionId: () => "session-1",
    applySessionState: (nextSession) => {
      state.appliedSession = nextSession
    },
    setSelectedWorkflowId: (workflowId) => {
      state.selectedWorkflowId = workflowId
    },
    setSelectedWorkflowNodeId: (nodeId) => {
      state.selectedNodeId = nodeId
    },
    rebuildTranscript: () => {
      state.rebuilds += 1
    },
    applyResponseLayout: () => {
      state.layouts += 1
    },
    sendRequest: async (request) => {
      requests.push(request)
      const variant = Object.keys(request)[0] ?? ""
      return responses[variant] ?? {}
    },
  })
  return {
    controller,
    requests,
    get selectedWorkflowId() { return state.selectedWorkflowId },
    get selectedNodeId() { return state.selectedNodeId },
    get appliedSession() { return state.appliedSession },
    get rebuilds() { return state.rebuilds },
    get layouts() { return state.layouts },
  }
}

function session(id: string): RuntimeSession {
  return { id } as RuntimeSession
}

function workflow(id: string, alias: string): WorkflowDefinition {
  return {
    id,
    alias,
    nodes: [],
  }
}
