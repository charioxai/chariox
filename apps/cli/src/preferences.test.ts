import assert from "node:assert/strict"
import test from "node:test"

import {
  mergeSessionPromptState,
  mergeSessionPromptHistory,
  mergeUiPreferences,
  sessionPromptDraftEntry,
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

test("mergeSessionPromptState stores prompt history and draft together", () => {
  const current: ArrobaPreferences = {
    sessions: {
      "session-1": {
        promptHistory: ["git status"],
        promptDraft: "draft one",
      },
    },
  }

  assert.deepEqual(
    mergeSessionPromptState(current, "session-1", {
      promptHistory: ["git diff\n"],
      promptDraft: "draft two",
    }),
    {
      sessions: {
        "session-1": {
          promptHistory: ["git diff"],
          promptDraft: "draft two",
        },
      },
    } satisfies ArrobaPreferences,
  )
})

test("sessionPromptDraftEntry returns normalized draft text for one session", () => {
  const current: ArrobaPreferences = {
    sessions: {
      "session-1": {
        promptDraft: "hello\r\nworld",
      },
      "session-2": {
        promptDraft: "",
      },
    },
  }

  assert.equal(sessionPromptDraftEntry(current, "session-1"), "hello\nworld")
  assert.equal(sessionPromptDraftEntry(current, "session-2"), "")
  assert.equal(sessionPromptDraftEntry(current, "missing"), "")
})
