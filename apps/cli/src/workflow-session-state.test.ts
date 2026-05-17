import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeSession, WorkflowDefinition } from "./cli-types.js"
import { createWorkflowSessionStateController } from "./workflow-session-state.js"

test("workflow session state replaces workflow definitions on the current session", () => {
  const harness = createHarness()
  const workflows = [workflow("workflow-2", "Two")]

  harness.controller.replaceWorkflowDefinitions(workflows)

  assert.deepEqual(harness.session.workflows, workflows)
  assert.equal(harness.rebuilds, 0)
  assert.equal(harness.layouts, 0)
})

test("workflow session state upserts workflow definitions by id", () => {
  const harness = createHarness()

  harness.controller.upsertWorkflowDefinition(workflow("workflow-1", "One updated"))
  harness.controller.upsertWorkflowDefinition(workflow("workflow-2", "Two"))

  assert.deepEqual(
    harness.session.workflows?.map((entry) => [entry.id, entry.alias]),
    [
      ["workflow-1", "One updated"],
      ["workflow-2", "Two"],
    ],
  )
})

test("workflow session state applies refreshed sessions and repaints once", () => {
  const harness = createHarness()
  const session = runtimeSession([workflow("workflow-3", "Three")])

  harness.controller.applyWorkflowSessionRefresh(session)

  assert.equal(harness.session, session)
  assert.equal(harness.rebuilds, 1)
  assert.equal(harness.layouts, 1)
})

function createHarness() {
  const state = {
    session: runtimeSession([workflow("workflow-1", "One")]),
    rebuilds: 0,
    layouts: 0,
  }
  const controller = createWorkflowSessionStateController({
    sessionState: () => state.session,
    applySessionState: (session) => {
      state.session = session
    },
    rebuildTranscript: () => {
      state.rebuilds += 1
    },
    applyResponseLayout: () => {
      state.layouts += 1
    },
  })
  return {
    controller,
    get session() { return state.session },
    get rebuilds() { return state.rebuilds },
    get layouts() { return state.layouts },
  }
}

function runtimeSession(workflows: WorkflowDefinition[]): RuntimeSession {
  return {
    id: "session-1",
    agents: [],
    created_at: 1,
    updated_at: 1,
    status: "active",
    worktree: "/tmp/project",
    workflows,
  } as unknown as RuntimeSession
}

function workflow(id: string, alias: string): WorkflowDefinition {
  return {
    id,
    alias,
    nodes: [],
  }
}
