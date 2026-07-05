import assert from "node:assert/strict"
import test from "node:test"

import {
  entryBelongsToAgent,
  focusedAgentIdForAgentPaneSession,
  preserveLoadedHistoryBlobs,
  prependHistoryEntriesWithoutDuplicates,
  selectCurrentAgentPaneEntries,
  shouldPreferCurrentPaneEntries,
  shouldRefreshAgentPanesForSessionChange,
  trimAgentPaneEntries,
} from "./agent-pane-state.js"

test("selectCurrentAgentPaneEntries prefers the live visible pane over stale cache", () => {
  const result = selectCurrentAgentPaneEntries({
    agentId: "agent-a",
    visibleAgentId: "agent-a",
    visibleEntries: [
      { id: 1, role: "user", text: "first question" },
      { id: 2, role: "assistant", text: "first answer" },
    ],
    paneEntriesByAgent: {
      "agent-a": [
        { id: 1, role: "user", text: "stale question" },
      ],
    },
  })

  assert.deepEqual(result.map((entry) => entry.text), ["first question", "first answer"])
})

test("shouldRefreshAgentPanesForSessionChange follows agent shape and focus policy", () => {
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
    splitAgentResponseMode: true,
    currentFocusedAgentId: "agent-a",
    nextFocusedAgentId: "agent-b",
  }), false)

  assert.equal(shouldRefreshAgentPanesForSessionChange({
    previousAgents: [{ id: "agent-a" }, { id: "agent-b" }],
    nextAgents: [{ id: "agent-a" }, { id: "agent-b" }],
    splitAgentResponseMode: false,
    currentFocusedAgentId: "agent-a",
    nextFocusedAgentId: "agent-b",
  }), true)
})

test("trimAgentPaneEntries drops oldest entries and reports trimmed merge keys", () => {
  const trimmedMergeKeys: string[] = []
  const entries = trimAgentPaneEntries({
    entries: [
      { id: 1, text: "alpha", mergeKey: "tool-1" },
      { id: 2, text: "beta" },
      { id: 3, text: "gamma" },
    ],
    maxEntries: 2,
    maxChars: 99,
    onTrimmedMergeKey: (mergeKey) => trimmedMergeKeys.push(mergeKey),
  })

  assert.deepEqual(entries.map((entry) => entry.id), [2, 3])
  assert.deepEqual(trimmedMergeKeys, ["tool-1"])
})

test("shouldPreferCurrentPaneEntries preserves richer live entries with matching lineage", () => {
  assert.equal(shouldPreferCurrentPaneEntries([
    {
      role: "user",
      text: "prompt",
      source: "external_provider_observed",
      externalProvider: "codex",
      externalProviderSessionId: "thread",
      externalProviderTurnId: "turn",
    },
    {
      role: "assistant",
      text: "longer live assistant output",
      source: "external_provider_observed",
      externalProvider: "codex",
      externalProviderSessionId: "thread",
      externalProviderTurnId: "turn",
    },
  ], [
    {
      role: "user",
      text: "prompt",
      source: "external_provider_observed",
      externalProvider: "codex",
      externalProviderSessionId: "thread",
      externalProviderTurnId: "turn",
    },
  ]), true)

  assert.equal(shouldPreferCurrentPaneEntries([
    {
      role: "assistant",
      text: "other agent output",
      source: "external_provider_observed",
      externalProvider: "opencode",
      externalProviderSessionId: "other-thread",
      externalProviderTurnId: "turn",
    },
  ], [
    {
      role: "assistant",
      text: "current agent output",
      source: "external_provider_observed",
      externalProvider: "codex",
      externalProviderSessionId: "thread",
      externalProviderTurnId: "turn",
    },
  ]), false)
})

test("prependHistoryEntriesWithoutDuplicates prepends older unique lineage only", () => {
  const entries = prependHistoryEntriesWithoutDuplicates([
    { role: "user", text: "first", turnId: 1 },
    { role: "assistant", text: "second", turnId: 2 },
  ], [
    { role: "assistant", text: "second", turnId: 2 },
  ])

  assert.deepEqual(entries.map((entry) => entry.text), ["first", "second"])
})

test("preserveLoadedHistoryBlobs keeps expanded loaded blob content after refresh", () => {
  type BlobEntry = {
    id?: number
    role: string
    turnId?: number
    text: string
    historyBlobId?: string
    historyBlobAgentId?: string
    historyBlobSourceId?: string
    historyBlobSourceAgentId?: string
    historyBlobLoaded?: boolean
  }

  const result = preserveLoadedHistoryBlobs<BlobEntry>({
    currentEntries: [
      { role: "user", turnId: 1, text: "question" },
      {
        role: "tool",
        turnId: 1,
        text: "loaded tool output",
        historyBlobSourceId: "blob-1",
        historyBlobSourceAgentId: "agent-a",
        historyBlobLoaded: true,
      },
      { role: "assistant", turnId: 1, text: "answer" },
    ],
    refreshedEntries: [
      { role: "user", turnId: 1, text: "question" },
      {
        role: "tool",
        turnId: 1,
        text: "",
        historyBlobId: "blob-1",
        historyBlobAgentId: "agent-a",
      },
      { role: "assistant", turnId: 1, text: "answer" },
    ],
    expandedTurnIds: [1],
    applyExpandedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
  })

  assert.deepEqual(result.map((entry) => entry.text), ["question", "loaded tool output", "answer"])
  assert.equal(result[1]?.historyBlobLoaded, true)
  assert.equal(result[1]?.id, 2)
})

test("entryBelongsToAgent scopes external observed entries to imported agents", () => {
  const agent = {
    external_provider_import: {
      external_provider: "codex",
      external_provider_session_id: "codex:thread-a",
      external_provider_session_provider_id: "thread-a",
    },
  }

  assert.equal(entryBelongsToAgent(agent, {
    source: "external_provider_observed",
    externalProvider: "codex",
    externalProviderSessionId: "thread-a",
  }), true)

  assert.equal(entryBelongsToAgent(agent, {
    source: "external_provider_observed",
    externalProvider: "opencode",
    externalProviderSessionId: "thread-b",
  }), false)

  assert.equal(entryBelongsToAgent(agent, {
    source: "provider_output",
    externalProvider: "opencode",
    externalProviderSessionId: "thread-b",
  }), true)
})

test("focusedAgentIdForAgentPaneSession ignores stale focus", () => {
  assert.equal(focusedAgentIdForAgentPaneSession({
    focused_agent_id: "stale",
    agents: [{ id: "agent-a" }, { id: "agent-b" }],
  }), "agent-a")
  assert.equal(focusedAgentIdForAgentPaneSession({
    focused_agent_id: "agent-b",
    agents: [{ id: "agent-a" }, { id: "agent-b" }],
  }), "agent-b")
})
