import assert from "node:assert/strict"
import test from "node:test"

import {
  focusedAgentIdForAgentPaneSession,
  preserveLoadedHistoryBlobs,
  prependHistoryEntriesWithoutDuplicates,
  refreshAgentPaneState,
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

test("trimAgentPaneEntries preserves entry array identity when unchanged", () => {
  const source = [
    { id: 1, text: "alpha", mergeKey: "tool-1" },
    { id: 2, text: "beta" },
  ]
  const entries = trimAgentPaneEntries({
    entries: source,
    maxEntries: 4,
    maxChars: 99,
  })

  assert.equal(entries, source)
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
    collapsedTurnIds: [1],
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
  })

  assert.deepEqual(result.map((entry) => entry.text), ["question", "loaded tool output", "answer"])
  assert.equal(result[1]?.historyBlobLoaded, true)
  assert.equal(result[1]?.id, 2)
})

test("refreshAgentPaneState projects kernel-scoped history pages without local import repair", async () => {
  const pages = new Map<string, Array<{
    entries: Array<{
      role: string
      text: string
      source?: string
      externalProvider?: string
      externalProviderSessionId?: string
      externalProviderTurnId?: string
      turnId?: number
    }>
    nextCursor: string | null
  }>>([
    ["agent-a:head", [{
      entries: [
        {
          role: "assistant",
          text: "codex current output",
          source: "external_provider_observed",
          externalProvider: "codex",
          externalProviderSessionId: "thread-1",
          externalProviderTurnId: "assistant-2",
          turnId: 2,
        },
        {
          role: "assistant",
          text: "opencode current output",
          source: "external_provider_observed",
          externalProvider: "opencode",
          externalProviderSessionId: "thread-2",
          externalProviderTurnId: "assistant-2",
          turnId: 2,
        },
      ],
      nextCursor: "older",
    }]],
    ["agent-a:older", [{
      entries: [
        {
          role: "user",
          text: "codex older prompt",
          source: "external_provider_observed",
          externalProvider: "codex",
          externalProviderSessionId: "thread-1",
          externalProviderTurnId: "user-1",
          turnId: 1,
        },
        {
          role: "user",
          text: "opencode older prompt",
          source: "external_provider_observed",
          externalProvider: "opencode",
          externalProviderSessionId: "thread-2",
          externalProviderTurnId: "user-1",
          turnId: 1,
        },
      ],
      nextCursor: null,
    }]],
  ])

  const result = await refreshAgentPaneState({
    session: {
      agents: [{
        id: "agent-a",
        external_provider_import: {
          external_provider: "codex",
          external_provider_session_id: "codex:thread-1",
          external_provider_session_provider_id: "thread-1",
        },
      }],
      focused_agent_id: "agent-a",
    },
    hasTurnWorkForAgent: () => false,
    collapsedTurnIdsByAgent: {},
    currentPaneEntriesByAgent: {
      "agent-a": [
        { role: "user", text: "existing prompt", turnId: 0 },
        { role: "assistant", text: "existing answer", turnId: 0 },
        { role: "user", text: "existing follow-up", turnId: 0 },
        { role: "assistant", text: "existing follow-up answer", turnId: 0 },
      ],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async (agentId, cursor) => {
      const key = `${agentId}:${cursor ?? "head"}`
      const page = pages.get(key)?.shift()
      assert.ok(page, `missing page for ${key}`)
      return page
    },
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(result.visibleEntries.map((entry) => entry.text), [
    "codex older prompt",
    "opencode older prompt",
    "codex current output",
    "opencode current output",
  ])
  assert.equal(result.visibleCursor, null)
  assert.equal(
    result.previews["agent-a"],
    "codex older prompt | opencode older prompt | codex current output | opencode current output",
  )
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

test("refreshAgentPaneState backfills enough history to preserve current pane depth", async () => {
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
    hasTurnWorkForAgent: () => false,
    collapsedTurnIdsByAgent: {},
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
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(requestedCursors, [null, "older"])
  assert.deepEqual(
    result.visibleEntries.map((entry) => entry.text),
    ["first question", "first answer", "second question", "second answer"],
  )
  assert.equal(result.visibleCursor, null)
})

test("refreshAgentPaneState backfills idle panes while another agent has turn work", async () => {
  const requestedCursorsByAgent: Record<string, Array<string | null>> = {
    "agent-a": [],
    "agent-b": [],
  }
  const result = await refreshAgentPaneState<
    { id: string },
    { role: string; turnId?: number; text: string },
    { id?: number; role: string; turnId?: number; text: string },
    string
  >({
    session: {
      agents: [{ id: "agent-a" }, { id: "agent-b" }],
      focused_agent_id: "agent-a",
    },
    hasTurnWorkForAgent: (agent) => agent.id === "agent-b",
    collapsedTurnIdsByAgent: {},
    currentPaneEntriesByAgent: {
      "agent-a": [
        { role: "user", turnId: 1, text: "idle older prompt" },
        { role: "assistant", turnId: 1, text: "idle older reply" },
        { role: "user", turnId: 2, text: "idle latest prompt" },
        { role: "assistant", turnId: 2, text: "idle latest reply" },
      ],
      "agent-b": [
        { role: "user", turnId: 3, text: "busy prompt" },
        { role: "assistant", turnId: 3, text: "busy live reply with more text" },
      ],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async (agentId, cursor) => {
      requestedCursorsByAgent[agentId]?.push(cursor)
      if (agentId === "agent-a" && cursor === null) {
        return {
          entries: [
            { role: "user", turnId: 2, text: "idle latest prompt" },
            { role: "assistant", turnId: 2, text: "idle latest reply" },
          ],
          nextCursor: "older-idle",
        }
      }
      if (agentId === "agent-a") {
        return {
          entries: [
            { role: "user", turnId: 1, text: "idle older prompt" },
            { role: "assistant", turnId: 1, text: "idle older reply" },
          ],
          nextCursor: null,
        }
      }
      return {
        entries: [
          { role: "user", turnId: 3, text: "busy prompt" },
        ],
        nextCursor: "older-busy",
      }
    },
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(requestedCursorsByAgent["agent-a"], [null, "older-idle"])
  assert.deepEqual(requestedCursorsByAgent["agent-b"], [null])
  assert.deepEqual(result.paneEntries["agent-a"]?.map((entry) => entry.text), [
    "idle older prompt",
    "idle older reply",
    "idle latest prompt",
    "idle latest reply",
  ])
  assert.deepEqual(result.paneEntries["agent-b"]?.map((entry) => entry.text), [
    "busy prompt",
    "busy live reply with more text",
  ])
})

test("refreshAgentPaneState prefers refreshed external history over queued prompt echoes", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{
        id: "agent-a",
        external_provider_import: {
          external_provider: "opencode",
          external_provider_session_id: "opencode:thread-a",
          external_provider_session_provider_id: "thread-a",
        },
      }],
      focused_agent_id: "agent-a",
    },
    hasTurnWorkForAgent: () => true,
    collapsedTurnIdsByAgent: {},
    currentPaneEntriesByAgent: {
      "agent-a": [
        {
          role: "user",
          text: "external prompt",
          source: "external_provider_observed",
          externalProvider: "opencode",
          externalProviderSessionId: "thread-a",
          externalProviderTurnId: "external-user-1",
        },
        { role: "user", text: "queued arroba prompt" },
      ],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async () => ({
      entries: [
        {
          role: "user",
          text: "external prompt",
          source: "external_provider_observed",
          externalProvider: "opencode",
          externalProviderSessionId: "thread-a",
          externalProviderTurnId: "external-user-1",
        },
        {
          role: "assistant",
          text: "external assistant settled",
          source: "external_provider_observed",
          externalProvider: "opencode",
          externalProviderSessionId: "thread-a",
          externalProviderTurnId: "external-user-1",
        },
      ],
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(result.visibleEntries.map((entry) => entry.text), [
    "external prompt",
    "external assistant settled",
  ])
})

test("refreshAgentPaneState preserves loaded history blob content across refresh", async () => {
  const result = await refreshAgentPaneState<
    { id: string },
    {
      role: string
      turnId?: number
      text: string
      historyBlobId?: string
      historyBlobAgentId?: string
    },
    {
      id?: number
      role: string
      turnId?: number
      text: string
      historyBlobId?: string
      historyBlobAgentId?: string
      historyBlobSourceId?: string
      historyBlobSourceAgentId?: string
      historyBlobLoaded?: boolean
    },
    string
  >({
    session: {
      agents: [{ id: "agent-a" }],
      focused_agent_id: "agent-a",
    },
    hasTurnWorkForAgent: () => false,
    collapsedTurnIdsByAgent: { "agent-a": [1] },
    currentPaneEntriesByAgent: {
      "agent-a": [
        { role: "user", turnId: 1, text: "question" },
        {
          role: "tool",
          turnId: 1,
          text: "TOOL_STEP_01 loaded output",
          historyBlobSourceId: "blob-1",
          historyBlobSourceAgentId: "agent-a",
          historyBlobLoaded: true,
        },
        { role: "assistant", turnId: 1, text: "answer" },
      ],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async () => ({
      entries: [
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
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(
    result.visibleEntries.map((entry) => entry.text),
    ["question", "TOOL_STEP_01 loaded output", "answer"],
  )
  assert.equal(result.visibleEntries[1]?.historyBlobLoaded, true)
})
