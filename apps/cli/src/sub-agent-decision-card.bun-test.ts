// MP-08 / MP-09 / MP-10 / MP-11 A3/A4 (TUI): a sub-agent's decision the user holds is ONE card in the room agent's
// panel, naming who asks; no other agent shows it.
import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import type { RuntimeInteraction } from "@chariox/kernel-client/kernel-types"
import { sessionActiveInteractionForAgent } from "@chariox/kernel-client/session-runtime-lookup"
import { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"
import { renderAgentInteractionStrips } from "./interaction-strip-renderer.js"

const card = (id: string, agentId: string, shownIn: string | null, title: string): RuntimeInteraction => ({
  id, agent_id: agentId, shown_in_agent_id: shownIn, kind: "permission", level: "warning", title, message: "Run the tests?",
  choices: [{ id: "yes", label: "Yes", reply: "yes", style: "primary" }], requested_at_ms: 1,
})

async function frame(interactions: RuntimeInteraction[], agents: string[], label: (interaction: RuntimeInteraction) => string | null) {
  const harness = await createTestRenderer({ width: 100, height: 14, useThread: false })
  const boxes = agents.map(() => new BoxRenderable(harness.renderer, { width: 100 }))
  for (const box of boxes) harness.renderer.root.add(box)
  const store = createInteractionChoiceStoreController()
  renderAgentInteractionStrips({
    renderer: harness.renderer, primaryBox: boxes[0], auxiliaryBoxes: boxes.slice(1), visibleAgents: agents.map(id => ({ id })) as never,
    maxAgentsPerScreen: agents.length, focusedAgentId: agents[0]!,
    activeInteractionForAgent: (id: string | null | undefined) => sessionActiveInteractionForAgent({ active_interactions: interactions }, id),
    askingAgentLabel: label,
    selectedChoiceIndex: store.selectedChoiceIndex, setSelectedChoiceIndex: store.setSelectedIndex,
    customReply: store.customReply, customEditing: store.isCustomEditing,
    queuedPromptStripItemsForAgent: () => [], selectedQueuedPromptIndexForAgent: () => -1, onQueuedPromptAction: () => {}, agentTriggerStripItemsForAgent: () => [],
  } as never)
  await harness.renderOnce()
  const rows = harness.captureCharFrame().split("\n").map(row => row.trimEnd()).filter(Boolean)
  harness.renderer.destroy()
  return rows.join("\n")
}

test("A3/A4 a hidden child's decision shows in the room agent's panel and names the child", async () => {
  const text = await frame([card("card-c", "child", "room", "Run tests")], ["room", "other"], interaction => interaction.agent_id === "child" ? "reviewer" : null)
  assert.match(text, /WARNING • reviewer • Run tests/, text)
  assert.equal(text.match(/Run the tests\?/g)?.length, 1, "one card, shown once")
})

test("A3/A4 the room agent's own decision names nobody", async () => {
  const text = await frame([card("card-own", "room", null, "Run tests")], ["room"], () => null)
  assert.match(text, /WARNING • Run tests/, text)
})

test("A3/A4 an ancestor-held decision has no card", async () => {
  const text = await frame([card("held", "child", null, "Run tests")], ["room"], () => "reviewer")
  assert.doesNotMatch(text, /Run the tests\?/, text)
})
