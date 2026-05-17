import assert from "node:assert/strict"
import test from "node:test"

import type { RuntimeInteraction } from "./cli-types.js"
import { createAgentInteractionStripController } from "./agent-interaction-strip-controller.js"

test("agent interaction strip controller renders current pane and interaction inputs", () => {
  const interaction = interactionFixture()
  const rendered: unknown[] = []
  let primaryBox = "primary"
  let auxiliaryBoxes = ["aux-1", "aux-2"]
  let visibleAgents: Array<{ id: string } | null> = [{ id: "agent-a" }, { id: "agent-b" }]
  const controller = createAgentInteractionStripController({
    renderer: "renderer",
    primaryBox: () => primaryBox,
    auxiliaryBoxes: () => auxiliaryBoxes,
    visibleAgents: () => visibleAgents,
    maxAgentsPerScreen: () => 3,
    focusedAgentId: () => "agent-a",
    activeInteractionForAgent: (agentId) => agentId === "agent-a" ? interaction : null,
    selectedChoiceIndex: () => 1,
    setSelectedChoiceIndex: () => {},
    customReply: () => "custom",
    customEditing: () => true,
    renderStrips: (options) => {
      rendered.push(options)
    },
  })

  controller.render()
  primaryBox = "next-primary"
  auxiliaryBoxes = ["next-aux"]
  visibleAgents = [{ id: "agent-c" }]
  controller.render()

  assert.equal(rendered.length, 2)
  const first = rendered[0] as {
    renderer: string
    primaryBox: string
    auxiliaryBoxes: string[]
    visibleAgents: Array<{ id: string }>
    maxAgentsPerScreen: number
    focusedAgentId: string
    activeInteractionForAgent: (agentId: string) => RuntimeInteraction | null
    selectedChoiceIndex: (interactionId: string) => number
    customReply: (interactionId: string) => string
    customEditing: (interactionId: string) => boolean
  }
  assert.equal(first.renderer, "renderer")
  assert.equal(first.primaryBox, "primary")
  assert.deepEqual(first.auxiliaryBoxes, ["aux-1", "aux-2"])
  assert.deepEqual(first.visibleAgents, [{ id: "agent-a" }, { id: "agent-b" }])
  assert.equal(first.maxAgentsPerScreen, 3)
  assert.equal(first.focusedAgentId, "agent-a")
  assert.equal(first.activeInteractionForAgent("agent-a"), interaction)
  assert.equal(first.selectedChoiceIndex("interaction-a"), 1)
  assert.equal(first.customReply("interaction-a"), "custom")
  assert.equal(first.customEditing("interaction-a"), true)

  const second = rendered[1] as { primaryBox: string; auxiliaryBoxes: string[]; visibleAgents: Array<{ id: string }> }
  assert.equal(second.primaryBox, "next-primary")
  assert.deepEqual(second.auxiliaryBoxes, ["next-aux"])
  assert.deepEqual(second.visibleAgents, [{ id: "agent-c" }])
})

function interactionFixture(): RuntimeInteraction {
  return {
    id: "interaction-a",
    agent_id: "agent-a",
    kind: "choice",
    level: "info",
    message: "Choose",
    choices: [{ id: "approve", label: "Approve", reply: "approve" }],
    requested_at_ms: 1,
  }
}
