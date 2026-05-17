import assert from "node:assert/strict"
import test from "node:test"

import { createPrimaryTranscriptRuntimeStoreController } from "./primary-transcript-runtime-store-controller.js"

test("primary transcript runtime store owns mutable render and tool state", () => {
  const store = createPrimaryTranscriptRuntimeStoreController<
    { wrapper: { y?: number } },
    { id: string },
    { status: string }
  >({ initialMountedTranscriptAgentId: "agent-1" })

  store.tools.set("tool-1", { status: "running" })
  store.activeToolLabels.set("tool-1", "running command")
  store.transcriptRenderables.set(7, { wrapper: { y: 42 } })
  store.setEmptyRenderable({ id: "empty" })
  store.setLastScrollTop(12)
  assert.equal(store.getMountedTranscriptAgentId(), "agent-1")
  store.setMountedTranscriptAgentId("agent-2")

  assert.equal(store.tools.get("tool-1")?.status, "running")
  assert.deepEqual([...store.activeToolLabelValues()], ["running command"])
  assert.equal(store.entryWrapperY(7), 42)
  assert.equal(store.getEmptyRenderable()?.id, "empty")
  assert.equal(store.getLastScrollTop(), 12)
  assert.equal(store.getMountedTranscriptAgentId(), "agent-2")

  store.deleteTool("tool-1")
  store.clearActiveToolLabels()
  store.clearTools()
  store.setEmptyRenderable(undefined)

  assert.equal(store.tools.size, 0)
  assert.deepEqual([...store.activeToolLabelValues()], [])
  assert.equal(store.getEmptyRenderable(), undefined)
  assert.equal(store.entryWrapperY(99), null)
})
