import assert from "node:assert/strict"
import test from "node:test"

import { createTranscriptRenderDeferralController } from "./transcript-render-deferral-controller.js"

test("request renders immediately outside a UI batch", () => {
  const renderable = { id: "transcript" }
  const rendered: unknown[] = []
  const controller = createTranscriptRenderDeferralController({
    isBatched: () => false,
    getRenderable: () => renderable,
    requestRender: (target) => {
      rendered.push(target)
    },
  })

  controller.request()

  assert.deepEqual(rendered, [renderable])
  assert.equal(controller.hasPending(), false)
})

test("request defers transcript rendering during a UI batch", () => {
  const renderable = { id: "transcript" }
  const rendered: unknown[] = []
  let batched = true
  const controller = createTranscriptRenderDeferralController({
    isBatched: () => batched,
    getRenderable: () => renderable,
    requestRender: (target) => {
      rendered.push(target)
    },
  })

  controller.request()
  controller.request()

  assert.deepEqual(rendered, [])
  assert.equal(controller.hasPending(), true)

  batched = false
  controller.flush()

  assert.deepEqual(rendered, [renderable])
  assert.equal(controller.hasPending(), false)
})

test("flush is idle without a pending transcript render", () => {
  let renders = 0
  const controller = createTranscriptRenderDeferralController({
    isBatched: () => false,
    getRenderable: () => ({ id: "transcript" }),
    requestRender: () => {
      renders += 1
    },
  })

  controller.flush()

  assert.equal(renders, 0)
})
