import assert from "node:assert/strict"
import test from "node:test"

import { createPromptSessionHistoryController } from "./prompt-session-history-controller.js"

test("prompt session history controller schedules refresh only for attached sessions", () => {
  const refreshedSessions: string[] = []
  let sessionId: string | null = null
  const controller = createPromptSessionHistoryController({
    currentSessionId: () => sessionId,
    navigationDraft: () => null,
    currentPromptText: () => "",
    scheduleHistoryRefresh: (id) => {
      refreshedSessions.push(id)
    },
  })

  assert.equal(controller.scheduleSharedRefresh(), false)
  assert.deepEqual(refreshedSessions, [])

  sessionId = "session-a"
  assert.equal(controller.scheduleSharedRefresh(), true)
  assert.deepEqual(refreshedSessions, ["session-a"])
})

test("prompt session history controller prefers navigation draft over current prompt text", () => {
  let navigationDraft: string | null = null
  let promptText = "current prompt"
  const controller = createPromptSessionHistoryController({
    currentSessionId: () => "session-a",
    navigationDraft: () => navigationDraft,
    currentPromptText: () => promptText,
    scheduleHistoryRefresh: () => {},
  })

  assert.equal(controller.persistableDraft(), "current prompt")

  navigationDraft = "history draft"
  promptText = "edited prompt"
  assert.equal(controller.persistableDraft(), "history draft")
})
