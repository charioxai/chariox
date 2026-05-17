import assert from "node:assert/strict"
import test from "node:test"

import type {
  SessionHistoryCursor,
  SessionHistoryPage,
  SessionHistoryPageEntry,
  TranscriptEntry,
} from "./cli-types.js"
import { createTranscriptHistoryLoadController } from "./transcript-history-load-controller.js"

const cursor: SessionHistoryCursor = {
  before_entry_index: 10,
  before_entry_char_offset: null,
}

function historyEntry(
  entryIndex: number,
  kind: SessionHistoryPageEntry["entry"]["kind"],
  text: string,
): SessionHistoryPageEntry {
  return {
    entry_index: entryIndex,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: { kind, text },
  }
}

function page(entries: SessionHistoryPageEntry[], nextCursor: SessionHistoryCursor | null): SessionHistoryPage {
  return {
    entries,
    next_cursor: nextCursor,
  }
}

test("loadOlderPage fetches older pages until history starts at a user turn", async () => {
  const prepended: TranscriptEntry[][] = []
  const requestedCursors: Array<SessionHistoryCursor | null> = []
  let nextCursor: SessionHistoryCursor | null = cursor
  let loading = false
  const controller = createTranscriptHistoryLoadController({
    isAttached: () => true,
    isLoading: () => loading,
    getCursor: () => nextCursor,
    getSessionId: () => "session-1",
    getVisibleAgentId: () => "agent-1",
    getEntryCounter: () => 7,
    setLoading: (value) => {
      loading = value
    },
    setNextCursor: (value) => {
      nextCursor = value
    },
    async loadPage(_sessionId, requestedCursor) {
      requestedCursors.push(requestedCursor)
      return requestedCursors.length === 1
        ? page([historyEntry(9, "provider_output", "answer")], { before_entry_index: 8, before_entry_char_offset: null })
        : page([historyEntry(8, "user_prompt", "prompt")], null)
    },
    prependEntries: async (entries) => {
      prepended.push(entries)
    },
    flashError: () => {},
    logWarning: () => {},
    formatError: String,
  })

  assert.equal(await controller.loadOlderPage(), true)

  assert.deepEqual(requestedCursors, [cursor, { before_entry_index: 8, before_entry_char_offset: null }])
  assert.equal(nextCursor, null)
  assert.equal(loading, false)
  assert.equal(prepended.length, 1)
  assert.deepEqual(prepended[0]?.map((entry) => [entry.id, entry.role, entry.text]), [
    [8, "user", "prompt"],
    [9, "assistant", "answer"],
  ])
})

test("loadOlderPage skips when history cannot load", async () => {
  let setLoadingCalls = 0
  const controller = createTranscriptHistoryLoadController({
    isAttached: () => false,
    isLoading: () => false,
    getCursor: () => cursor,
    getSessionId: () => "session-1",
    getVisibleAgentId: () => null,
    getEntryCounter: () => 0,
    setLoading: () => {
      setLoadingCalls += 1
    },
    setNextCursor: () => {},
    loadPage: async () => page([], null),
    prependEntries: async () => {},
    flashError: () => {},
    logWarning: () => {},
    formatError: String,
  })

  assert.equal(await controller.loadOlderPage(), false)
  assert.equal(setLoadingCalls, 0)
})

test("loadOlderPage drops stale results after generation changes", async () => {
  const prepended: TranscriptEntry[][] = []
  let nextCursor: SessionHistoryCursor | null = cursor
  let loading = false
  const controller = createTranscriptHistoryLoadController({
    isAttached: () => true,
    isLoading: () => loading,
    getCursor: () => nextCursor,
    getSessionId: () => "session-1",
    getVisibleAgentId: () => null,
    getEntryCounter: () => 0,
    setLoading: (value) => {
      loading = value
    },
    setNextCursor: (value) => {
      nextCursor = value
    },
    async loadPage() {
      controller.bumpGeneration()
      return page([historyEntry(7, "user_prompt", "prompt")], null)
    },
    prependEntries: async (entries) => {
      prepended.push(entries)
    },
    flashError: () => {},
    logWarning: () => {},
    formatError: String,
  })

  assert.equal(await controller.loadOlderPage(), false)

  assert.equal(nextCursor, cursor)
  assert.deepEqual(prepended, [])
  assert.equal(loading, false)
})

test("loadOlderPage reports load failures", async () => {
  let flash = ""
  let warning = ""
  const controller = createTranscriptHistoryLoadController({
    isAttached: () => true,
    isLoading: () => false,
    getCursor: () => cursor,
    getSessionId: () => "session-1",
    getVisibleAgentId: () => null,
    getEntryCounter: () => 0,
    setLoading: () => {},
    setNextCursor: () => {},
    async loadPage() {
      throw new Error("history unavailable")
    },
    prependEntries: async () => {},
    flashError: (message) => {
      flash = message
    },
    logWarning: (message) => {
      warning = message
    },
    formatError: (error) => error instanceof Error ? error.message : String(error),
  })

  assert.equal(await controller.loadOlderPage(), false)

  assert.equal(warning, "older history load failed")
  assert.equal(flash, "failed to load older history")
})
