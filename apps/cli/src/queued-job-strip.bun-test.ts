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
  resident_set_bytes: 1, cpu_millis: 0, disk_io_bytes: null, disk_written_bytes: null, deleted_open_bytes: 0, outlived: false, age_ms: 0, sampled_at_ms: 0,
  state: { kind: "held", reason: "memory", position, need_bytes: 6 * 2 ** 30, free_bytes: 2 ** 30, command_line, since_ms: 1 },
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
    queuedPromptStripItemsForAgent: () => [], selectedQueuedPromptIndexForAgent: () => -1, onQueuedPromptAction: () => {}, agentTriggerStripItemsForAgent: () => [],
    queuedJobsForAgent: agentId => queuedJobsForAgent(processes, agentId),
  })
  render()
  await harness.renderOnce()
  const rows = harness.captureCharFrame().split("\n").map(row => row.trimEnd())
  assert.ok(rows.some(row => /Queued cargo build --release · waiting for memory, 2nd in line/.test(row)), rows.join("\n"))
  // A long command wraps without losing its wait details.
  const long = "cargo build --release --workspace --all-targets --features native-display --target-dir /var/tmp/very/long/target/directory"
  processes = [held(42, 1, long)]
  render()
  await harness.renderOnce()
  assert.match(harness.captureCharFrame().replaceAll("\n", " "), /waiting for memory, needs 6 GB/)
  // The command starts: its row leaves the stack.
  processes = [{ ...processes[0]!, state: { kind: "running" } }]
  render()
  await harness.renderOnce()
  assert.ok(!harness.captureCharFrame().includes("Queued"))
  assert.equal(box.visible, false)
  harness.renderer.destroy()
})


test("OF-22 the queued job stripe shows its rate and invokes VM and Stop actions", async () => {
  const harness = await createTestRenderer({ width: 140, height: 12, useThread: false })
  const box = new BoxRenderable(harness.renderer, { width: 140 }); harness.renderer.root.add(box)
  const store = createInteractionChoiceStoreController(), actions: string[] = []
  const render = (remote: boolean) => renderAgentInteractionStrips({
    renderer: harness.renderer, primaryBox: box, auxiliaryBoxes: [], visibleAgents: [{ id: "agent-1" }] as never,
    maxAgentsPerScreen: 1, focusedAgentId: "agent-1", activeInteractionForAgent: () => null,
    selectedChoiceIndex: store.selectedChoiceIndex, setSelectedChoiceIndex: store.setSelectedIndex,
    customReply: store.customReply, customEditing: store.isCustomEditing,
    queuedPromptStripItemsForAgent: () => [], selectedQueuedPromptIndexForAgent: () => -1, onQueuedPromptAction: () => {}, agentTriggerStripItemsForAgent: () => [],
    queuedJobsForAgent: () => [{ processId: "j", command: "cargo build", wait: "waiting for memory, 2nd in line", detail: null, remoteAvailable: remote, rate: "≈ $0.04" }],
    onQueuedJobAction: (job, action) => actions.push(`${job.processId}:${action}`),
  })
  try {
    render(true); await harness.renderOnce()
    let frame = harness.captureCharFrame()
    assert.match(frame, /Queued cargo build · waiting for memory, 2nd in line/)
    assert.match(frame, /≈ \$0.04/)
    const click = async (label: string) => {
      const lines = harness.captureCharFrame().split("\n"), y = lines.findIndex(line => line.includes(label)), x = lines[y]!.indexOf(label)
      assert.ok(x >= 0 && y >= 0, label)
      await harness.mockMouse.click(x + 2, y)
      await new Promise(done => setTimeout(done, 10))
    }
    await click("[Run on a VM]"); await click("[Stop]")
    assert.deepEqual(actions, ["j:remote", "j:stop"])
    render(false); await harness.renderOnce(); frame = harness.captureCharFrame()
    assert.ok(!frame.includes("$0.04") && !frame.includes("Run on a VM"))
    assert.ok(frame.includes("[Stop]"))
  } finally { harness.renderer.destroy() }
})

test("unchanged work strips keep their renderables and changed queue controls repaint", async () => {
  const harness = await createTestRenderer({ width: 100, height: 12, useThread: false })
  const box = new BoxRenderable(harness.renderer, { width: 100 }); harness.renderer.root.add(box)
  const store = createInteractionChoiceStoreController()
  let prompt = "queued work"
  const render = () => renderAgentInteractionStrips({
    renderer: harness.renderer, primaryBox: box, auxiliaryBoxes: [], visibleAgents: [{ id: "agent-1" }] as never,
    maxAgentsPerScreen: 1, focusedAgentId: "agent-1", activeInteractionForAgent: () => null,
    selectedChoiceIndex: store.selectedChoiceIndex, setSelectedChoiceIndex: store.setSelectedIndex,
    customReply: store.customReply, customEditing: store.isCustomEditing,
    queuedPromptStripItemsForAgent: () => [{ promptId: "q", agentId: "agent-1", prompt, status: "queued", canCancel: true, canSteer: true }] as never,
    selectedQueuedPromptIndexForAgent: () => 0, onQueuedPromptAction: () => {}, agentTriggerStripItemsForAgent: () => [],
  })
  try {
    render(); await harness.renderOnce()
    const children = box.getChildren()
    render(); await harness.renderOnce()
    assert.equal(box.getChildren().every((child, index) => child === children[index]), true, "unchanged work does not rebuild the strip")
    prompt = "edited work"; render(); await harness.renderOnce()
    assert.match(harness.captureCharFrame(), /edited work/)
    assert.ok(!harness.captureCharFrame().includes("queued work"))
  } finally { harness.renderer.destroy() }
})
