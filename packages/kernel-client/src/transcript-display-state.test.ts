import assert from "node:assert/strict"
import test from "node:test"

import {
  applyTranscriptDisplayState,
  collapseLatestTranscriptTurn,
  resolveVisibleTurnToggle,
  setTranscriptBlobCollapsed,
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

test("setTranscriptBlobCollapsed preserves turn display state", () => {
  const expandedTurn = applyTranscriptDisplayState(baseTurnEntries())
  const entries = setTranscriptBlobCollapsed(expandedTurn, 3, [], false)

  assert.equal(entries.find((entry) => entry.id === 3)?.blobCollapsed, false)
  assert.equal(entries.find((entry) => entry.role === "turn_toggle")?.text, "click to collapse")
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
