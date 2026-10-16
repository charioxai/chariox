import assert from "node:assert/strict"
import test from "node:test"
import { createRoot } from "solid-js"
import { createCliViewPersistence } from "./cli-view-persistence.js"

// MP-08 / MP-10 PANE-03: the persistence sampler sees every composed frame; while
// its capture timer is pending, a frame must not pay for the covered() tree walk.
test("MP-08 / MP-10 PANE-03 frames during a pending capture do not walk the renderable tree", async () => {
  let post: (buffer: unknown) => void = () => {}
  let covered = 0
  const renderer = { addPostProcessFn: (fn: typeof post) => { post = fn }, removePostProcessFn: () => {}, requestRender: () => {} }
  const dispose = createRoot((dispose) => {
    createCliViewPersistence({
      client: {} as never, renderer: renderer as never, viewport: () => undefined,
      covered: () => { covered += 1; return false },
      session: () => ({ id: "s" }) as never, agentId: () => "a",
      entries: () => [{ id: 1, role: "user", text: "hello" }] as never, attached: () => true,
    })
    return dispose
  })
  await new Promise((resolve) => setTimeout(resolve, 0))
  for (let frame = 0; frame < 20; frame += 1) post({})
  assert.equal(covered, 1, "one coverage check arms the timer; the other 19 frames must not repeat it")
  dispose()
})
