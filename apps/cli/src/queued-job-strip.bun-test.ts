import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import type { SessionProcess } from "@chariox/kernel-client/kernel-events"
import { queuedJobsForAgent } from "@chariox/kernel-client/queued-jobs"
import { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"
import { renderAgentInteractionStrips } from "./interaction-strip-renderer.js"

const held = (pid: number, position: number, command_line: string): SessionProcess => ({
  process_id: `process-${pid}`, owner: { kind: "agent", id: "agent-1" }, pid, command: "sh",
  resident_set_bytes: 1, cpu_millis: 0, age_ms: 0, sampled_at_ms: 0,
  state: { kind: "held", reason: "memory", position, need_bytes: 6 * 2 ** 30, free_bytes: 2 ** 30, command_line },
})

test("OF-04 a command waiting for room shows as a queued-job row in its agent's pane", async () => {
  const harness = await createTestRenderer({ width: 100, height: 12, useThread: false })
  const box = new BoxRenderable(harness.renderer, { width: 100 })
  harness.renderer.root.add(box)
  const store = createInteractionChoiceStoreController()
  let processes: SessionProcess[] = [held(42, 2, "cargo build --release")]
  const render = () => renderAgentInteractionStrips({
    renderer: harness.renderer, primaryBox: box, auxiliaryBoxes: [], visibleAgents: [{ id: "agent-1" }] as never,
    maxAgentsPerScreen: 1, focusedAgentId: "agent-1", activeInteractionForAgent: () => null,
    selectedChoiceIndex: store.selectedChoiceIndex, setSelectedChoiceIndex: store.setSelectedIndex,
    customReply: store.customReply, customEditing: store.isCustomEditing,
    queuedPromptStripItemsForAgent: () => [], selectedQueuedPromptIndexForAgent: () => -1, onQueuedPromptAction: () => {},
    queuedJobsForAgent: agentId => queuedJobsForAgent(processes, agentId),
  })
  render()
  await harness.renderOnce()
  const rows = harness.captureCharFrame().split("\n").map(row => row.trimEnd())
  assert.ok(rows.some(row => /Queued waiting for memory, 2nd in line · cargo build --release/.test(row)), rows.join("\n"))
  // A long command clips its own end, never the place in line.
  const long = "cargo build --release --workspace --all-targets --features native-display --target-dir /var/tmp/very/long/target/directory"
  processes = [held(42, 1, long)]
  render()
  await harness.renderOnce()
  const first = harness.captureCharFrame().split("\n").find(row => row.includes("Queued"))
  assert.ok(first?.includes("waiting for memory, 1st in line"), first)
  // The command starts: its row leaves the stack.
  processes = [{ ...processes[0]!, state: { kind: "running" } }]
  render()
  await harness.renderOnce()
  assert.ok(!harness.captureCharFrame().includes("Queued"))
  assert.equal(box.visible, false)
  harness.renderer.destroy()
})
