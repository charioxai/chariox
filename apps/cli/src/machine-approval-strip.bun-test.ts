import assert from "node:assert/strict"
import test from "node:test"
import { BoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { machineApprovalLinesForAgent, withRuntimeInteractions, type MachineApprovalReceipt } from "@chariox/kernel-client/machine-approval-receipts"
import { createInteractionChoiceStoreController } from "./interaction-choice-store-controller.js"
import { renderAgentInteractionStrips } from "./interaction-strip-renderer.js"

const message = "builder in Build Room · Run the build — Acquire small in north at 0.000284 EUR per minute"
const receipt = (state: MachineApprovalReceipt["state"], expires_at_ms: number | null = null): MachineApprovalReceipt =>
  ({ id: "machine-acquisition-direct-a", shown_in_agent_id: "agent-1", message, state, expires_at_ms })

test("an approved machine request stays in the strip while the service response is held, then the strip closes", async () => {
  const harness = await createTestRenderer({ width: 140, height: 12, useThread: false })
  const box = new BoxRenderable(harness.renderer, { width: 140 })
  harness.renderer.root.add(box)
  const store = createInteractionChoiceStoreController()
  let session: Parameters<typeof withRuntimeInteractions>[0] = {}
  const render = async () => {
    renderAgentInteractionStrips({
      renderer: harness.renderer, primaryBox: box, auxiliaryBoxes: [], visibleAgents: [{ id: "agent-1" }] as never,
      maxAgentsPerScreen: 1, focusedAgentId: "agent-1", activeInteractionForAgent: () => null,
      selectedChoiceIndex: store.selectedChoiceIndex, setSelectedChoiceIndex: store.setSelectedIndex,
      customReply: store.customReply, customEditing: store.isCustomEditing,
      queuedPromptStripItemsForAgent: () => [], selectedQueuedPromptIndexForAgent: () => -1, onQueuedPromptAction: () => {}, agentTriggerStripItemsForAgent: () => [],
      machineApprovalLinesForAgent: agentId => machineApprovalLinesForAgent(session.machine_approval_receipts, agentId, 1_000),
    })
    await harness.renderOnce()
    return harness.captureCharFrame().replaceAll("\n", " ")
  }
  try {
    // Approve: the card leaves with the kernel's event; the receipt keeps the wait visible while the service response is held.
    session = withRuntimeInteractions(session, [], [receipt("approving")])
    assert.match(await render(), /Run the build — Acquire small in north at 0\.000284 EUR per minute — Approving…/)
    assert.equal(box.visible, true)
    // The service answered: the strip closes.
    session = withRuntimeInteractions(session, [], [receipt("sent")])
    assert.ok(!(await render()).includes("Approving"))
    assert.equal(box.visible, false)
    // An unanswered approval that expired is not silent.
    session = withRuntimeInteractions(session, [], [receipt("expired", Date.parse("1970-01-01T00:00:00Z"))])
    assert.match(await render(), /Expired at 1970-01-01T00:00:00Z/)
  } finally { harness.renderer.destroy() }
})
