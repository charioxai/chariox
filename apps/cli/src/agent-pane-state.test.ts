import assert from "node:assert/strict"
import test from "node:test"

import { refreshAgentPaneState, trimAgentPaneEntries } from "./agent-pane-state.js"

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
