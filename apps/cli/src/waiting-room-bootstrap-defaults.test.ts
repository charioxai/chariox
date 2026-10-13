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


test("MP-08/MP-11 defaults never retry an authentication refusal", async () => {
  const { LocalIpcError } = await import("./ipc.js")
  const { readWaitingRoomConfiguredDefaults } = await import("./waiting-room-bootstrap-defaults.js")
  let reads = 0, disposed = 0
  const client = { onKernelEvent: () => () => { disposed += 1 } }
  await assert.rejects(readWaitingRoomConfiguredDefaults(client as never, async () => {
    reads += 1
    throw new LocalIpcError("read defaults", "permission denied", "authorization_denied", false)
  }), /permission denied/)
  assert.equal(reads, 1)
  assert.equal(disposed, 1)
})
