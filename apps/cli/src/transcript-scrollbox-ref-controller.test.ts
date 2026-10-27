import assert from "node:assert/strict"
import test from "node:test"

import { createTranscriptScrollboxRefController } from "./transcript-scrollbox-ref-controller.js"

test("transcript scrollbox ref controller owns mounted scrollbox helpers", () => {
  const removedIds: string[] = []
  const scrolls: Array<{ x: number; y: number }> = []
  let renderCount = 0
  let scrollTop = 10
  const controller = createTranscriptScrollboxRefController<{
    scrollLeft: number
    scrollTop: number
    scrollTo(position: { x: number; y: number }): void
    requestRender(): void
    remove(renderableId: string): void
  }>()

  assert.equal(controller.hasScrollbox(), false)
  assert.equal(controller.scrollTop(4), 4)
  assert.equal(controller.scrollState(), null)
  assert.equal(controller.remove("entry-1"), false)

  controller.assignScrollbox({
    scrollLeft: 3,
    get scrollTop() {
      return scrollTop
    },
    scrollTo(position) {
      scrolls.push(position)
      scrollTop = position.y
    },
    requestRender() {
      renderCount += 1
    },
    remove(renderableId) {
      removedIds.push(renderableId)
    },
  })

  assert.equal(controller.hasScrollbox(), true)
  assert.equal(controller.current()?.scrollTop, 10)
  assert.deepEqual(controller.scrollState(), { left: 3, top: 10 })

  controller.scrollTo({ x: 3, y: 42 })
  controller.requestRender()

  assert.deepEqual(scrolls, [{ x: 3, y: 42 }])
  assert.equal(controller.scrollTop(0), 42)
  assert.equal(renderCount, 1)
  assert.equal(controller.remove("entry-2"), true)
  assert.deepEqual(removedIds, ["entry-2"])
})
