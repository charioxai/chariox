import { roomWorkflowRows, roomWorkflowRunTargets, controlRoomWorkflowRunsRequest, roomWorkflowInventoryPayload } from "./room-workflows.js"
import { invokeWorkflowEndpointRequest } from "./ipc-workflow-requests.js"
import type { RoomWorkflowInventory, RoomWorkflowRunAction, RoomWorkflowRunsControlled } from "./kernel-types.js"
export type RoomWorkflowPaneRecord = { key: string; inventory: RoomWorkflowInventory; selectedKey: string | null; draft: string; dismissed: boolean; fresh: boolean; busy: boolean; message: string; lastGuard?: string }
export type RoomWorkflowsPaneDeps = {
 clientId: string
 send<T>(request: unknown): Promise<T>
 changed(): void
 dismissed(key: string): boolean
 saveDismissed(key: string, closed: boolean): void
 trace?(guard: string): void
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
 private report(record: RoomWorkflowPaneRecord, guard: string, message: string): void {
  record.lastGuard = guard; record.message = message; this.deps.trace?.(guard); this.deps.changed()
 }
 private async resync(record: RoomWorkflowPaneRecord): Promise<void> {
  this.report(record, "stale", "Refreshing workflow state before acting…")
  const response = await this.deps.send<{ SessionState?: { room_workflows: RoomWorkflowInventory } }>({ GetSessionState: { session_id: record.inventory.session_id } })
  const inventory = roomWorkflowInventoryPayload(response.SessionState?.room_workflows, record.inventory.session_id)
  if (!inventory || inventory.home_kernel_id !== record.inventory.home_kernel_id || this.record !== record) throw new Error("Workflow authority changed during refresh; select the Room and try again")
  this.apply(inventory, true)
 }
 async start(): Promise<void> {
  const record = this.record; const row = this.selected
  if (!record) return
  if (record.busy) { this.report(record, "busy", "Start refused: another workflow request is busy; try again when it finishes"); return }
  if (!row || !record.draft.trim()) { this.report(record, "no-selection-or-prompt", "Select an entry point and enter a prompt"); return }
  const prompt = record.draft; record.lastGuard = "ready"; record.busy = true; record.message = "Submitting…"; this.deps.changed()
  try {
   if (!record.fresh) await this.resync(record)
   const current = record.inventory.workflows.find(workflow => workflow.workflow_id === row.workflow.workflow_id)?.endpoints.find(endpoint => endpoint.endpoint_id === row.endpoint.endpoint_id)
   if (!current?.can_start) throw new Error(current?.start_disabled_reason ?? "The selected entry point cannot start")
   const response = await this.deps.send<{ WorkflowRunInvoked?: { workflow_run: { id: string } }; WorkflowPromptEnqueued?: { queued_prompt: { id: string } } }>(invokeWorkflowEndpointRequest(record.inventory.session_id, row.workflow.workflow_id, row.endpoint.endpoint_id, prompt))
   if (response.WorkflowRunInvoked?.workflow_run.id) { if (record.lastGuard !== "busy") record.message = `Started run ${response.WorkflowRunInvoked.workflow_run.id}` }
   else if (response.WorkflowPromptEnqueued?.queued_prompt.id) { if (record.lastGuard !== "busy") record.message = `Queued request ${response.WorkflowPromptEnqueued.queued_prompt.id}` }
   else throw new Error("Kernel did not confirm whether the request started or queued")
   if (record.draft === prompt) record.draft = ""
  } catch (error) { record.message = error instanceof Error ? error.message : "Start failed" }
  finally { record.busy = false; this.deps.changed() }
 }
 async control(action: RoomWorkflowRunAction): Promise<void> {
  const record = this.record; const row = this.selected; let ids = this.targets(action)
  const capturedRuns = new Set(row?.workflow.runs.map(run => run.run_id) ?? [])
  const needsRefresh = record?.fresh === false
  if (!record) return
  if (record.busy) { this.report(record, "busy", `${action} refused: another workflow request is busy; try again when it finishes`); return }
  if (!row || (!needsRefresh && !ids.length)) { this.report(record, !row ? "no-selection" : "no-targets", `${action} refused: ${!row ? "select a workflow" : "no eligible runs"}`); return }
  // MP-08 / MP-10: capture targets before refresh; never include later admissions.
  record.lastGuard = "ready"; record.busy = true; record.message = `${action} current runs (${ids.length})…`; this.deps.changed()
  try {
   if (needsRefresh) {
    await this.resync(record)
    const workflow = record.inventory.workflows.find(workflow => workflow.workflow_id === row.workflow.workflow_id)
    ids = workflow ? roomWorkflowRunTargets(workflow, action).filter(id => capturedRuns.has(id)) : []
    if (!ids.length) { this.report(record, "no-targets", `${action} refused: no eligible captured runs; review the refreshed workflow and try again`); return }
   }
   const request = controlRoomWorkflowRunsRequest(record.inventory.session_id, row.workflow.workflow_id, action, ids)
   record.lastGuard ??= "ready"; this.deps.trace?.("sending")
   const response = await this.deps.send<{ RoomWorkflowRunsControlled?: RoomWorkflowRunsControlled }>(request)
   const result = response.RoomWorkflowRunsControlled
   if (!result || result.inventory.session_id !== record.inventory.session_id || result.inventory.home_kernel_id !== record.inventory.home_kernel_id) throw new Error("Kernel did not confirm the captured run outcomes")
   if (record.lastGuard !== "busy") record.message = result.results.map(result => `${result.run_id}: ${result.outcome}${result.error ? ` — ${result.error}` : ""}`).join("\n")
  } catch (error) { this.report(record, "send-or-refresh-failed", error instanceof Error ? error.message : "Control failed") }
  finally { record.busy = false; this.deps.changed() }
 }
 manage(): void { const id = this.selected?.workflow.workflow_id ?? this.record?.inventory.workflows[0]?.workflow_id; if (id) { this.focused = false; this.deps.manage(id); this.deps.changed() } }
}
