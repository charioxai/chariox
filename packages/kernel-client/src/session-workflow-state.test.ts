import assert from "node:assert/strict"
import test from "node:test"

import { sessionWorkflowSchedules } from "./session-workflow-state.js"

test("sessionWorkflowSchedules prefers canonical schedules over legacy watchdog alias", () => {
  assert.deepEqual(sessionWorkflowSchedules({
    workflow_schedules: [{ id: "schedule-1" }],
    workflow_watchdogs: [{ id: "watchdog-legacy" }],
  } as never), [{ id: "schedule-1" }])
})

test("sessionWorkflowSchedules supports legacy watchdog-only snapshots", () => {
  assert.deepEqual(sessionWorkflowSchedules({
    workflow_watchdogs: [{ id: "watchdog-legacy" }],
  } as never), [{ id: "watchdog-legacy" }])
})

test("sessionWorkflowSchedules defaults absent schedule collections to empty", () => {
  assert.deepEqual(sessionWorkflowSchedules({} as never), [])
})
