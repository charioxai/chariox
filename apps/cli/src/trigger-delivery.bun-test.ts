import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { renderAgentInteractionStrips } from "./interaction-strip-renderer.js"
import { buildTranscriptEntryRenderable } from "./transcript-render.js"
import { createTranscriptSyntaxStyle } from "./theme.js"
import type { AgentInstance, TranscriptEntry } from "./cli-types.js"

test("MP-08/MP-10/MP-11 retired trigger leaves a visible TUI resend stripe", async () => {
  const harness = await createTestRenderer({ width: 100, height: 8, useThread: false })
  const box = new BoxRenderable(harness.renderer, { width: "100%" })
  harness.renderer.root.add(box)
  try {
    renderAgentInteractionStrips({
      renderer: harness.renderer, primaryBox: box, auxiliaryBoxes: [],
      visibleAgents: [{ id: "agent-1" } as AgentInstance], maxAgentsPerScreen: 1, focusedAgentId: "agent-1",
      activeInteractionForAgent: () => null, selectedChoiceIndex: () => 0, setSelectedChoiceIndex: () => {},
      customReply: () => "", customEditing: () => false, queuedPromptStripItemsForAgent: () => [],
      selectedQueuedPromptIndexForAgent: () => 0, onQueuedPromptAction: () => {}, agentTriggerStripItemsForAgent: () => [],
      unconfirmedTriggerDeliveriesForAgent: () => [{ prompt_id: "firing-1", trigger_id: "retired", agent_id: "agent-1", origin: "schedule", prompt: "Review restart", state: "not_confirmed", busy_policy: "steer", recorded_at_ms: 1 }],
    })
    await harness.renderOnce()
    const frame = harness.captureCharFrame()
    assert.match(frame, /Schedule.*Not confirmed.*steer/)
    assert.ok(frame.includes("/trigger resend firing-1"))
  } finally { harness.renderer.destroy() }
})

test("MP-08/MP-10/MP-11 TUI prompts retain their trigger origin through live updates", async () => {
  const harness = await createTestRenderer({ width: 80, height: 8, useThread: false })
  const syntax = createTranscriptSyntaxStyle()
  const entry: TranscriptEntry = { id: 1, role: "user", text: "Review restart", promptOrigin: "schedule" }
  const node = buildTranscriptEntryRenderable(harness.renderer, entry, syntax, () => {}, () => {})
  harness.renderer.root.add(node.wrapper)
  try {
    await harness.renderOnce()
    assert.ok(harness.captureCharFrame().includes("Schedule"))
    node.update({ ...entry, promptOrigin: "notification", text: "Review settlement" })
    await harness.renderOnce()
    const updated = harness.captureCharFrame()
    assert.ok(updated.includes("Notification"))
    assert.ok(!updated.includes("Schedule"))
    assert.ok(updated.includes("Review settlement"))
  } finally { harness.renderer.destroy(); syntax.destroy() }
})
