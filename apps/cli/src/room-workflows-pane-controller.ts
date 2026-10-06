import { roomWorkflowRows, roomWorkflowRunTargets, controlRoomWorkflowRunsRequest } from "@chariox/kernel-client/room-workflows"
import { invokeWorkflowEndpointRequest } from "./ipc-requests.js"
import type { RoomWorkflowInventory, RoomWorkflowRunAction, RoomWorkflowRunsControlled } from "@chariox/kernel-client/kernel-types"
export type RoomWorkflowPaneRecord = { key: string; inventory: RoomWorkflowInventory; selectedKey: string | null; draft: string; dismissed: boolean; fresh: boolean; busy: boolean; message: string }
export type RoomWorkflowsPaneDeps = {
 clientId: string
 send<T>(request: unknown): Promise<T>
 changed(): void
 dismissed(key: string): boolean
 saveDismissed(key: string, closed: boolean): void
 manage(workflowId: string): void
}
export class RoomWorkflowsPaneController {
 private rowsInventory: RoomWorkflowInventory | null = null
 private rowCache: ReturnType<typeof roomWorkflowRows> = []
 private records = new Map<string, RoomWorkflowPaneRecord>()
 record: RoomWorkflowPaneRecord | null = null
 focused = false
 screenActive = true
 constructor(private readonly deps: RoomWorkflowsPaneDeps) {}
 get rows() {
  const inventory = this.record?.inventory ?? null
  if (inventory !== this.rowsInventory) { this.rowsInventory = inventory; this.rowCache = inventory ? roomWorkflowRows(inventory) : [] }
  return this.rowCache
 }
 get selected() { return this.rows.find(row => row.key === this.record?.selectedKey) ?? null }
 get visible() { return Boolean(this.record?.inventory.workflow_count && !this.record.dismissed) }
 get available() { return Boolean(this.record?.inventory.workflow_count) }
 get canStart() { return Boolean(this.record?.fresh && !this.record.busy && this.record.draft.trim() && this.selected?.endpoint.can_start) }
 apply(inventory: RoomWorkflowInventory, fresh = true): void {
  const key = JSON.stringify([this.deps.clientId, inventory.home_kernel_id, inventory.session_id])
  if (this.record?.key !== key) { this.screenActive = true; this.focused = false }
  else if (!this.record.inventory.workflow_count && inventory.workflow_count) this.screenActive = true
  const record = this.records.get(key) ?? { key, inventory, selectedKey: null, draft: "", dismissed: this.deps.dismissed(key), fresh, busy: false, message: "" }
  record.inventory = inventory; record.fresh = fresh
  this.records.set(key, record); this.record = record
  const rows = this.rows
  if (!rows.some(row => row.key === record.selectedKey)) record.selectedKey = rows.length === 1 ? rows[0]!.key : null
  if (!inventory.workflow_count) { record.dismissed = false; this.deps.saveDismissed(key, false); this.focused = false }
  this.deps.changed()
 }
 deactivate(): void { this.record = null; this.focused = false; this.deps.changed() }
 stale(): void { if (this.record) this.record.fresh = false; this.deps.changed() }
 close(): void { if (this.record) { this.record.dismissed = true; this.deps.saveDismissed(this.record.key, true) }; this.focused = false; this.deps.changed() }
 open(): void { if (this.record) { this.record.dismissed = false; this.deps.saveDismissed(this.record.key, false) }; this.focused = true; this.screenActive = true; this.deps.changed() }
 focus(value: boolean): void { this.focused = value; this.screenActive = value; this.deps.changed() }
 select(index: number): void { const row = this.rows[index]; if (row && this.record) this.record.selectedKey = row.key; this.deps.changed() }
 cycle(delta: number): void { const rows = this.rows; if (!rows.length) return; const index = rows.findIndex(row => row.key === this.record?.selectedKey); this.select((Math.max(index, 0) + delta + rows.length) % rows.length) }
 draft(text: string): void { if (this.record) this.record.draft = text; this.deps.changed() }
 targets(action: RoomWorkflowRunAction): string[] { return this.selected ? roomWorkflowRunTargets(this.selected.workflow, action) : [] }
 async start(): Promise<void> {
  const record = this.record; const row = this.selected
  if (!record || !row || !this.canStart) return
  const prompt = record.draft; record.busy = true; record.message = "Submitting…"; this.deps.changed()
  try {
   const response = await this.deps.send<{ WorkflowRunInvoked?: { workflow_run: { id: string } }; WorkflowPromptEnqueued?: { queued_prompt: { id: string } } }>(invokeWorkflowEndpointRequest(record.inventory.session_id, row.workflow.workflow_id, row.endpoint.endpoint_id, prompt))
   if (response.WorkflowRunInvoked?.workflow_run.id) record.message = `Started run ${response.WorkflowRunInvoked.workflow_run.id}`
   else if (response.WorkflowPromptEnqueued?.queued_prompt.id) record.message = `Queued request ${response.WorkflowPromptEnqueued.queued_prompt.id}`
   else throw new Error("Kernel did not confirm whether the request started or queued")
   if (record.draft === prompt) record.draft = ""
  } catch (error) { record.message = error instanceof Error ? error.message : "Start failed" }
  finally { record.busy = false; this.deps.changed() }
 }
 async control(action: RoomWorkflowRunAction): Promise<void> {
  const record = this.record; const row = this.selected; const ids = this.targets(action)
  if (!record?.fresh || record.busy || !row || !ids.length) return
  const request = controlRoomWorkflowRunsRequest(record.inventory.session_id, row.workflow.workflow_id, action, ids)
  record.busy = true; record.message = `${action} current runs (${ids.length})…`; this.deps.changed()
  try {
   const response = await this.deps.send<{ RoomWorkflowRunsControlled?: RoomWorkflowRunsControlled }>(request)
   const result = response.RoomWorkflowRunsControlled
   if (!result || result.inventory.session_id !== record.inventory.session_id || result.inventory.home_kernel_id !== record.inventory.home_kernel_id) throw new Error("Kernel did not confirm the captured run outcomes")
   record.message = result.results.map(result => `${result.run_id}: ${result.outcome}${result.error ? ` — ${result.error}` : ""}`).join("\n")
  } catch (error) { record.message = error instanceof Error ? error.message : "Control failed" }
  finally { record.busy = false; this.deps.changed() }
 }
 manage(): void { const id = this.selected?.workflow.workflow_id ?? this.record?.inventory.workflows[0]?.workflow_id; if (id) { this.focused = false; this.deps.manage(id); this.deps.changed() } }
}
