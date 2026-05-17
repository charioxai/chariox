import assert from "node:assert/strict"
import test from "node:test"

import { createAgentPaneRuntimeStoreController } from "./agent-pane-runtime-store-controller.js"

test("agent pane runtime store owns pane render and auxiliary-agent state", () => {
  const store = createAgentPaneRuntimeStoreController<
    { id: string },
    { id: number },
    { id: string },
    { id: string }
  >()

  store.registerScrollbox("agent-1", { id: "scrollbox-1" })
  store.entryRenderables.set("agent-1", new Map([[7, { id: 7 }]]))
  store.emptyRenderables.set("agent-1", { id: "empty-1" })
  store.toolStates.set("agent-1", new Map([["tool-1", { id: "tool-1" }]]))
  store.setCurrentAuxiliaryAgentId(0, "agent-1")

  assert.equal(store.scrollboxes.get("agent-1")?.id, "scrollbox-1")
  assert.equal(store.entryRenderables.get("agent-1")?.get(7)?.id, 7)
  assert.equal(store.emptyRenderables.get("agent-1")?.id, "empty-1")
  assert.deepEqual([...(store.toolUpdatesForAgent("agent-1") ?? [])], [{ id: "tool-1" }])
  assert.equal(store.getCurrentAuxiliaryAgentId(0), "agent-1")

  store.unregisterScrollbox("agent-1")
  store.clearCurrentAuxiliaryAgentIds()

  assert.equal(store.scrollboxes.has("agent-1"), false)
  assert.equal(store.getCurrentAuxiliaryAgentId(0), null)
  assert.equal(store.toolUpdatesForAgent("missing"), undefined)
})
