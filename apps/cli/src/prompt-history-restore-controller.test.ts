import assert from "node:assert/strict"
import test from "node:test"

import type { ArrobaPreferences } from "./preferences.js"
import { createPromptHistoryRestoreController } from "./prompt-history-restore-controller.js"

test("prompt history restore controller restores saved history and draft for a session", () => {
  const harness = restoreHarness({
    sessions: {
      "session-1": {
        promptHistory: ["git status", "git diff"],
        promptDraft: "continue refactor",
      },
    },
  })

  harness.controller.restore("session-1")

  assert.deepEqual(harness.historyEntries, ["git status", "git diff"])
  assert.equal(harness.navigationResetCount, 1)
  assert.equal(harness.promptText, "continue refactor")
})

test("prompt history restore controller clears prompt state when detached", () => {
  const harness = restoreHarness({
    sessions: {
      "session-1": {
        promptHistory: ["git status"],
        promptDraft: "draft",
      },
    },
  })

  harness.controller.restore(null)

  assert.deepEqual(harness.historyEntries, [])
  assert.equal(harness.navigationResetCount, 1)
  assert.equal(harness.promptText, "")
})

test("prompt history restore controller normalizes missing saved state", () => {
  const harness = restoreHarness({})

  harness.controller.restore("missing-session")

  assert.deepEqual(harness.historyEntries, [])
  assert.equal(harness.navigationResetCount, 1)
  assert.equal(harness.promptText, "")
})

function restoreHarness(preferences: ArrobaPreferences) {
  const harness = {
    historyEntries: null as string[] | null,
    navigationResetCount: 0,
    promptText: null as string | null,
    controller: null as ReturnType<typeof createPromptHistoryRestoreController> | null,
  }
  harness.controller = createPromptHistoryRestoreController({
    getPreferences: () => preferences,
    setPromptHistoryEntries: (entries) => {
      harness.historyEntries = entries
    },
    resetPromptHistoryNavigation: () => {
      harness.navigationResetCount += 1
    },
    setPromptText: (text) => {
      harness.promptText = text
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createPromptHistoryRestoreController>
  }
}
