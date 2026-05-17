import assert from "node:assert/strict"
import test from "node:test"

import { createPromptInputHistoryController } from "./prompt-input-history-controller.js"

test("replaceFromHydration applies history, latest sequence, navigation reset, and persistence", async () => {
  let entries: readonly string[] = []
  let persisted: unknown
  let resetCount = 0
  const controller = createPromptInputHistoryController({
    getCurrentSessionId: () => "session-1",
    getAttachmentId: () => null,
    getEntries: () => entries,
    setEntries: (nextEntries) => {
      entries = nextEntries
    },
    resetNavigation: () => {
      resetCount += 1
    },
    clearDraftPersistQueue: () => {},
    persistPromptState: async (sessionId, next) => {
      persisted = { sessionId, next }
    },
    recordPromptInputHistory: async () => ({ sequence: 1, text: "" }),
    onSharedHistoryPersistFailed: () => {},
    onPromptEchoPersistFailed: () => {},
    onPromptStatePersistFailed: () => {},
    onRecordSharedHistoryFailed: () => {},
  })

  await controller.replaceFromHydration("session-1", ["one", "two"], 42)

  assert.deepEqual(entries, ["one", "two"])
  assert.equal(controller.latestSequence(), 42)
  assert.equal(resetCount, 1)
  assert.deepEqual(persisted, {
    sessionId: "session-1",
    next: { promptHistory: ["one", "two"] },
  })
})

test("appendShared sorts remote entries, updates latest sequence, and persists changed history", () => {
  let currentSessionId = "session-1"
  let entries: readonly string[] = ["local"]
  const persisted: unknown[] = []
  const controller = createPromptInputHistoryController({
    getCurrentSessionId: () => currentSessionId,
    getAttachmentId: () => null,
    getEntries: () => entries,
    setEntries: (nextEntries) => {
      entries = nextEntries
    },
    resetNavigation: () => {},
    clearDraftPersistQueue: () => {},
    persistPromptState: async (sessionId, next) => {
      persisted.push({ sessionId, next })
    },
    recordPromptInputHistory: async () => ({ sequence: 1, text: "" }),
    onSharedHistoryPersistFailed: () => {},
    onPromptEchoPersistFailed: () => {},
    onPromptStatePersistFailed: () => {},
    onRecordSharedHistoryFailed: () => {},
  })

  assert.equal(controller.appendShared("other-session", [{ sequence: 3, text: "ignored" }]), false)
  assert.deepEqual(entries, ["local"])

  assert.equal(controller.appendShared("session-1", [
    { sequence: 3, text: "third" },
    { sequence: 2, text: "second" },
  ]), true)

  assert.deepEqual(entries, ["local", "second", "third"])
  assert.equal(controller.latestSequence(), 3)
  assert.deepEqual(persisted, [{
    sessionId: "session-1",
    next: { promptHistory: ["local", "second", "third"] },
  }])

  currentSessionId = "session-1"
  assert.equal(controller.appendShared("session-1", [{ sequence: 4, text: "third" }]), false)
  assert.equal(controller.latestSequence(), 4)
})

test("appendEcho appends local terminal echo only for the current session", () => {
  let currentSessionId: string | null = null
  let entries: readonly string[] = ["one"]
  const persisted: unknown[] = []
  const controller = createPromptInputHistoryController({
    getCurrentSessionId: () => currentSessionId,
    getAttachmentId: () => null,
    getEntries: () => entries,
    setEntries: (nextEntries) => {
      entries = nextEntries
    },
    resetNavigation: () => {},
    clearDraftPersistQueue: () => {},
    persistPromptState: async (sessionId, next) => {
      persisted.push({ sessionId, next })
    },
    recordPromptInputHistory: async () => ({ sequence: 1, text: "" }),
    onSharedHistoryPersistFailed: () => {},
    onPromptEchoPersistFailed: () => {},
    onPromptStatePersistFailed: () => {},
    onRecordSharedHistoryFailed: () => {},
  })

  assert.equal(controller.appendEcho("two"), false)

  currentSessionId = "session-1"
  assert.equal(controller.appendEcho("two"), true)
  assert.equal(controller.appendEcho("two"), false)

  assert.deepEqual(entries, ["one", "two"])
  assert.deepEqual(persisted, [{
    sessionId: "session-1",
    next: { promptHistory: ["one", "two"] },
  }])
})

test("recordPromptAreaEntry persists prompt state and records slash commands into shared history", async () => {
  let entries: readonly string[] = []
  let resetCount = 0
  let clearedDraftQueue = 0
  const persisted: unknown[] = []
  const recorded: unknown[] = []
  const controller = createPromptInputHistoryController({
    getCurrentSessionId: () => "session-1",
    getAttachmentId: () => "attachment-1",
    getEntries: () => entries,
    setEntries: (nextEntries) => {
      entries = nextEntries
    },
    resetNavigation: () => {
      resetCount += 1
    },
    clearDraftPersistQueue: () => {
      clearedDraftQueue += 1
    },
    persistPromptState: async (sessionId, next) => {
      persisted.push({ sessionId, next })
    },
    recordPromptInputHistory: async (sessionId, attachmentId, kind, text) => {
      recorded.push({ sessionId, attachmentId, kind, text })
      return { sequence: 9, text: "/help" }
    },
    onSharedHistoryPersistFailed: () => {},
    onPromptEchoPersistFailed: () => {},
    onPromptStatePersistFailed: () => {},
    onRecordSharedHistoryFailed: () => {},
  })

  assert.equal(controller.recordPromptAreaEntry(null, "ignored"), false)
  assert.equal(controller.recordPromptAreaEntry("session-1", "/help   "), true)
  await Promise.resolve()

  assert.deepEqual(entries, ["/help"])
  assert.equal(controller.latestSequence(), 9)
  assert.equal(resetCount, 1)
  assert.equal(clearedDraftQueue, 1)
  assert.deepEqual(recorded, [{
    sessionId: "session-1",
    attachmentId: "attachment-1",
    kind: "command",
    text: "/help",
  }])
  assert.deepEqual(persisted, [
    {
      sessionId: "session-1",
      next: { promptHistory: ["/help"], promptDraft: "" },
    },
  ])
})

test("recordPromptAreaEntry reports shared history record failures", async () => {
  let failure: unknown
  const controller = createPromptInputHistoryController({
    getCurrentSessionId: () => "session-1",
    getAttachmentId: () => null,
    getEntries: () => [],
    setEntries: () => {},
    resetNavigation: () => {},
    clearDraftPersistQueue: () => {},
    persistPromptState: async () => {},
    recordPromptInputHistory: async () => {
      throw new Error("record failed")
    },
    onSharedHistoryPersistFailed: () => {},
    onPromptEchoPersistFailed: () => {},
    onPromptStatePersistFailed: () => {},
    onRecordSharedHistoryFailed: (_sessionId, error) => {
      failure = error
    },
  })

  controller.recordPromptAreaEntry("session-1", "/bad")
  await Promise.resolve()
  await Promise.resolve()

  assert.match(failure instanceof Error ? failure.message : String(failure), /record failed/)
})
