import assert from "node:assert/strict"
import test from "node:test"

import {
  createPromptDraftPersistController,
  type PromptDraftPersistRequest,
} from "./prompt-draft-persist-controller.js"

type FakeTimer = {
  callback: () => void
  delayMs: number
  cleared: boolean
}

function createHarness() {
  const timers: FakeTimer[] = []
  const persisted: PromptDraftPersistRequest[] = []
  const errors: Array<{ error: unknown; request: PromptDraftPersistRequest }> = []
  const controller = createPromptDraftPersistController<FakeTimer>({
    delayMs: 300,
    scheduleTimer(callback, delayMs) {
      const timer = { callback, delayMs, cleared: false }
      timers.push(timer)
      return timer
    },
    clearTimer(timer) {
      timer.cleared = true
    },
    async persistPromptDraft(request) {
      persisted.push(request)
    },
    onPersistError(error, request) {
      errors.push({ error, request })
    },
  })

  return { controller, errors, persisted, timers }
}

test("prompt draft persist controller coalesces scheduled drafts", async () => {
  const { controller, persisted, timers } = createHarness()

  controller.schedule("s1", "first")
  controller.schedule("s1", "second")

  assert.equal(timers[0]?.cleared, true)
  assert.equal(timers[1]?.delayMs, 300)
  timers[1]?.callback()
  await Promise.resolve()

  assert.deepEqual(persisted, [{ sessionId: "s1", promptDraft: "second" }])
})

test("prompt draft persist controller flushes the pending draft immediately", async () => {
  const { controller, persisted, timers } = createHarness()

  controller.schedule("s1", "draft")
  await controller.flush()

  assert.equal(timers[0]?.cleared, true)
  assert.deepEqual(persisted, [{ sessionId: "s1", promptDraft: "draft" }])
})

test("prompt draft persist controller can clear pending draft state", async () => {
  const { controller, persisted, timers } = createHarness()

  controller.schedule("s1", "draft")
  controller.clearPending()
  timers[0]?.callback()
  await Promise.resolve()

  assert.equal(timers[0]?.cleared, true)
  assert.deepEqual(persisted, [])
})
