import assert from "node:assert/strict"
import test from "node:test"

import { createPromptHistoryAttachmentController } from "./prompt-history-attachment-controller.js"

test("prompt history attachment sync hydrates a newly attached session once", async () => {
  const harness = createHarness({ attachedSessionId: "session-1" })

  await harness.controller.sync()
  await harness.controller.sync()

  assert.deepEqual(harness.calls(), ["restore:session-1", "hydrate:session-1"])
})

test("prompt history attachment sync restores detached history and invalidates hydration", () => {
  const harness = createHarness({ attachedSessionId: null })

  harness.controller.sync()

  assert.deepEqual(harness.calls(), ["restore:null", "invalidate"])
})

test("prompt history attachment sync warns only while the failed session is current", async () => {
  const current = createHarness({
    attachedSessionId: "session-1",
    hydrateError: new Error("failed"),
  })

  await current.controller.sync()

  assert.deepEqual(current.calls(), [
    "restore:session-1",
    "hydrate:session-1",
    "warn:session-1:failed",
  ])

  const stale = createHarness({
    attachedSessionId: "session-1",
    currentSessionId: "session-2",
    hydrateError: new Error("stale"),
  })

  await stale.controller.sync()

  assert.deepEqual(stale.calls(), ["restore:session-1", "hydrate:session-1"])
})

function createHarness(options: {
  attachedSessionId: string | null
  currentSessionId?: string | null
  hydrateError?: Error
}) {
  const calls: string[] = []
  const controller = createPromptHistoryAttachmentController({
    getAttachedSessionId: () => options.attachedSessionId,
    restorePromptHistory: (sessionId) => {
      calls.push(`restore:${sessionId ?? "null"}`)
    },
    invalidateHydration: () => {
      calls.push("invalidate")
    },
    hydratePromptHistory: async (sessionId) => {
      calls.push(`hydrate:${sessionId}`)
      if (options.hydrateError) {
        throw options.hydrateError
      }
    },
    isCurrentSession: (sessionId) => sessionId === (options.currentSessionId ?? options.attachedSessionId),
    warnHydrationError: (sessionId, error) => {
      calls.push(`warn:${sessionId}:${error instanceof Error ? error.message : String(error)}`)
    },
  })

  return {
    controller,
    calls: () => calls,
  }
}
