import assert from "node:assert/strict"
import test from "node:test"
import { createWaitingRoomBootstrapDefaultsController } from "./waiting-room-bootstrap-defaults.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"

test("background defaults reconcile only before user launch choices or attachment", () => {
  let state = createWaitingRoomState([], fallbackProviderCatalog(), "opencode", "default", "")
  let revision = 0
  let attached = false
  const controller = createWaitingRoomBootstrapDefaultsController({ state: () => state, ownershipRevision: () => revision, isAttached: () => attached, apply: next => { state = next } })
  controller.apply({ provider: "codex", model: "codex/gpt-6.1-sol", effort: "high" })
  assert.equal(state.providerId, "codex")
  assert.equal(state.modelId, "codex/gpt-6.1-sol")
  revision += 1
  controller.apply({ provider: "claude" })
  assert.equal(state.providerId, "codex")
  revision = 0; attached = true
  controller.apply({ provider: "claude" })
  assert.equal(state.providerId, "codex")
})
