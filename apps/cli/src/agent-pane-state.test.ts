import assert from "node:assert/strict"
import test from "node:test"

import {
  refreshAgentPaneState,
  selectCurrentAgentPaneEntries,
  shouldRefreshAgentPanesForSessionChange,
  trimAgentPaneEntries,
} from "@arroba/kernel-client/agent-pane-state"
import { applyTranscriptDisplayState } from "@arroba/kernel-client/transcript-display-state"

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

test("refreshAgentPaneState loads the latest page and keeps only valid collapsed turns", async () => {
  const pages = new Map<string, Array<{ entries: Array<{ role: string; turnId?: number; text: string }>; nextCursor: string | null }>>([
    ["agent-a:head", [{ entries: [{ role: "assistant", turnId: 2, text: "answer" }], nextCursor: "older" }]],
    ["agent-a:older", [{ entries: [{ role: "user", turnId: 2, text: "question" }], nextCursor: null }]],
    ["agent-b:head", [{ entries: [{ role: "user", turnId: 1, text: "other" }], nextCursor: null }]],
  ])

  const result = await refreshAgentPaneState({
    session: {
      agents: [
        {
          id: "agent-a",
          external_provider_import: {
            external_provider: "opencode",
            external_provider_session_id: "opencode:agent-a-thread",
            external_provider_session_provider_id: "agent-a-thread",
          },
        },
        {
          id: "agent-b",
          external_provider_import: {
            external_provider: "opencode",
            external_provider_session_id: "opencode:agent-b-thread",
            external_provider_session_provider_id: "agent-b-thread",
          },
        },
      ],
      focused_agent_id: "agent-b",
    },
    hasTurnWorkForAgent: () => true,
    collapsedTurnIdsByAgent: {
      "agent-a": [2, 999],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async (agentId, cursor) => {
      const key = `${agentId}:${cursor ?? "head"}`
      const page = pages.get(key)?.shift()
      assert.ok(page, `missing page for ${key}`)
      return page
    },
    hydrateEntries: (entries: Array<{ role: string; text: string }>) => entries.map((entry) => ({ ...entry })),
    collapseHistoricalTurns: (entries) => [...entries, { role: "turn_toggle", turnId: 2, text: "toggle" }],
    applyCollapsedTurns: (entries, collapsedTurnIds) =>
      collapsedTurnIds.includes(2)
        ? entries.filter((entry) => entry.role !== "turn_toggle")
        : entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.ok(result.paneEntries["agent-a"])
  assert.deepEqual(result.collapsedTurnIdsByAgent, { "agent-a": [2] })
  assert.equal(result.visibleAgentId, "agent-b")
  assert.deepEqual(result.paneEntries["agent-a"]!.map((entry) => entry.text), ["answer"])
  assert.deepEqual(result.visibleEntries.map((entry) => entry.text), ["other", "toggle"])
  assert.equal(result.previews["agent-a"], "answer")
  assert.equal(result.visibleCursor, null)
})

test("refreshAgentPaneState does not preserve current entries from another agent while busy", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{ id: "agent-a" }, { id: "agent-b" }],
      focused_agent_id: "agent-b",
    },
    hasTurnWorkForAgent: () => true,
    collapsedTurnIdsByAgent: {},
    currentPaneEntriesByAgent: {
      "agent-b": [
        {
          role: "user",
          text: "agent a prompt",
          source: "external_provider_observed",
          externalProvider: "opencode",
          externalProviderSessionId: "opencode:agent-a-thread",
          externalProviderTurnId: "agent-a-turn",
        },
        {
          role: "assistant",
          text: "agent a output",
          source: "external_provider_observed",
          externalProvider: "opencode",
          externalProviderSessionId: "opencode:agent-a-thread",
          externalProviderTurnId: "agent-a-turn",
        },
      ],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async (agentId) => ({
      entries: agentId === "agent-b"
        ? [
          {
            role: "user",
            text: "agent b prompt",
            source: "external_provider_observed",
            externalProvider: "opencode",
            externalProviderSessionId: "opencode:agent-b-thread",
            externalProviderTurnId: "agent-b-turn",
          },
          {
            role: "assistant",
            text: "agent b output",
            source: "external_provider_observed",
            externalProvider: "opencode",
            externalProviderSessionId: "opencode:agent-b-thread",
            externalProviderTurnId: "agent-b-turn",
          },
        ]
        : [],
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(result.paneEntries["agent-b"]?.map((entry) => entry.text), [
    "agent b prompt",
    "agent b output",
  ])
})

test("refreshAgentPaneState preserves compatible current live entries while busy", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{ id: "agent-a" }],
      focused_agent_id: "agent-a",
    },
    hasTurnWorkForAgent: () => true,
    collapsedTurnIdsByAgent: {},
    currentPaneEntriesByAgent: {
      "agent-a": [
        {
          role: "user",
          text: "prompt",
          source: "external_provider_observed",
          externalProvider: "codex",
          externalProviderSessionId: "codex-thread",
          externalProviderTurnId: "codex-turn",
        },
        {
          role: "assistant",
          text: "longer live assistant output",
          source: "external_provider_observed",
          externalProvider: "codex",
          externalProviderSessionId: "codex-thread",
          externalProviderTurnId: "codex-turn",
        },
      ],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async () => ({
      entries: [
        {
          role: "user",
          text: "prompt",
          source: "external_provider_observed",
          externalProvider: "codex",
          externalProviderSessionId: "codex-thread",
          externalProviderTurnId: "codex-turn",
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
    "prompt",
    "longer live assistant output",
  ])
})

test("refreshAgentPaneState does not hide new external history behind a queued prompt", async () => {
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
        {
          role: "user",
          text: "queued arroba prompt",
          queuedPrompt: {
            promptId: "prompt-1",
            agentId: "agent-a",
            status: "queued",
            attachmentCount: 0,
            steerDisabled: true,
            canSteer: false,
            canCancel: true,
            steerDisabledReason: "Steering is unavailable while the active provider turn was started outside Arroba.",
            cancelDisabledReason: null,
          },
        },
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

test("refreshAgentPaneState does not preserve stale queued prompt rows over caught-up history", async () => {
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
        {
          role: "assistant",
          text: "external assistant settled",
          source: "external_provider_observed",
          externalProvider: "opencode",
          externalProviderSessionId: "thread-a",
          externalProviderTurnId: "external-user-1",
        },
        {
          role: "user",
          text: "queued arroba prompt with enough text to make stale current entries look richer",
          queuedPrompt: {
            promptId: "prompt-1",
            agentId: "agent-a",
            status: "queued",
            attachmentCount: 0,
            steerDisabled: true,
            canSteer: false,
            canCancel: true,
            steerDisabledReason: "Steering is unavailable while the active provider turn was started outside Arroba.",
            cancelDisabledReason: null,
          },
        },
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
  assert.equal(result.visibleEntries.some((entry) => entry.queuedPrompt), false)
})

test("refreshAgentPaneState does not preserve another imported agent when refreshed history is empty", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{
        id: "agent-b",
        external_provider_import: {
          external_provider: "codex",
          external_provider_session_id: "codex:agent-b-thread",
          external_provider_session_provider_id: "agent-b-thread",
        },
      }],
      focused_agent_id: "agent-b",
    },
    hasTurnWorkForAgent: () => true,
    collapsedTurnIdsByAgent: {},
    currentPaneEntriesByAgent: {
      "agent-b": [{
        role: "assistant",
        text: "agent a output",
        source: "external_provider_observed",
        externalProvider: "opencode",
        externalProviderSessionId: "agent-a-thread",
        externalProviderTurnId: "agent-a-turn",
      }],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async () => ({
      entries: [] as Array<{
        role: string
        text: string
        source?: string | null
        externalProvider?: string | null
        externalProviderSessionId?: string | null
        externalProviderTurnId?: string | null
      }>,
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(result.visibleEntries, [])
})

test("refreshAgentPaneState ignores stray external ids without observed source when filtering panes", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{
        id: "agent-b",
        external_provider_import: {
          external_provider: "codex",
          external_provider_session_id: "codex:agent-b-thread",
          external_provider_session_provider_id: "agent-b-thread",
        },
      }],
      focused_agent_id: "agent-b",
    },
    hasTurnWorkForAgent: () => true,
    collapsedTurnIdsByAgent: {},
    currentPaneEntriesByAgent: {
      "agent-b": [{
        role: "assistant",
        text: "ordinary live output",
        source: "provider_output",
        externalProvider: "opencode",
        externalProviderSessionId: "agent-a-thread",
        externalProviderTurnId: "agent-a-turn",
      }],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async () => ({
      entries: [] as Array<{
        role: string
        text: string
        source?: string | null
        externalProvider?: string | null
        externalProviderSessionId?: string | null
        externalProviderTurnId?: string | null
      }>,
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(result.visibleEntries.map((entry) => entry.text), ["ordinary live output"])
})

test("refreshAgentPaneState ignores stale focused agent ids", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{ id: "agent-a" }, { id: "agent-b" }],
      focused_agent_id: "stale-agent",
    },
    hasTurnWorkForAgent: () => false,
    collapsedTurnIdsByAgent: {},
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async (agentId) => ({
      entries: [{ role: "assistant", text: `${agentId} history` }],
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join("\n"),
  })

  assert.equal(result.visibleAgentId, "agent-a")
  assert.deepEqual(result.visibleEntries.map((entry) => entry.text), ["agent-a history"])
})

test("refreshAgentPaneState can preserve collapsed turn ids during refresh", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{ id: "agent-a" }],
      focused_agent_id: "agent-a",
    },
    hasTurnWorkForAgent: () => false,
    collapsedTurnIdsByAgent: {
      "agent-a": [2, 999],
    },
    preserveCollapsedTurnIds: true,
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async () => ({
      entries: [
        { role: "user", turnId: 2, text: "question" },
        { role: "assistant", turnId: 2, text: "answer" },
      ],
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(result.collapsedTurnIdsByAgent, { "agent-a": [2, 999] })
})

test("refreshAgentPaneState preserves collapsed turn display across history refresh", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{ id: "agent-a" }],
      focused_agent_id: "agent-a",
    },
    hasTurnWorkForAgent: () => false,
    collapsedTurnIdsByAgent: {
      "agent-a": [1],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async () => ({
      entries: [
        { id: 1, role: "user", turnId: 1, text: "question" },
        { id: 2, role: "reasoning", turnId: 1, text: "reasoning" },
        { id: 3, role: "assistant", turnId: 1, text: "answer" },
      ],
      nextCursor: null,
    }),
    hydrateEntries: (entries) => entries.map((entry) => ({ ...entry })),
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries, collapsedTurnIds) => applyTranscriptDisplayState(entries, collapsedTurnIds),
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.filter((entry) => !entry.hidden).map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(result.collapsedTurnIdsByAgent, { "agent-a": [1] })
  assert.deepEqual(
    result.visibleEntries.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
    [
      ["user", "question"],
      ["turn_toggle", "click to expand"],
      ["assistant", "answer"],
    ],
  )
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

test("refreshAgentPaneState preserves completed turns when collapse is disabled", async () => {
  const result = await refreshAgentPaneState({
    session: {
      agents: [{ id: "agent-a" }],
      focused_agent_id: "agent-a",
    },
    hasTurnWorkForAgent: () => false,
    collapsedTurnIdsByAgent: {},
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
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
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

test("refreshAgentPaneState stops backfill when older cursor repeats without duplicating entries", async () => {
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
        { role: "user", turnId: 3, text: "third question" },
        { role: "assistant", turnId: 3, text: "third answer" },
      ],
    },
    resolveVisibleAgentId: (_agents, focusedAgentId) => focusedAgentId,
    loadHistoryPage: async (_agentId, cursor) => {
      requestedCursors.push(cursor)
      return {
        entries: [
          { role: "user", turnId: 3, text: "third question" },
          { role: "assistant", turnId: 3, text: "third answer" },
        ],
        nextCursor: "older",
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
    ["third question", "third answer"],
  )
  assert.equal(result.visibleCursor, null)
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
    hasTurnWorkForAgent: () => true,
    collapsedTurnIdsByAgent: {},
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
    collapseHistoricalTurns: (entries) => entries,
    applyCollapsedTurns: (entries) => entries,
    reindexEntries: (entries) => entries.map((entry, index) => ({ ...entry, id: index + 1 })),
    formatPreview: (entries) => entries.map((entry) => entry.text).join(" | "),
  })

  assert.deepEqual(
    result.visibleEntries.map((entry) => entry.text),
    ["question", "partial answer still streaming"],
  )
  assert.equal(result.previews["agent-a"], "question | partial answer still streaming")
})
