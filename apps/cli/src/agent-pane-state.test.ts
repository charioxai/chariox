import assert from "node:assert/strict"
import test from "node:test"

import {
  refreshAgentPaneState,
  selectCurrentAgentPaneEntries,
  shouldRefreshAgentPanesForSessionChange,
  trimAgentPaneEntries,
} from "./agent-pane-state.js"

test("trimAgentPaneEntries drops the oldest entries and clears trimmed merge keys", () => {
  const trimmedMergeKeys: string[] = []
  const entries = trimAgentPaneEntries({
    entries: [
      { id: 1, text: "alpha", mergeKey: "tool-1" },
      { id: 2, text: "beta" },
      { id: 3, text: "gamma" },
    ],
    maxEntries: 2,
    maxChars: 99,
    onTrimmedMergeKey: (mergeKey) => {
      trimmedMergeKeys.push(mergeKey)
    },
  })

  assert.deepEqual(entries.map((entry) => entry.id), [2, 3])
  assert.deepEqual(trimmedMergeKeys, ["tool-1"])
})

test("selectCurrentAgentPaneEntries prefers the live visible transcript over stale pane cache", () => {
  const result = selectCurrentAgentPaneEntries({
    agentId: "agent-a",
    visibleAgentId: "agent-a",
    visibleEntries: [
      { id: 1, role: "user", text: "first question" },
      { id: 2, role: "assistant", text: "first answer" },
      { id: 3, role: "user", text: "second question" },
      { id: 4, role: "assistant", text: "second answer" },
    ],
    paneEntriesByAgent: {
      "agent-a": [
        { id: 1, role: "user", text: "first question" },
        { id: 2, role: "assistant", text: "first answer" },
      ],
    },
  })

  assert.deepEqual(result.map((entry) => entry.text), [
    "first question",
    "first answer",
    "second question",
    "second answer",
  ])
})

test("shouldRefreshAgentPanesForSessionChange refreshes on agent shape or focused agent changes", () => {
  assert.equal(shouldRefreshAgentPanesForSessionChange({
    previousAgents: [{ id: "agent-a" }],
    nextAgents: [{ id: "agent-a" }, { id: "agent-b" }],
    splitAgentResponseMode: false,
    currentFocusedAgentId: "agent-a",
    nextFocusedAgentId: "agent-a",
  }), true)

  assert.equal(shouldRefreshAgentPanesForSessionChange({
    previousAgents: [{ id: "agent-a" }, { id: "agent-b" }],
    nextAgents: [{ id: "agent-a" }, { id: "agent-b" }],
    splitAgentResponseMode: false,
    currentFocusedAgentId: "agent-a",
    nextFocusedAgentId: "agent-b",
  }), true)

  assert.equal(shouldRefreshAgentPanesForSessionChange({
    previousAgents: [{ id: "agent-a" }, { id: "agent-b" }],
    nextAgents: [{ id: "agent-a" }, { id: "agent-b" }],
    splitAgentResponseMode: true,
    currentFocusedAgentId: "agent-a",
    nextFocusedAgentId: "agent-b",
  }), false)

  assert.equal(shouldRefreshAgentPanesForSessionChange({
    previousAgents: [{ id: "agent-a" }, { id: "agent-b" }],
    nextAgents: [{ id: "agent-a" }, { id: "agent-b" }],
    splitAgentResponseMode: false,
    currentFocusedAgentId: "agent-a",
    nextFocusedAgentId: "agent-a",
  }), false)
})

test("refreshAgentPaneState backfills older history until a user turn and keeps only valid expanded turns", async () => {
  const pages = new Map<string, Array<{ entries: Array<{ role: string; turnId?: number; text: string }>; nextCursor: string | null }>>([
    ["agent-a:head", [{ entries: [{ role: "assistant", turnId: 2, text: "answer" }], nextCursor: "older" }]],
    ["agent-a:older", [{ entries: [{ role: "user", turnId: 2, text: "question" }], nextCursor: null }]],
    ["agent-b:head", [{ entries: [{ role: "user", turnId: 1, text: "other" }], nextCursor: null }]],
  ])

  const result = await refreshAgentPaneState({
    session: {
      agents: [{ id: "agent-a" }, { id: "agent-b" }],
      focused_agent_id: "agent-b",
    },
    hasPromptWork: true,
    expandedTurnIdsByAgent: {
      "agent-a": [2, 999],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async (agentId, cursor) => {
      const key = `${agentId}:${cursor ?? "head"}`
      const page = pages.get(key)?.shift()
      assert.ok(page, `missing page for ${key}`)
      return page
    },
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    stitchPrependedHistory: (olderEntries, currentEntries) => [...olderEntries, ...currentEntries],
    collapseHistoricalTurns: (entries) => [...entries, { role: "turn_toggle", turnId: 2, text: "toggle" }],
    applyExpandedTurns: (entries, expandedTurnIds) =>
      expandedTurnIds.includes(2)
        ? entries.filter((entry) => entry.role !== "turn_toggle")
        : entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.ok(result.paneEntries["agent-a"])
  assert.deepEqual(result.expandedTurnIdsByAgent, { "agent-a": [2] })
  assert.equal(result.visibleAgentId, "agent-b")
  assert.deepEqual(result.paneEntries["agent-a"]!.map((entry) => entry.text), ["question", "answer"])
  assert.deepEqual(result.visibleEntries.map((entry) => entry.text), ["other", "toggle"])
  assert.equal(result.previews["agent-a"], "question | answer")
  assert.equal(result.visibleCursor, null)
})

test("refreshAgentPaneState ignores stale focused agent ids", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{ id: "agent-a" }, { id: "agent-b" }],
      focused_agent_id: "stale-agent",
    },
    hasPromptWork: false,
    expandedTurnIdsByAgent: {},
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async (agentId) => ({
      entries: [{ role: "assistant", text: `${agentId} history` }],
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    stitchPrependedHistory: (olderEntries, currentEntries) => [...olderEntries, ...currentEntries],
    collapseHistoricalTurns: (entries) => entries,
    applyExpandedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join("\n"),
  })

  assert.equal(result.visibleAgentId, "agent-a")
  assert.deepEqual(result.visibleEntries.map((entry) => entry.text), ["agent-a history"])
})

test("refreshAgentPaneState can preserve expanded turn ids during refresh", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{ id: "agent-a" }],
      focused_agent_id: "agent-a",
    },
    hasPromptWork: false,
    expandedTurnIdsByAgent: {
      "agent-a": [2, 999],
    },
    preserveExpandedTurnIds: true,
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async () => ({
      entries: [
        { role: "user", turnId: 2, text: "question" },
        { role: "assistant", turnId: 2, text: "answer" },
      ],
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    stitchPrependedHistory: (olderEntries, currentEntries) => [...olderEntries, ...currentEntries],
    collapseHistoricalTurns: (entries) => entries,
    applyExpandedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(result.expandedTurnIdsByAgent, { "agent-a": [2, 999] })
})

test("refreshAgentPaneState preserves completed turns when collapse is disabled", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{ id: "agent-a" }],
      focused_agent_id: "agent-a",
    },
    hasPromptWork: false,
    expandedTurnIdsByAgent: {},
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async () => ({
      entries: [
        { role: "user", turnId: 1, text: "first question" },
        { role: "assistant", turnId: 1, text: "first answer" },
        { role: "user", turnId: 2, text: "second question" },
        { role: "assistant", turnId: 2, text: "second answer" },
      ],
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    stitchPrependedHistory: (olderEntries, currentEntries) => [...olderEntries, ...currentEntries],
    collapseHistoricalTurns: (entries) => entries,
    applyExpandedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.equal(result.visibleAgentId, "agent-a")
  assert.deepEqual(
    result.visibleEntries.map((entry) => entry.text),
    ["first question", "first answer", "second question", "second answer"],
  )
  assert.equal(
    result.previews["agent-a"],
    "first question | first answer | second question | second answer",
  )
})

test("refreshAgentPaneState backfills enough history to preserve the current pane depth", async () => {
  const requestedCursors: Array<string | null> = []
  const result = await refreshAgentPaneState<
    { id: string },
    { role: string; turnId?: number; text: string },
    { id?: number; role: string; turnId?: number; text: string },
    string
  >({
    session: {
      agents: [{ id: "agent-a" }],
      focused_agent_id: "agent-a",
    },
    hasPromptWork: false,
    expandedTurnIdsByAgent: {},
    currentPaneEntriesByAgent: {
      "agent-a": [
        { role: "user", turnId: 1, text: "first question" },
        { role: "assistant", turnId: 1, text: "first answer" },
        { role: "user", turnId: 2, text: "second question" },
        { role: "assistant", turnId: 2, text: "second answer" },
      ],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async (_agentId, cursor) => {
      requestedCursors.push(cursor)
      if (cursor === null) {
        return {
          entries: [
            { role: "user", turnId: 2, text: "second question" },
            { role: "assistant", turnId: 2, text: "second answer" },
          ],
          nextCursor: "older",
        }
      }
      return {
        entries: [
          { role: "user", turnId: 1, text: "first question" },
          { role: "assistant", turnId: 1, text: "first answer" },
        ],
        nextCursor: null,
      }
    },
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    stitchPrependedHistory: (olderEntries, currentEntries) => [...olderEntries, ...currentEntries],
    collapseHistoricalTurns: (entries) => entries,
    applyExpandedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(requestedCursors, [null, "older"])
  assert.deepEqual(
    result.visibleEntries.map((entry) => entry.text),
    ["first question", "first answer", "second question", "second answer"],
  )
})

test("refreshAgentPaneState preserves richer live pane entries while prompt work is active", async () => {
  const result = await refreshAgentPaneState<
    { id: string },
    { role: string; turnId?: number; text: string },
    { id?: number; role: string; turnId?: number; text: string },
    string
  >({
    session: {
      agents: [{ id: "agent-a" }],
      focused_agent_id: "agent-a",
    },
    hasPromptWork: true,
    expandedTurnIdsByAgent: {},
    currentPaneEntriesByAgent: {
      "agent-a": [
        { role: "user", turnId: 1, text: "question" },
        { role: "assistant", turnId: 1, text: "partial answer still streaming" },
      ],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async () => ({
      entries: [
        { role: "user", turnId: 1, text: "question" },
        { role: "assistant", turnId: 1, text: "partial" },
      ],
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    stitchPrependedHistory: (olderEntries, currentEntries) => [...olderEntries, ...currentEntries],
    collapseHistoricalTurns: (entries) => entries,
    applyExpandedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(
    result.visibleEntries.map((entry) => entry.text),
    ["question", "partial answer still streaming"],
  )
  assert.equal(result.previews["agent-a"], "question | partial answer still streaming")
})
