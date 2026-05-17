import assert from "node:assert/strict"
import test from "node:test"

import {
  createPromptSessionStatePersistenceController,
  type PromptSessionStateUpdate,
} from "./prompt-session-state-persistence-controller.js"
import type { ArrobaPreferences } from "./preferences.js"

test("prompt session state persistence merges local preferences before saving", async () => {
  const calls: string[] = []
  const saved: Array<{ sessionId: string; next: PromptSessionStateUpdate }> = []
  let preferences: ArrobaPreferences = {}
  const controller = createPromptSessionStatePersistenceController({
    updatePreferences: (updater) => {
      calls.push("update")
      preferences = updater(preferences)
    },
    savePromptState: async (sessionId, next) => {
      calls.push("save")
      saved.push({ sessionId, next })
    },
  })

  await controller.persist("session-1", {
    promptHistory: ["one", "two"],
    promptDraft: "draft",
  })

  assert.deepEqual(calls, ["update", "save"])
  assert.deepEqual(preferences.sessions?.["session-1"], {
    promptHistory: ["one", "two"],
    promptDraft: "draft",
  })
  assert.deepEqual(saved, [{
    sessionId: "session-1",
    next: {
      promptHistory: ["one", "two"],
      promptDraft: "draft",
    },
  }])
})
