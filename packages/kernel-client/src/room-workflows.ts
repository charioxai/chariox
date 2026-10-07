import type { RoomWorkflowInventory, RoomWorkflowSummary, RoomWorkflowRunAction } from "./kernel-types.js"

export const ROOM_WORKFLOWS_MIN_PROTOCOL = 436

export function roomWorkflowRows(inventory: RoomWorkflowInventory) {
  return inventory.workflows.flatMap(workflow => workflow.endpoints.map(endpoint => ({
    key: JSON.stringify([workflow.workflow_id, endpoint.endpoint_id]), workflow, endpoint,
  })))
}

export function roomWorkflowRunTargets(workflow: RoomWorkflowSummary, action: RoomWorkflowRunAction): string[] {
  return workflow.runs.filter(run => run[`can_${action}`]).map(run => run.run_id)
}

export function controlRoomWorkflowRunsRequest(sessionId: string, workflowId: string, action: RoomWorkflowRunAction, runIds: readonly string[]) {
  return { ControlRoomWorkflowRuns: { session_id: sessionId, workflow_id: workflowId, action, run_ids: [...runIds] } }
}

export function roomWorkflowInventoryPayload(value: unknown, sessionId?: string): RoomWorkflowInventory | null {
  if (!value || typeof value !== "object") return null
  const inventory = value as RoomWorkflowInventory
  if (typeof inventory.session_id !== "string" || (sessionId && inventory.session_id !== sessionId)
    || typeof inventory.home_kernel_id !== "string" || typeof inventory.revision !== "string"
    || !Number.isInteger(inventory.workflow_count) || inventory.workflow_count < 0 || !Array.isArray(inventory.workflows)) return null
  for (const workflow of inventory.workflows) {
    if (!workflow || typeof workflow.workflow_id !== "string" || typeof workflow.label !== "string" || !Array.isArray(workflow.endpoints) || !Array.isArray(workflow.runs)) return null
    if (workflow.endpoints.some(endpoint => !endpoint || typeof endpoint.endpoint_id !== "string" || typeof endpoint.label !== "string" || typeof endpoint.can_start !== "boolean")) return null
    if (workflow.runs.some(run => !run || typeof run.run_id !== "string" || typeof run.can_pause !== "boolean" || typeof run.can_resume !== "boolean" || typeof run.can_stop !== "boolean")) return null
  }
  return inventory
}
