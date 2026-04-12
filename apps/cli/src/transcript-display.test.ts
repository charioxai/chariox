import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import {
  applyTranscriptDisplayState,
  resolveVisibleTurnToggle,
  setTranscriptBlobCollapsed,
} from "./transcript-display.js"

function baseTurnEntries(): TranscriptEntry[] {
  return [
    { id: 1, role: "user", text: "Investigate the CLI transcript UI", turnId: 1 },
    { id: 2, role: "reasoning", text: "Thinking through the render model", turnId: 1 },
    {
      id: 3,
      role: "tool",
      text: "**bash** · COMPLETED\n\n**Command**\n```bash\n$ git status\n```",
      turnId: 1,
      mergeKey: "tool-1",
      sourceText: JSON.stringify({
        id: "tool-1",
        tool: "bash",
        status: "completed",
        input: { command: "git status" },
      }),
    },
    { id: 4, role: "assistant", text: "I changed the transcript layout.", turnId: 1 },
  ]
}

test("applyTranscriptDisplayState keeps completed turns expanded by default", () => {
  const entries = applyTranscriptDisplayState(baseTurnEntries())

  assert.deepEqual(
    entries.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
    [
      ["user", "Investigate the CLI transcript UI"],
      ["turn_toggle", "click to collapse"],
      ["reasoning", "Thinking through the render model"],
      ["tool", "**bash** · COMPLETED\n\n**Command**\n```bash\n$ git status\n```"],
      ["assistant", "I changed the transcript layout."],
    ],
  )

  const toolEntry = entries.find((entry) => entry.id === 3)
  assert.equal(toolEntry?.blobCollapsible, true)
  assert.equal(toolEntry?.blobCollapsed, true)
  assert.equal(toolEntry?.blobTitle, "bash · COMPLETED")
  assert.equal(toolEntry?.blobSummary, "$ git status")
})

test("applyTranscriptDisplayState collapses completed turns when the turn id is in the collapsed set", () => {
  const entries = applyTranscriptDisplayState(baseTurnEntries(), [1])

  assert.deepEqual(
    entries.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
    [
      ["user", "Investigate the CLI transcript UI"],
      ["turn_toggle", "click to expand"],
      ["assistant", "I changed the transcript layout."],
    ],
  )

  const assistantEntry = entries.find((entry) => entry.id === 4)
  assert.equal(assistantEntry?.blobCollapsible, false)
})

test("applyTranscriptDisplayState keeps the active turn expanded until completion is confirmed", () => {
  const entries = applyTranscriptDisplayState(baseTurnEntries(), [], 1)

  assert.deepEqual(
    entries.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
    [
      ["user", "Investigate the CLI transcript UI"],
      ["reasoning", "Thinking through the render model"],
      ["tool", "**bash** · COMPLETED\n\n**Command**\n```bash\n$ git status\n```"],
      ["assistant", "I changed the transcript layout."],
    ],
  )

  assert.equal(entries.find((entry) => entry.role === "turn_toggle"), undefined)
})

test("setTranscriptBlobCollapsed expands an individual blob without disturbing turn expansion", () => {
  const expandedTurn = applyTranscriptDisplayState(baseTurnEntries())
  const entries = setTranscriptBlobCollapsed(expandedTurn, 3, [], false)

  const toolEntry = entries.find((entry) => entry.id === 3)
  assert.equal(toolEntry?.hidden, false)
  assert.equal(toolEntry?.blobCollapsible, true)
  assert.equal(toolEntry?.blobCollapsed, false)
  assert.equal(entries.find((entry) => entry.role === "turn_toggle")?.text, "click to collapse")
})

test("resolveVisibleTurnToggle falls back when a synthetic toggle id was regenerated", () => {
  const entries = applyTranscriptDisplayState(baseTurnEntries())
  const toggle = resolveVisibleTurnToggle(entries, 1, 999)

  assert.equal(toggle?.role, "turn_toggle")
  assert.equal(toggle?.turnId, 1)
  assert.equal(toggle?.toggleMode, "collapse")
})

test("applyTranscriptDisplayState never marks assistant entries as blob-collapsible", () => {
  const entries = applyTranscriptDisplayState(
    [
      { id: 1, role: "user", text: "Investigate the CLI transcript UI", turnId: 1 },
      { id: 2, role: "assistant", text: "Draft response", turnId: 1 },
      { id: 3, role: "tool", text: "tool output", turnId: 1 },
      { id: 4, role: "assistant", text: "Final response", turnId: 1 },
    ],
    [1],
  )

  assert.equal(entries.find((entry) => entry.id === 2)?.blobCollapsible, false)
  assert.equal(entries.find((entry) => entry.id === 4)?.blobCollapsible, false)
})
