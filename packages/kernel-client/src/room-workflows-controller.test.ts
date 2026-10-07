import assert from "node:assert/strict"
import test from "node:test"
import type { RoomWorkflowInventory } from "./kernel-types.js"
import { RoomWorkflowsPaneController } from "./room-workflows-controller.js"
const inventory = (count = 1, endpoints = ["a"]): RoomWorkflowInventory => ({ session_id: "room", home_kernel_id: "home", revision: "1", workflow_count: count, workflows: count ? [{ workflow_id: "flow", workflow_revision: 1, label: "Flow", state: "running", running_count: 2, paused_count: 0, queued_count: 0, endpoints: endpoints.map(endpoint_id => ({ endpoint_id, label: endpoint_id, entry_node_id: "node", can_start: true })), runs: ["one", "two"].map(run_id => ({ run_id, status: "Running", endpoint_id: "a", can_pause: true, can_resume: false, can_stop: true })) }] : [] })
function harness() {
 const requests: unknown[] = []; const dismissed = new Map<string, boolean>(); let resolve!: (result: any) => void
 const pane = new RoomWorkflowsPaneController({ clientId: "client", send: async request => { requests.push(request); return await new Promise<any>(done => { resolve = done }) }, changed: () => {}, dismissed: key => dismissed.get(key) ?? false, saveDismissed: (key, closed) => { dismissed.set(key, closed) }, manage: () => {} })
 return { pane, requests, respond: (value: any) => resolve(value), dismissed }
}
test("authoritative blank workflow appears, dismissal persists, empty resets, one endpoint auto-selects", () => {
 const { pane } = harness(); pane.apply(inventory(1, [])); assert.equal(pane.visible, true); assert.equal(Boolean(pane.selected), false)
 pane.close(); pane.apply(inventory()); assert.equal(pane.visible, false); pane.open(); assert.equal(pane.selected?.endpoint.endpoint_id, "a")
 pane.close(); pane.apply(inventory(0)); assert.equal(pane.visible, false); pane.apply(inventory()); assert.equal(pane.visible, true)
})
test("draft, endpoint selection and focus stay separate from agent state; queued acknowledgement", async () => {
 const h = harness(); h.pane.apply(inventory(1, ["a", "b"])); assert.equal(h.pane.selected, null)
 h.pane.select(1); h.pane.draft("first"); const pending = h.pane.start(); h.pane.draft("next")
 h.respond({ WorkflowPromptEnqueued: { queued_prompt: { id: "queued" } } }); await pending
 assert.match(h.pane.record!.message, /Queued request queued/); assert.equal(h.pane.record!.draft, "next")
 assert.deepEqual(h.requests[0], { InvokeWorkflowEndpoint: { session_id: "room", workflow_ref: "flow", endpoint_ref: "b", prompt: "first", queue_ref: null } })
})
test("captured batch excludes later admissions and retains per-run outcomes", async () => {
 const h = harness(); h.pane.apply(inventory()); const pending = h.pane.control("stop")
 const newer = inventory(); newer.revision = "2"; newer.workflows[0]!.runs.push({ run_id: "later", endpoint_id: "a", status: "Running", can_pause: true, can_resume: false, can_stop: true })
 h.pane.apply(newer); h.respond({ RoomWorkflowRunsControlled: { inventory: inventory(), results: [{ run_id: "one", outcome: "applied" }, { run_id: "two", outcome: "unchanged" }] } }); await pending
 assert.deepEqual(h.requests[0], { ControlRoomWorkflowRuns: { session_id: "room", workflow_id: "flow", action: "stop", run_ids: ["one", "two"] } }); assert.equal(h.pane.record!.inventory.revision, "2"); assert.match(h.pane.record!.message, /two: unchanged/)
})
test("stale start refreshes before mutation and authority switches preserve new drafts", async () => {
 const h = harness(); h.pane.apply(inventory()); h.pane.draft("home draft"); h.pane.stale(); const refresh = h.pane.start(); h.respond({ SessionState: { room_workflows: inventory() } }); await new Promise(resolve => setTimeout(resolve, 0)); h.respond({ WorkflowRunInvoked: { workflow_run: { id: "refreshed" } } }); await refresh; assert.equal(h.requests.length, 2); h.pane.draft("home draft")
 const other = inventory(); other.home_kernel_id = "other"; h.pane.apply(other); assert.equal(h.pane.record!.draft, ""); h.pane.draft("other draft"); h.pane.apply(inventory()); assert.equal(h.pane.record!.draft, "home draft")
})

// MP-08 / MP-10: these controls previously returned without feedback.
test("stale control resyncs its captured authority and excludes later admissions", async () => {
 const h = harness(); h.pane.apply(inventory()); h.pane.stale()
 const pending = h.pane.control("pause")
 assert.deepEqual(h.requests[0], { GetSessionState: { session_id: "room" } })
 const newer = inventory(); newer.workflows[0]!.runs.push({ run_id: "later", endpoint_id: "a", status: "Running", can_pause: true, can_resume: false, can_stop: true })
 h.respond({ SessionState: { room_workflows: newer } }); await new Promise(resolve => setTimeout(resolve, 0))
 assert.deepEqual(h.requests[1], { ControlRoomWorkflowRuns: { session_id: "room", workflow_id: "flow", action: "pause", run_ids: ["one", "two"] } })
 h.respond({ RoomWorkflowRunsControlled: { inventory: newer, results: [{ run_id: "one", outcome: "applied" }] } }); await pending
 assert.equal(h.pane.record!.fresh, true)
})
test("busy and empty-target controls visibly report the guard", async () => {
 const h = harness(); h.pane.apply(inventory()); h.pane.draft("start")
 const pending = h.pane.start(); await h.pane.control("stop")
 assert.match(h.pane.record!.message, /busy/)
 h.respond({ WorkflowRunInvoked: { workflow_run: { id: "run" } } }); await pending
 await h.pane.control("resume"); assert.match(h.pane.record!.message, /no eligible runs/)
})
test("failed resync refuses visibly without controlling a different home", async () => {
 const h = harness(); h.pane.apply(inventory()); h.pane.stale(); const pending = h.pane.control("stop")
 const other = inventory(); other.home_kernel_id = "other"
 h.respond({ SessionState: { room_workflows: other } }); await pending
 assert.equal(h.requests.length, 1); assert.match(h.pane.record!.message, /authority/)
})
