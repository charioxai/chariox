import assert from "node:assert/strict"
import test from "node:test"

import { createFocusedStatusBadgeController } from "./focused-status-badge-controller.js"
import type { AgentBusyState } from "./session-chrome-state.js"

test("focused status badge controller projects attached busy state", () => {
  let busy = false
  const controller = createFocusedStatusBadgeController({
    isAttached: () => true,
    daemonDisconnected: () => false,
    activeStatusLabel: () => "Streaming",
    focusedBusy: () => busy,
    agents: () => [],
  })

  assert.equal(controller.badge().label, "IDLE")

  busy = true

  assert.equal(controller.badge().label, "STREAMING")
})

test("focused status badge controller projects disconnected and multi-agent states", () => {
  let disconnected = true
  let agents: AgentBusyState[] = [
    { id: "a", busy: true },
    { id: "b", busy: false },
  ]
  const controller = createFocusedStatusBadgeController({
    isAttached: () => true,
    daemonDisconnected: () => disconnected,
    activeStatusLabel: () => null,
    focusedBusy: () => false,
    agents: () => agents,
  })

  assert.equal(controller.badge().label, "DISCONNECTED")

  disconnected = false

  assert.equal(controller.badge().label, "1 IDLE 1 WORKING")

  agents = []

  assert.equal(controller.badge().label, "IDLE")
})
