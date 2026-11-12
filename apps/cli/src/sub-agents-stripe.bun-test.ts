// MP-08 / MP-09 / MP-10 / MP-11 A6 (TUI): ONE sub-agents stripe in the room agent's stack, first, in the queued-row design.
import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import type { SessionProcess } from "@chariox/kernel-client/kernel-events"
import { queuedJobsForAgent } from "@chariox/kernel-client/queued-jobs"
import { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"
import { renderAgentInteractionStrips } from "./interaction-strip-renderer.js"

const held = (): SessionProcess => ({
  process_id: "process-1", owner: { kind: "agent", id: "room" }, pid: 1, command: "sh",
  resident_set_bytes: 1, cpu_millis: 0, disk_io_bytes: null, disk_written_bytes: null, deleted_open_bytes: 0, outlived: false, age_ms: 0, sampled_at_ms: 0,
  state: { kind: "held", reason: "memory", position: 1, need_bytes: 2 ** 30, free_bytes: 2 ** 29, command_line: "cargo build", since_ms: 1 },
})

async function frame(width: number, stripe: { text: string; needsYou: boolean } | null, processes: SessionProcess[] = [], agentId = "room", opened: string[] = []) {
  const harness = await createTestRenderer({ width, height: 8, useThread: false })
  const box = new BoxRenderable(harness.renderer, { width })
  harness.renderer.root.add(box)
  const store = createInteractionChoiceStoreController()
  renderAgentInteractionStrips({
    renderer: harness.renderer, primaryBox: box, auxiliaryBoxes: [], visibleAgents: [{ id: agentId }] as never,
    maxAgentsPerScreen: 1, focusedAgentId: agentId, activeInteractionForAgent: () => null,
    selectedChoiceIndex: store.selectedChoiceIndex, setSelectedChoiceIndex: store.setSelectedIndex,
    customReply: store.customReply, customEditing: store.isCustomEditing,
    queuedPromptStripItemsForAgent: () => [], selectedQueuedPromptIndexForAgent: () => -1, onQueuedPromptAction: () => {}, agentTriggerStripItemsForAgent: () => [],
    queuedJobsForAgent: id => queuedJobsForAgent(processes, id),
    subAgentStripeForAgent: id => id === "room" ? stripe && { ...stripe, open: () => { opened.push("open") } } : null,
  })
  await harness.renderOnce()
  const rows = harness.captureCharFrame().split("\n").map(row => row.trimEnd()).filter(Boolean)
  return { rows, box, harness }
}

test("AO-UI-01 the stripe reads plain counts after queued commands", async () => {
  const { rows, harness } = await frame(100, { text: "1 agent needs you · 5 running", needsYou: true }, [held()])
  assert.match(rows[1]!, /^\s*Agents 1 agent needs you · 5 running$/, rows.join("\n"))
  assert.match(rows[0]!, /Queued cargo build · waiting for memory/, "held commands lead the work stack")
  harness.renderer.destroy()
})

test("AO-UI-01 no stripe for another agent, none without sub-agents; the box hides when nothing else is in the stack", async () => {
  const none = await frame(100, null)
  assert.equal(none.rows.length, 0); assert.equal(none.box.visible, false); none.harness.renderer.destroy()
  const other = await frame(100, { text: "2 agents running", needsYou: false }, [], "someone-else")
  assert.equal(other.rows.length, 0); other.harness.renderer.destroy()
})

test("AO-UI-01 at 80 columns the stripe stays on its row", async () => {
  const { rows, harness } = await frame(80, { text: "1 agent needs you · 2 running · 1 waiting · 2 done", needsYou: true })
  assert.equal(rows.length, 1); assert.equal(rows[0]!.trim(), "Agents 1 agent needs you · 2 running · 1 waiting · 2 done")
  harness.renderer.destroy()
})

test("AO-UI-01 a click on the stripe opens the list card", async () => {
  const opened: string[] = []
  const { harness } = await frame(100, { text: "3 agents running", needsYou: false }, [], "room", opened)
  await harness.mockMouse.click(10, 0)
  await new Promise(done => setTimeout(done, 10))
  assert.deepEqual(opened, ["open"])
  harness.renderer.destroy()
})
