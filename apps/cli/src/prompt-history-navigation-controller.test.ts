import assert from "node:assert/strict"
import test from "node:test"

import {
  createPromptHistoryNavigationController,
} from "./prompt-history-navigation-controller.js"

test("navigate applies prompt history state, persists draft text, and retains focus", () => {
  let promptText = "draft"
  let navigationIndex: number | null = null
  let navigationDraft: string | null = null
  let focusRetained = 0
  const persistedDrafts: unknown[] = []
  const controller = createPromptHistoryNavigationController({
    getPromptText: () => promptText,
    getEntries: () => ["first", "second"],
    getNavigationIndex: () => navigationIndex,
    getNavigationDraft: () => navigationDraft,
    setNavigationIndex: (index) => {
      navigationIndex = index
    },
    setNavigationDraft: (draft) => {
      navigationDraft = draft
    },
    setPromptText: (text) => {
      promptText = text
    },
    getSessionId: () => "session-1",
    schedulePromptDraftPersist: (sessionId, draft) => {
      persistedDrafts.push({ sessionId, draft })
    },
    retainPromptFocus: () => {
      focusRetained += 1
    },
  })

  assert.equal(controller.navigate("previous"), true)
  assert.equal(promptText, "second")
  assert.equal(navigationIndex, 1)
  assert.equal(navigationDraft, "draft")
  assert.deepEqual(persistedDrafts, [{ sessionId: "session-1", draft: "draft" }])
  assert.equal(focusRetained, 1)

  assert.equal(controller.navigate("next"), true)
  assert.equal(promptText, "draft")
  assert.equal(navigationIndex, null)
  assert.equal(navigationDraft, null)
  assert.deepEqual(persistedDrafts, [
    { sessionId: "session-1", draft: "draft" },
    { sessionId: "session-1", draft: "draft" },
  ])
  assert.equal(focusRetained, 2)
})

test("navigate skips draft persistence while detached", () => {
  let promptText = ""
  let navigationIndex: number | null = null
  let navigationDraft: string | null = null
  const persistedDrafts: unknown[] = []
  const controller = createPromptHistoryNavigationController({
    getPromptText: () => promptText,
    getEntries: () => ["history"],
    getNavigationIndex: () => navigationIndex,
    getNavigationDraft: () => navigationDraft,
    setNavigationIndex: (index) => {
      navigationIndex = index
    },
    setNavigationDraft: (draft) => {
      navigationDraft = draft
    },
    setPromptText: (text) => {
      promptText = text
    },
    getSessionId: () => null,
    schedulePromptDraftPersist: (sessionId, draft) => {
      persistedDrafts.push({ sessionId, draft })
    },
    retainPromptFocus: () => {},
  })

  assert.equal(controller.navigate("previous"), true)
  assert.equal(promptText, "history")
  assert.deepEqual(persistedDrafts, [])
})

test("navigate is idle when history policy returns no text change", () => {
  let mutated = false
  const controller = createPromptHistoryNavigationController({
    getPromptText: () => "draft",
    getEntries: () => [],
    getNavigationIndex: () => null,
    getNavigationDraft: () => null,
    setNavigationIndex: () => {
      mutated = true
    },
    setNavigationDraft: () => {
      mutated = true
    },
    setPromptText: () => {
      mutated = true
    },
    getSessionId: () => "session-1",
    schedulePromptDraftPersist: () => {
      mutated = true
    },
    retainPromptFocus: () => {
      mutated = true
    },
  })

  assert.equal(controller.navigate("previous"), false)
  assert.equal(mutated, false)
})
