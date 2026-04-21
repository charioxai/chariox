import assert from "node:assert/strict"
import os from "node:os"
import path from "node:path"
import { mkdtemp, rm } from "node:fs/promises"
import test from "node:test"

import {
  loadPreferences,
  mergeRelayCloudProfile,
  mergeSessionPromptState,
  mergeSessionPromptHistory,
  mergeUiPreferences,
  preferencesPath,
  relayCloudProfile,
  saveRelayCloudProfile,
  saveSessionPromptState,
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
      theme: "sober",
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
        theme: "sober",
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

test("mergeRelayCloudProfile stores and clears the cloud relay profile", () => {
  const current: ArrobaPreferences = {
    providers: {
      opencode: {
        model: "openai/gpt-5",
      },
    },
  }

  const merged = mergeRelayCloudProfile(current, {
    apiUrl: "https://cloud.example",
    email: "user@example.com",
    accountId: "account-1",
    userId: "user-1",
    accountSlug: "user",
    realmId: "realm-1",
    relayUrl: "wss://relay.example",
    issuerId: "issuer-1",
    clientId: "client-1",
  })
  assert.equal(relayCloudProfile(merged)?.clientId, "client-1")
  assert.equal(relayCloudProfile(mergeRelayCloudProfile(merged, null)), null)
})

test("saveSessionPromptState preserves prompt history across queued draft-only writes", async () => {
  const previousConfigHome = process.env.XDG_CONFIG_HOME
  const tempConfigHome = await mkdtemp(path.join(os.tmpdir(), "arroba-preferences-"))
  process.env.XDG_CONFIG_HOME = tempConfigHome

  try {
    await Promise.all([
      saveSessionPromptState("session-1", {
        promptHistory: ["prompt 1", "prompt 2"],
        promptDraft: "",
      }),
      saveSessionPromptState("session-1", {
        promptDraft: "draft prompt",
      }),
    ])

    const current = await loadPreferences()
    assert.equal(preferencesPath(), path.join(tempConfigHome, "arroba", "config.json"))
    assert.deepEqual(sessionPromptHistoryEntries(current, "session-1"), ["prompt 1", "prompt 2"])
    assert.equal(sessionPromptDraftEntry(current, "session-1"), "draft prompt")
  } finally {
    if (previousConfigHome === undefined) {
      delete process.env.XDG_CONFIG_HOME
    } else {
      process.env.XDG_CONFIG_HOME = previousConfigHome
    }
    await rm(tempConfigHome, { recursive: true, force: true })
  }
})

test("saveRelayCloudProfile persists the configured cloud relay profile", async () => {
  const previousConfigHome = process.env.XDG_CONFIG_HOME
  const tempConfigHome = await mkdtemp(path.join(os.tmpdir(), "arroba-preferences-"))
  process.env.XDG_CONFIG_HOME = tempConfigHome

  try {
    await saveRelayCloudProfile({
      apiUrl: "https://cloud.example",
      email: "user@example.com",
      accountId: "account-1",
      userId: "user-1",
      accountSlug: "user",
      realmId: "realm-1",
      relayUrl: "wss://relay.example",
      issuerId: "issuer-1",
      clientId: "client-1",
    })

    const current = await loadPreferences()
    assert.equal(relayCloudProfile(current)?.relayUrl, "wss://relay.example")
  } finally {
    if (previousConfigHome === undefined) {
      delete process.env.XDG_CONFIG_HOME
    } else {
      process.env.XDG_CONFIG_HOME = previousConfigHome
    }
    await rm(tempConfigHome, { recursive: true, force: true })
  }
})
