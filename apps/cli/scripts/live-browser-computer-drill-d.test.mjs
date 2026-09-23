import assert from "node:assert/strict"
import test from "node:test"
import { assertDrillDComputerAction } from "./live-browser-computer-drill-d.mjs"

test("rejects a Browser history record as evidence of the attributed desktop edit", () => {
  const action = {
    action_id: "action-typed",
    sequence: 12,
    actor_id: "agent:agent-live",
    mode: "browser",
    kind: "keyboard_text",
    arguments: { utf8_byte_count: 42 },
    targets: [{ kind: "browser_tab", id: "tab-live" }],
    state: "completed",
    outcome: { status: "completed" },
  }

  assert.throws(() => assertDrillDComputerAction(action, {
    actionId: "action-typed",
    agentId: "agent-live",
    baselineSequence: 7,
  }), /completed Computer keyboard_text action targeting desktop/)
})
