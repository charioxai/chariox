import assert from "node:assert/strict"
import test from "node:test"
import { createComponent, createSignal } from "solid-js"
import { testRender } from "@opentui/solid"
import { RoomWorkflowsPane } from "./room-workflows-pane.js"
import { RoomWorkflowsPaneController } from "./room-workflows-pane-controller.js"
for (const width of [36, 70]) test(`room workflows render an independent composer at ${width} columns`, async () => {
 const [revision, setRevision] = createSignal(0)
 const controller = new RoomWorkflowsPaneController({ clientId: "test", send: async () => { throw new Error("unexpected request") }, changed: () => setRevision(value => value + 1), dismissed: () => false, saveDismissed() {}, manage() {} })
 controller.apply({ session_id: "room", home_kernel_id: "home", revision: "1", workflow_count: 1, workflows: [{ workflow_id: "flow", workflow_revision: 1, label: "Review", state: "running", running_count: 2, paused_count: 0, queued_count: 0,
   endpoints: [{ endpoint_id: "entry", label: "Check", entry_node_id: "node", can_start: true }], runs: ["one", "two"].map(run_id => ({ run_id, endpoint_id: "entry", status: "Running", can_pause: true, can_resume: false, can_stop: true })) }] })
 controller.draft("independent draft")
 const harness = await testRender(() => createComponent(RoomWorkflowsPane, { controller, revision, width, onAgents() {}, onFocus() {} }), { width, height: 28, useThread: false })
 try {
   await harness.renderOnce()
   const frame = harness.captureCharFrame()
   assert.match(frame, /Review \/ Check/)
   assert.match(frame, /Pause current runs \(2\)/)
   assert.match(frame, /Stop current runs \(2\)/)
   assert.match(frame, /independent draft/)
   assert.doesNotMatch(frame, /trigger|agent-/i)
   controller.stale()
   await harness.renderOnce()
   assert.match(harness.captureCharFrame(), /Offline · stale/)
 } finally { harness.renderer.destroy() }
})

test("keyboard selection scrolls long endpoint lists while preserving the composer", async () => {
 const [revision, setRevision] = createSignal(0)
 const controller = new RoomWorkflowsPaneController({ clientId: "scroll", send: async () => { throw new Error("unexpected request") }, changed: () => setRevision(value => value + 1), dismissed: () => false, saveDismissed() {}, manage() {} })
 controller.apply({ session_id: "room", home_kernel_id: "home", revision: "1", workflow_count: 1, workflows: [{ workflow_id: "flow", workflow_revision: 1, label: "Review", state: "idle", running_count: 0, paused_count: 0, queued_count: 0, runs: [],
   endpoints: Array.from({ length: 12 }, (_, index) => ({ endpoint_id: `entry-${index}`, label: `Endpoint ${index + 1}`, entry_node_id: "node", can_start: true })) }] })
 controller.draft("retained draft")
 const harness = await testRender(() => createComponent(RoomWorkflowsPane, { controller, revision, width: 70, onAgents() {}, onFocus() {} }), { width: 70, height: 22, useThread: false })
 try {
   await harness.renderOnce()
   controller.select(11)
   await harness.renderOnce()
   assert.match(harness.captureCharFrame(), /› Review \/ Endpoint 12/)
   assert.match(harness.captureCharFrame(), /retained draft/)
 } finally { harness.renderer.destroy() }
})
