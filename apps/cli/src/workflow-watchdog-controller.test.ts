import assert from "node:assert/strict"
import test from "node:test"

import type {
  RuntimeSession,
  WorkflowDefinition,
  WorkflowEndpointDefinition,
  WorkflowWatchdogDefinition,
} from "./cli-types.js"
import { createWorkflowWatchdogController } from "./workflow-watchdog-controller.js"

test("workflow watchdog controller creates watchdogs and refreshes returned sessions", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const nextSession = session("session-updated")
  const harness = createHarness({
    CreateWorkflowWatchdog: {
      WorkflowWatchdogCreated: {
        watchdog: watchdog("watchdog-1"),
        workflow: workflow(),
        endpoint: endpoint(),
        session: nextSession,
      },
    },
  }, refreshedSessions)

  const payload = await harness.controller.createWorkflowWatchdog(
    "workflow-1",
    "endpoint-1",
    60,
    "check status",
    "queue",
    3,
  )

  assert.equal(payload.watchdog.id, "watchdog-1")
  assert.deepEqual(refreshedSessions, [nextSession])
  assert.deepEqual(harness.requests.at(-1), {
    CreateWorkflowWatchdog: {
      session_id: "session-1",
      workflow_ref: "workflow-1",
      endpoint_ref: "endpoint-1",
      interval_seconds: 60,
      invocation_prompt: "check status",
      policy: "queue",
      max_wakeups_configured: true,
      max_wakeups: 3,
    },
  })
})

test("workflow watchdog controller lists configured watchdogs", async () => {
  const item = watchdog("watchdog-1")
  const harness = createHarness({
    ListWorkflowWatchdogs: {
      WorkflowWatchdogsListed: {
        watchdogs: [item],
      },
    },
  })

  assert.deepEqual(await harness.controller.listWorkflowWatchdogs("workflow-1"), { watchdogs: [item] })
  assert.deepEqual(harness.requests.at(-1), {
    ListWorkflowWatchdogs: {
      session_id: "session-1",
      workflow_ref: "workflow-1",
    },
  })
})

test("workflow watchdog controller toggles watchdogs and refreshes returned sessions", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const nextSession = session("session-updated")
  const harness = createHarness({
    SetWorkflowWatchdogEnabled: {
      WorkflowWatchdogUpdated: {
        watchdog: watchdog("watchdog-1"),
        session: nextSession,
      },
    },
  }, refreshedSessions)

  const payload = await harness.controller.setWorkflowWatchdogEnabled("watchdog-1", false)

  assert.equal(payload.watchdog.id, "watchdog-1")
  assert.deepEqual(refreshedSessions, [nextSession])
  assert.deepEqual(harness.requests.at(-1), {
    SetWorkflowWatchdogEnabled: {
      session_id: "session-1",
      watchdog_ref: "watchdog-1",
      enabled: false,
    },
  })
})

function createHarness(
  responses: Record<string, Record<string, unknown>>,
  refreshedSessions: RuntimeSession[] = [],
) {
  const requests: Record<string, unknown>[] = []
  const controller = createWorkflowWatchdogController({
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

function watchdog(id: string): WorkflowWatchdogDefinition {
  return {
    id,
    workflow_id: "workflow-1",
    endpoint_id: "endpoint-1",
    enabled: true,
    interval_seconds: 60,
    invocation_prompt: "check status",
    policy: "queue",
    wakeups_executed: 0,
    next_run_at_ms: 1,
    created_at_ms: 1,
    updated_at_ms: 1,
  }
}
