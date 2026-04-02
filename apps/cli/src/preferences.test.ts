import assert from "node:assert/strict"
import test from "node:test"

import {
  mergeSessionPromptHistory,
  mergeUiPreferences,
  sessionPromptHistoryEntries,
  type ArrobaPreferences,
} from "./preferences.js"

test("mergeUiPreferences updates the global response layout without losing other preferences", () => {
  const current: ArrobaPreferences = {
    providers: {
      opencode: {
        model: "openai/gpt-5",
        effort: "medium",
      },
    },
    ui: {
      multiAgentResponseLayout: "individual",
    },
  }

  assert.deepEqual(
    mergeUiPreferences(current, { multiAgentResponseLayout: "split" }),
    {
      providers: {
        opencode: {
          model: "openai/gpt-5",
          effort: "medium",
        },
      },
      ui: {
        multiAgentResponseLayout: "split",
      },
    } satisfies ArrobaPreferences,
  )
})

test("mergeSessionPromptHistory stores prompt history without losing other sessions or preferences", () => {
  const current: ArrobaPreferences = {
    providers: {
      opencode: {
        model: "openai/gpt-5",
      },
    },
    sessions: {
      "session-1": {
        promptHistory: ["git status"],
      },
    },
  }

  assert.deepEqual(
    mergeSessionPromptHistory(current, "session-2", ["git diff", "git log\n"]),
    {
      providers: {
        opencode: {
          model: "openai/gpt-5",
        },
      },
      sessions: {
        "session-1": {
          promptHistory: ["git status"],
        },
        "session-2": {
          promptHistory: ["git diff", "git log"],
        },
      },
    } satisfies ArrobaPreferences,
  )
})

test("sessionPromptHistoryEntries returns normalized prompt history for one session only", () => {
  const current: ArrobaPreferences = {
    sessions: {
      "session-1": {
        promptHistory: ["git status", "git diff\n", "", "   "],
      },
      "session-2": {
        promptHistory: ["git log"],
      },
    },
  }

  assert.deepEqual(sessionPromptHistoryEntries(current, "session-1"), ["git status", "git diff"])
  assert.deepEqual(sessionPromptHistoryEntries(current, "session-2"), ["git log"])
  assert.deepEqual(sessionPromptHistoryEntries(current, "missing"), [])
})
