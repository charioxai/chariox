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
  type CharioxPreferences,
} from "./preferences.js"

test("mergeUiPreferences updates the global response layout without losing other preferences", () => {
  const current: CharioxPreferences = {
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
    } satisfies CharioxPreferences,
  )
})

test("mergeSessionPromptHistory stores prompt history without losing other sessions or preferences", () => {
  const current: CharioxPreferences = {
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
    } satisfies CharioxPreferences,
  )
})

test("sessionPromptHistoryEntries returns normalized prompt history for one session only", () => {
  const current: CharioxPreferences = {
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
  const current: CharioxPreferences = {
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
    } satisfies CharioxPreferences,
  )
})

test("sessionPromptDraftEntry returns normalized draft text for one session", () => {
  const current: CharioxPreferences = {
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
  const current: CharioxPreferences = {
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
  const previousExplicitHome = process.env.CHARIOX_HOME
  const previousConfigHome = process.env.XDG_CONFIG_HOME
  const tempConfigHome = await mkdtemp(path.join(os.tmpdir(), "chariox-preferences-"))
  process.env.XDG_CONFIG_HOME = tempConfigHome
  process.env.CHARIOX_HOME = tempConfigHome

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
    assert.equal(preferencesPath(), path.join(tempConfigHome, "config.json"))
    assert.deepEqual(sessionPromptHistoryEntries(current, "session-1"), ["prompt 1", "prompt 2"])
    assert.equal(sessionPromptDraftEntry(current, "session-1"), "draft prompt")
  } finally {
    if (previousExplicitHome === undefined) delete process.env.CHARIOX_HOME
    else process.env.CHARIOX_HOME = previousExplicitHome
    if (previousConfigHome === undefined) {
      delete process.env.XDG_CONFIG_HOME
    } else {
      process.env.XDG_CONFIG_HOME = previousConfigHome
    }
    await rm(tempConfigHome, { recursive: true, force: true })
  }
})

test("saveRelayCloudProfile persists the configured cloud relay profile", async () => {
  const previousExplicitHome = process.env.CHARIOX_HOME
  const previousConfigHome = process.env.XDG_CONFIG_HOME
  const tempConfigHome = await mkdtemp(path.join(os.tmpdir(), "chariox-preferences-"))
  process.env.XDG_CONFIG_HOME = tempConfigHome
  process.env.CHARIOX_HOME = tempConfigHome

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
    if (previousExplicitHome === undefined) delete process.env.CHARIOX_HOME
    else process.env.CHARIOX_HOME = previousExplicitHome
    if (previousConfigHome === undefined) {
      delete process.env.XDG_CONFIG_HOME
    } else {
      process.env.XDG_CONFIG_HOME = previousConfigHome
    }
    await rm(tempConfigHome, { recursive: true, force: true })
  }
})

test("explicit CLI profiles isolate keys/preferences and scrub predecessor secrets at 0600", async () => {
  const { readFile, writeFile, stat } = await import("node:fs/promises")
  const { defaultCliRelayIdentityPath } = await import("./cli-relay-identity-store.js")
  const previous = process.env.CHARIOX_HOME
  const root = await mkdtemp(path.join(os.tmpdir(), "chariox-profile-boundary-"))
  try {
    process.env.CHARIOX_HOME = root
    assert.equal(preferencesPath(), path.join(root, "config.json"))
    assert.equal(defaultCliRelayIdentityPath(), path.join(root, "relay", "cli-identity-v1.json"))
    const privatePredecessor = { apiUrl: "https://cloud.example.test", email: "fixture@example.test", accountId: "account-a", userId: "owner-a", accountSlug: "a", realmId: "realm-a", issuerId: "issuer-a", relayUrl: "wss://relay.example.test", machineCredential: "synthetic-predecessor-machine", cloudSessionToken: "synthetic-predecessor-human", kernelCredential: "synthetic-kernel-grant", relayToken: "synthetic-kernel-token" }
    await writeFile(preferencesPath(), JSON.stringify({relay: {cloud: privatePredecessor}}), {mode: 0o644})
    const loaded = await loadPreferences()
    assert.equal(loaded.relay?.cloud?.accountId, "account-a")
    const stored = await readFile(preferencesPath(), "utf8")
    assert.ok(!stored.includes("synthetic"))
    assert.equal((await stat(preferencesPath())).mode & 0o777, 0o600)
    await saveRelayCloudProfile(privatePredecessor)
    assert.ok(!(await readFile(preferencesPath(), "utf8")).includes("synthetic"))
    process.env.CHARIOX_HOME = path.join(root, "second-profile")
    assert.equal((await loadPreferences()).relay, undefined)
  } finally {
    if (previous === undefined) delete process.env.CHARIOX_HOME
    else process.env.CHARIOX_HOME = previous
    await rm(root, {recursive: true, force: true})
  }
})
