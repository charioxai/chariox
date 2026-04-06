import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import {
  applyTranscriptDisplayState,
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

test("applyTranscriptDisplayState collapses completed turns down to prompt, toggle, and final summary", () => {
  const entries = applyTranscriptDisplayState(baseTurnEntries())

  assert.deepEqual(
    entries.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]),
    [
      ["user", "Investigate the CLI transcript UI"],
      ["turn_toggle", "click to expand"],
      ["assistant", "I changed the transcript layout."],
    ],
  )

  const toolEntry = entries.find((entry) => entry.id === 3)
  assert.equal(toolEntry?.blobCollapsible, true)
  assert.equal(toolEntry?.blobCollapsed, true)
  assert.equal(toolEntry?.blobTitle, "bash · COMPLETED")
  assert.equal(toolEntry?.blobSummary, "$ git status")
})

test("applyTranscriptDisplayState keeps completed turns expanded when the turn id is in the expanded set", () => {
  const entries = applyTranscriptDisplayState(baseTurnEntries(), [1])

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

  const assistantEntry = entries.find((entry) => entry.id === 4)
  assert.equal(assistantEntry?.blobCollapsible, false)
})

test("setTranscriptBlobCollapsed expands an individual blob without disturbing turn expansion", () => {
  const expandedTurn = applyTranscriptDisplayState(baseTurnEntries(), [1])
  const entries = setTranscriptBlobCollapsed(expandedTurn, 3, [1], false)

  const toolEntry = entries.find((entry) => entry.id === 3)
  assert.equal(toolEntry?.hidden, false)
  assert.equal(toolEntry?.blobCollapsible, true)
  assert.equal(toolEntry?.blobCollapsed, false)
  assert.equal(entries.find((entry) => entry.role === "turn_toggle")?.text, "click to collapse")
})
