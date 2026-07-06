import assert from "node:assert/strict"
import test from "node:test"

import {
  applyTranscriptDisplayState,
  collapseLatestTranscriptTurn,
  compactTranscriptDisplayEntries,
  replaceCollapsedTranscriptTurnIds,
  projectCompactTranscriptDisplayState,
  projectTranscriptBlobToggleDisplayState,
  projectTranscriptDisplayState,
  projectSettledTranscriptTurnDisplayState,
  projectTranscriptTurnToggleDisplayState,
  resolveVisibleTurnToggle,
  setTranscriptBlobCollapsed,
  updateCollapsedTranscriptTurnState,
  type TranscriptDisplayEntry,
} from "./transcript-display-state.js"

function baseTurnEntries(): TranscriptDisplayEntry[] {
  return [
    { id: 1, role: "user", text: "Investigate the transcript UI", turnId: 1 },
    { id: 2, role: "reasoning", text: "Thinking through the render model", turnId: 1 },
    { id: 3, role: "tool", text: "tool output", turnId: 1, historyBlobId: "blob-1" },
    { id: 4, role: "assistant", text: "I changed the transcript layout.", turnId: 1 },
  ]
}

test("applyTranscriptDisplayState keeps completed turns expanded by default", () => {
  const entries = applyTranscriptDisplayState(baseTurnEntries())

  assert.deepEqual(
    entries.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
    [
      ["user", "Investigate the transcript UI"],
      ["turn_toggle", "click to collapse"],
      ["reasoning", "Thinking through the render model"],
      ["tool", "tool output"],
      ["assistant", "I changed the transcript layout."],
    ],
  )
  assert.equal(entries.find((entry) => entry.id === 2)?.blobCollapsible, true)
  assert.equal(entries.find((entry) => entry.id === 2)?.blobCollapsed, true)
  assert.equal(entries.find((entry) => entry.id === 3)?.blobCollapsible, true)
})

test("projectTranscriptDisplayState returns display entries with derived transcript state", () => {
  const projection = projectTranscriptDisplayState([
    ...baseTurnEntries(),
    { id: 5, role: "user", text: "Next prompt", turnId: 4 },
  ], [1])

  assert.deepEqual(
    projection.entries.filter((entry) => !entry.hidden).map((entry) => [entry.turnId ?? null, entry.role]),
    [
      [1, "user"],
      [1, "turn_toggle"],
      [1, "assistant"],
      [4, "user"],
    ],
  )
  assert.equal(projection.currentTurnId, 4)
  assert.equal(projection.nextTurnId, 5)
  assert.equal(projection.entryCounter, 6)
})

test("projectCompactTranscriptDisplayState compacts inputs before deriving display state", () => {
  const projection = projectCompactTranscriptDisplayState([
    baseTurnEntries()[0],
    null,
    { id: 99, role: "turn_toggle", text: "stale display row", turnId: 1 },
    undefined,
    false,
    ...baseTurnEntries().slice(1),
  ], [1])

  assert.deepEqual(
    projection.entries.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
    [
      ["user", "Investigate the transcript UI"],
      ["turn_toggle", "click to expand"],
      ["assistant", "I changed the transcript layout."],
    ],
  )
  assert.equal(projection.currentTurnId, 1)
  assert.equal(projection.entryCounter, 5)
})

test("compactTranscriptDisplayEntries removes nullish placeholder entries", () => {
  assert.deepEqual(
    compactTranscriptDisplayEntries([
      baseTurnEntries()[0],
      null,
      undefined,
      false,
      baseTurnEntries()[1],
    ]).map((entry) => entry.id),
    [1, 2],
  )
})

test("projectSettledTranscriptTurnDisplayState clears stale collapsed state for the active turn", () => {
  const projection = projectSettledTranscriptTurnDisplayState(baseTurnEntries(), [1, 7])

  assert.equal(projection.settledTurnId, 1)
  assert.deepEqual(projection.collapsedTurnIds, [7])
  assert.deepEqual(
    projection.entries.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
    [
      ["user", "Investigate the transcript UI"],
      ["turn_toggle", "click to collapse"],
      ["reasoning", "Thinking through the render model"],
      ["tool", "tool output"],
      ["assistant", "I changed the transcript layout."],
    ],
  )
})

test("projectSettledTranscriptTurnDisplayState keeps incomplete history turns expanded", () => {
  const projection = projectSettledTranscriptTurnDisplayState(
    baseTurnEntries().map((entry) => ({
      ...entry,
      historyTurnCompletedAtMs: null,
    })),
    [1],
  )

  assert.equal(projection.settledTurnId, 1)
  assert.deepEqual(projection.collapsedTurnIds, [])
  assert.equal(projection.entries.find((entry) => entry.role === "turn_toggle"), undefined)
  assert.deepEqual(
    projection.entries.filter((entry) => !entry.hidden).map((entry) => entry.role),
    ["user", "reasoning", "tool", "assistant"],
  )
})

test("projectTranscriptTurnToggleDisplayState projects expansion through shared state", () => {
  const current = applyTranscriptDisplayState(baseTurnEntries(), [1])
  const projection = projectTranscriptTurnToggleDisplayState(current, 1, [1], 999)

  assert.equal(projection?.turnId, 1)
  assert.equal(projection?.expanded, true)
  assert.deepEqual(projection?.collapsedTurnIds, [])
  assert.deepEqual(
    projection?.entries.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
    [
      ["user", "Investigate the transcript UI"],
      ["turn_toggle", "click to collapse"],
      ["reasoning", "Thinking through the render model"],
      ["tool", "tool output"],
      ["assistant", "I changed the transcript layout."],
    ],
  )
})

test("projectTranscriptTurnToggleDisplayState ignores invalid targets", () => {
  const current = applyTranscriptDisplayState(baseTurnEntries(), [1])

  assert.equal(projectTranscriptTurnToggleDisplayState(current, null, [1]), null)
  assert.equal(projectTranscriptTurnToggleDisplayState(current, 2, [1]), null)
})

test("projectTranscriptBlobToggleDisplayState projects blob expansion through shared state", () => {
  const current = applyTranscriptDisplayState(baseTurnEntries())
  const projection = projectTranscriptBlobToggleDisplayState(current, 3, [], false)

  assert.equal(projection?.entryId, 3)
  assert.equal(projection?.collapsed, false)
  assert.equal(projection?.entries.find((entry) => entry.id === 3)?.blobCollapsed, false)
  assert.equal(projection?.entryCounter, 5)
  assert.deepEqual(
    projection?.entries.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
    [
      ["user", "Investigate the transcript UI"],
      ["turn_toggle", "click to collapse"],
      ["reasoning", "Thinking through the render model"],
      ["tool", "tool output"],
      ["assistant", "I changed the transcript layout."],
    ],
  )
})

test("projectTranscriptBlobToggleDisplayState ignores invalid entries", () => {
  assert.equal(projectTranscriptBlobToggleDisplayState(baseTurnEntries(), 99, [], false), null)
})

test("applyTranscriptDisplayState uses shared collapsed blob descriptions by default", () => {
  const sourceEntries: TranscriptDisplayEntry[] = [
    { id: 1, role: "user", text: "Investigate the transcript UI", turnId: 1 },
    { id: 2, role: "reasoning", text: "Thinking through the render model", turnId: 1 },
    {
      id: 3,
      role: "tool",
      text: "**bash** · COMPLETED\n\n**Command**\n```bash\n$ git status\n```",
      turnId: 1,
      sourceText: JSON.stringify({
        id: "tool-1",
        tool: "bash",
        status: "completed",
        input: { command: "git status" },
      }),
    },
    { id: 4, role: "assistant", text: "I changed the transcript layout.", turnId: 1 },
  ]
  const entries = applyTranscriptDisplayState(sourceEntries)

  const reasoning = entries.find((entry) => entry.id === 2)
  assert.equal(reasoning?.blobCollapsible, true)
  assert.equal(reasoning?.blobCollapsed, true)
  assert.equal(reasoning?.blobTitle, "reasoning")
  assert.equal(reasoning?.blobSummary, "Thinking through the render model")

  const toolEntry = entries.find((entry) => entry.id === 3)
  assert.equal(toolEntry?.blobCollapsible, true)
  assert.equal(toolEntry?.blobCollapsed, true)
  assert.equal(toolEntry?.blobTitle, "bash · COMPLETED")
  assert.equal(toolEntry?.blobSummary, "$ git status")
})

test("applyTranscriptDisplayState uses supplied blob descriptions for non-history blobs", () => {
  const entries = applyTranscriptDisplayState(baseTurnEntries(), [], null, {
    describeCollapsedBlob: (entry) => ({
      title: `${entry.role} title`,
      summary: `${entry.text} summary`,
    }),
  })

  const reasoning = entries.find((entry) => entry.id === 2)
  const historyBlob = entries.find((entry) => entry.id === 3)
  assert.equal(reasoning?.blobTitle, "reasoning title")
  assert.equal(reasoning?.blobSummary, "Thinking through the render model summary")
  assert.equal(historyBlob?.blobTitle, undefined)
  assert.equal(historyBlob?.blobSummary, undefined)
})

test("applyTranscriptDisplayState applies shared blob defaults by role", () => {
  const sourceEntries: TranscriptDisplayEntry[] = [
    { id: 1, role: "user", text: "Prompt", turnId: 1 },
    { id: 2, role: "reasoning", text: "Reasoning details", turnId: 1 },
    { id: 3, role: "status", text: "Status details", turnId: 1 },
    { id: 4, role: "tool", text: "Tool details", turnId: 1 },
    { id: 5, role: "error", text: "Readable error", turnId: 1 },
    { id: 6, role: "assistant", text: "Assistant response", turnId: 1 },
  ]
  const entries = applyTranscriptDisplayState(sourceEntries)

  assert.deepEqual(
    entries
      .filter((entry) => entry.role !== "turn_toggle")
      .map((entry) => [entry.role, entry.blobCollapsible ?? false, entry.blobCollapsed ?? null, entry.blobTitle ?? null]),
    [
      ["user", false, null, null],
      ["reasoning", true, true, "reasoning"],
      ["status", true, true, "status"],
      ["tool", true, true, "tool"],
      ["error", false, null, null],
      ["assistant", false, null, null],
    ],
  )
})

test("applyTranscriptDisplayState collapses completed turns when requested", () => {
  const entries = applyTranscriptDisplayState(baseTurnEntries(), [1])

  assert.deepEqual(
    entries.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
    [
      ["user", "Investigate the transcript UI"],
      ["turn_toggle", "click to expand"],
      ["assistant", "I changed the transcript layout."],
    ],
  )
})

test("applyTranscriptDisplayState keeps active turns expanded even with stale collapsed ids", () => {
  const entries = applyTranscriptDisplayState(baseTurnEntries(), [1], 1)

  assert.equal(entries.find((entry) => entry.role === "turn_toggle"), undefined)
  assert.deepEqual(
    entries.filter((entry) => !entry.hidden).map((entry) => entry.role),
    ["user", "reasoning", "tool", "assistant"],
  )
})

test("applyTranscriptDisplayState infers active turns from history lifecycle metadata", () => {
  const entries = applyTranscriptDisplayState(
    baseTurnEntries().map((entry) => ({
      ...entry,
      historyTurnCompletedAtMs: null,
    })),
    [1],
  )

  assert.equal(entries.find((entry) => entry.role === "turn_toggle"), undefined)
  assert.deepEqual(
    entries.filter((entry) => !entry.hidden).map((entry) => entry.role),
    ["user", "reasoning", "tool", "assistant"],
  )
})

test("applyTranscriptDisplayState keeps every incomplete history turn expanded", () => {
  const entries = applyTranscriptDisplayState([
    ...baseTurnEntries().map((entry) => ({
      ...entry,
      historyTurnCompletedAtMs: null,
    })),
    { id: 5, role: "user", text: "Second external prompt", turnId: 2, historyTurnCompletedAtMs: null },
    { id: 6, role: "reasoning", text: "Second turn reasoning", turnId: 2, historyTurnCompletedAtMs: null },
    { id: 7, role: "assistant", text: "Second partial assistant", turnId: 2, historyTurnCompletedAtMs: null },
  ], [1, 2])

  assert.equal(entries.find((entry) => entry.role === "turn_toggle"), undefined)
  assert.deepEqual(
    entries.filter((entry) => !entry.hidden).map((entry) => [entry.turnId ?? null, entry.role]),
    [
      [1, "user"],
      [1, "reasoning"],
      [1, "tool"],
      [1, "assistant"],
      [2, "user"],
      [2, "reasoning"],
      [2, "assistant"],
    ],
  )
})

test("collapseLatestTranscriptTurn marks only completed non-trivial turns", () => {
  assert.deepEqual(collapseLatestTranscriptTurn(baseTurnEntries()), [1])
  assert.deepEqual(
    collapseLatestTranscriptTurn([
      ...baseTurnEntries(),
      { id: 5, role: "user", text: "Hi", turnId: 2 },
      { id: 6, role: "assistant", text: "hi", turnId: 2 },
    ], [1]),
    [1],
  )
})

test("collapsed transcript turn state stores turns sorted by agent", () => {
  const current = { "agent-1": [5] }

  const next = updateCollapsedTranscriptTurnState(current, "agent-1", 2, false)

  assert.deepEqual(next, { "agent-1": [2, 5] })
  assert.notEqual(next, current)
})

test("collapsed transcript turn state removes the agent when the last collapsed turn expands", () => {
  const current = { "agent-1": [2] }

  const next = updateCollapsedTranscriptTurnState(current, "agent-1", 2, true)

  assert.deepEqual(next, {})
  assert.notEqual(next, current)
})

test("replaceCollapsedTranscriptTurnIds deduplicates, sorts, and preserves identity when unchanged", () => {
  const current = { "agent-1": [1, 3] }

  assert.deepEqual(replaceCollapsedTranscriptTurnIds(current, "agent-1", [3, 1, 3]), current)
  assert.equal(replaceCollapsedTranscriptTurnIds(current, "agent-1", [1, 3]), current)
})

test("collapseLatestTranscriptTurn skips active history turns", () => {
  assert.deepEqual(
    collapseLatestTranscriptTurn(baseTurnEntries().map((entry) => ({
      ...entry,
      historyTurnCompletedAtMs: null,
    }))),
    [],
  )
})

test("collapseLatestTranscriptTurn skips latest active history turn when earlier turns are also incomplete", () => {
  assert.deepEqual(
    collapseLatestTranscriptTurn([
      ...baseTurnEntries().map((entry) => ({
        ...entry,
        historyTurnCompletedAtMs: null,
      })),
      { id: 5, role: "user", text: "Second external prompt", turnId: 2, historyTurnCompletedAtMs: null },
      { id: 6, role: "reasoning", text: "Second turn reasoning", turnId: 2, historyTurnCompletedAtMs: null },
      { id: 7, role: "assistant", text: "Second partial assistant", turnId: 2, historyTurnCompletedAtMs: null },
    ]),
    [],
  )
})

test("setTranscriptBlobCollapsed preserves turn display state", () => {
  const expandedTurn = applyTranscriptDisplayState(baseTurnEntries())
  const entries = setTranscriptBlobCollapsed(expandedTurn, 3, [], false)

  assert.equal(entries.find((entry) => entry.id === 3)?.blobCollapsed, false)
  assert.equal(entries.find((entry) => entry.role === "turn_toggle")?.text, "click to collapse")
})

test("applyTranscriptDisplayState never marks assistant entries as blob-collapsible", () => {
  const sourceEntries: TranscriptDisplayEntry[] = [
    { id: 1, role: "user", text: "Investigate the transcript UI", turnId: 1 },
    { id: 2, role: "assistant", text: "Draft response", turnId: 1 },
    { id: 3, role: "tool", text: "tool output", turnId: 1 },
    { id: 4, role: "assistant", text: "Final response", turnId: 1 },
  ]
  const entries = applyTranscriptDisplayState(
    sourceEntries,
    [1],
  )

  assert.equal(entries.find((entry) => entry.id === 2)?.blobCollapsible, false)
  assert.equal(entries.find((entry) => entry.id === 4)?.blobCollapsible, false)
})

test("resolveVisibleTurnToggle falls back when a synthetic toggle id changes", () => {
  const entries = applyTranscriptDisplayState(baseTurnEntries())
  const toggle = resolveVisibleTurnToggle(entries, 1, 999)

  assert.equal(toggle?.role, "turn_toggle")
  assert.equal(toggle?.turnId, 1)
  assert.equal(toggle?.toggleMode, "collapse")
})

test("applyTranscriptDisplayState keeps steered prompts out of turn tracking", () => {
  const entries = applyTranscriptDisplayState([
    { id: 1, role: "user", text: "Initial prompt", turnId: 1 },
    { id: 2, role: "assistant", text: "Working...", turnId: 1 },
    { id: 3, role: "user", text: "Steered prompt", turnTracking: "none" },
    { id: 4, role: "assistant", text: "Final response", turnId: 1 },
  ])

  assert.equal(entries.find((entry) => entry.id === 3)?.turnId, undefined)
  assert.deepEqual(
    entries.filter((entry) => entry.role !== "turn_toggle").map((entry) => [entry.id, entry.turnId ?? null]),
    [
      [1, 1],
      [2, 1],
      [3, null],
      [4, 1],
    ],
  )
})
