import assert from "node:assert/strict"
import test from "node:test"
import type { RoomWorkflowInventory } from "@chariox/kernel-client/kernel-types"
import { RoomWorkflowsPaneController } from "./room-workflows-pane-controller.js"
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
test("offline disables all mutation and authority switches preserve drafts", async () => {
 const h = harness(); h.pane.apply(inventory()); h.pane.draft("home draft"); h.pane.stale(); await h.pane.start(); await h.pane.control("pause"); assert.equal(h.requests.length, 0)
 const other = inventory(); other.home_kernel_id = "other"; h.pane.apply(other); assert.equal(h.pane.record!.draft, ""); h.pane.draft("other draft"); h.pane.apply(inventory()); assert.equal(h.pane.record!.draft, "home draft")
})
