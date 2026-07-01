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

test("workflow schedule controller creates schedules and refreshes returned sessions", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const nextSession = session("session-updated")
  const harness = createHarness({
    CreateWorkflowSchedule: {
      WorkflowScheduleCreated: {
        schedule: watchdog("schedule-1"),
        workflow: workflow(),
        endpoint: endpoint(),
        session: nextSession,
      },
    },
  }, refreshedSessions)

  const trigger = { kind: "cron" as const, expression: "15 30 14 * * *", timezone: "UTC" }
  const payload = await harness.controller.createWorkflowSchedule(
    "workflow-1",
    "endpoint-1",
    trigger,
    "check status",
    "queue",
    3,
    "queue-1",
  )

  assert.equal(payload.schedule.id, "schedule-1")
  assert.deepEqual(refreshedSessions, [nextSession])
  assert.deepEqual(harness.requests.at(-1), {
    CreateWorkflowSchedule: {
      session_id: "session-1",
      workflow_ref: "workflow-1",
      endpoint_ref: "endpoint-1",
      queue_ref: "queue-1",
      trigger,
      invocation_prompt: "check status",
      overlap_policy: "queue",
      max_runs_configured: true,
      max_runs: 3,
    },
  })
})

test("workflow schedule controller lists toggles and removes schedules", async () => {
  const refreshedSessions: RuntimeSession[] = []
  const nextSession = session("session-updated")
  const item = watchdog("schedule-1")
  const harness = createHarness({
    ListWorkflowSchedules: {
      WorkflowSchedulesListed: {
        schedules: [item],
      },
    },
    SetWorkflowScheduleEnabled: {
      WorkflowScheduleUpdated: {
        schedule: item,
        session: nextSession,
      },
    },
    RemoveWorkflowSchedule: {
      WorkflowScheduleRemoved: {
        schedule: item,
        session: nextSession,
      },
    },
  }, refreshedSessions)

  assert.deepEqual(await harness.controller.listWorkflowSchedules("workflow-1"), { schedules: [item] })
  const updated = await harness.controller.setWorkflowScheduleEnabled("schedule-1", false)
  const removed = await harness.controller.removeWorkflowSchedule("schedule-1")

  assert.equal(updated.schedule.id, "schedule-1")
  assert.equal(removed.schedule.id, "schedule-1")
  assert.deepEqual(refreshedSessions, [nextSession, nextSession])
  assert.deepEqual(harness.requests, [
    {
      ListWorkflowSchedules: {
        session_id: "session-1",
        workflow_ref: "workflow-1",
      },
    },
    {
      SetWorkflowScheduleEnabled: {
        session_id: "session-1",
        schedule_ref: "schedule-1",
        enabled: false,
      },
    },
    {
      RemoveWorkflowSchedule: {
        session_id: "session-1",
        schedule_ref: "schedule-1",
      },
    },
  ])
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
    trigger: { kind: "interval", every_seconds: 60 },
    interval_seconds: 60,
    invocation_prompt: "check status",
    overlap_policy: "queue",
    policy: "queue",
    runs_started: 0,
    wakeups_executed: 0,
    next_run_at_ms: 1,
    created_at_ms: 1,
    updated_at_ms: 1,
  }
}
