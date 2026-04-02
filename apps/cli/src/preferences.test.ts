import assert from "node:assert/strict"
import test from "node:test"

import { mergeUiPreferences, type ArrobaPreferences } from "./preferences.js"

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
