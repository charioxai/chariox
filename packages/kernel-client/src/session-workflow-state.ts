import type {
  RuntimeSession,
  WorkflowScheduleDefinition,
} from "./kernel-types.js"

export function sessionWorkflowSchedules(
  session: Pick<RuntimeSession, "workflow_schedules" | "workflow_watchdogs">,
): WorkflowScheduleDefinition[] {
  return Array.isArray(session.workflow_schedules)
    ? session.workflow_schedules
    : Array.isArray(session.workflow_watchdogs)
      ? session.workflow_watchdogs
      : []
}
