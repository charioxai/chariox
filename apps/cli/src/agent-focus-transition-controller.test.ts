import assert from "node:assert/strict"
import test from "node:test"

import { createAgentFocusTransitionController } from "./agent-focus-transition-controller.js"

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve
    reject = promiseReject
  })
  return { promise, resolve, reject }
}

test("track returns the operation result and clears the pending transition", async () => {
  const controller = createAgentFocusTransitionController()
  const transition = deferred<string>()

  const result = controller.track(() => transition.promise)

  assert.equal(controller.hasPending(), true)
  transition.resolve("focused")
  assert.equal(await result, "focused")
  assert.equal(controller.hasPending(), false)
})

test("track clears pending transition when the operation rejects", async () => {
  const controller = createAgentFocusTransitionController()
  const transition = deferred<string>()

  const result = controller.track(() => transition.promise)

  transition.reject(new Error("focus failed"))
  await assert.rejects(result, /focus failed/)
  assert.equal(controller.hasPending(), false)
})

test("wait follows the latest pending transition", async () => {
  const controller = createAgentFocusTransitionController()
  const first = deferred<string>()
  const second = deferred<string>()

  const firstResult = controller.track(() => first.promise)
  const secondResult = controller.track(() => second.promise)
  const waited = controller.wait()

  first.resolve("first")
  assert.equal(await firstResult, "first")
  assert.equal(controller.hasPending(), true)

  second.resolve("second")
  await waited
  assert.equal(await secondResult, "second")
  assert.equal(controller.hasPending(), false)
})

test("wait resolves immediately when there is no pending transition", async () => {
  const controller = createAgentFocusTransitionController()

  await controller.wait()

  assert.equal(controller.hasPending(), false)
})
