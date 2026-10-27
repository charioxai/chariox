import { strict as assert } from "node:assert"
import test from "node:test"

import { createCliUiBatchController } from "./cli-ui-batch-controller.js"

test("CLI UI batch controller flushes deferred work after the outer batch", () => {
  const calls: string[] = []
  const controller = createCliUiBatchController({
    batch: (callback) => {
      calls.push(`batch:start:${controller.isBatched()}`)
      callback()
      calls.push(`batch:end:${controller.isBatched()}`)
    },
    flushDeferredUpdates: () => {
      calls.push(`flush:${controller.isBatched()}`)
    },
  })

  controller.run(() => {
    calls.push(`work:${controller.isBatched()}`)
    controller.run(() => {
      calls.push(`nested:${controller.isBatched()}`)
    })
  })

  assert.deepEqual(calls, [
    "batch:start:true",
    "work:true",
    "batch:start:true",
    "nested:true",
    "batch:end:true",
    "batch:end:true",
    "flush:false",
  ])
})

test("CLI UI batch controller restores depth and flushes when a batch throws", () => {
  const calls: string[] = []
  const controller = createCliUiBatchController({
    batch: (callback) => {
      callback()
    },
    flushDeferredUpdates: () => {
      calls.push(`flush:${controller.isBatched()}`)
    },
  })

  assert.throws(() => {
    controller.run(() => {
      throw new Error("boom")
    })
  }, /boom/)
  assert.equal(controller.isBatched(), false)
  assert.deepEqual(calls, ["flush:false"])
})
