import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import {
  createTranscriptTurnExpansionController,
  replaceExpandedTurnIds,
  updateExpandedTurnState,
} from "./transcript-turn-expansion-controller.js"

test("updateExpandedTurnState stores collapsed turns sorted by agent", () => {
  const current = { "agent-1": [5] }

  const next = updateExpandedTurnState(current, "agent-1", 2, false)

  assert.deepEqual(next, { "agent-1": [2, 5] })
  assert.notEqual(next, current)
})

test("updateExpandedTurnState removes the agent when the last collapsed turn expands", () => {
  const current = { "agent-1": [2] }

  const next = updateExpandedTurnState(current, "agent-1", 2, true)

  assert.deepEqual(next, {})
  assert.notEqual(next, current)
})

test("replaceExpandedTurnIds deduplicates, sorts, and preserves identity when unchanged", () => {
  const current = { "agent-1": [1, 3] }

  assert.deepEqual(replaceExpandedTurnIds(current, "agent-1", [3, 1, 3]), current)
  assert.equal(replaceExpandedTurnIds(current, "agent-1", [1, 3]), current)
})

test("transcript turn expansion controller collapses the latest collapsible turn", () => {
  const harness = expansionHarness()

  const nextTurnIds = harness.controller.collapseLatestTurnForAgent("agent-1", [
    entry(1, "user", "first", 1),
    entry(2, "assistant", "summary", 1),
    entry(3, "user", "second", 2),
    entry(4, "tool", "details", 2),
    entry(5, "assistant", "final", 2),
  ])

  assert.deepEqual(nextTurnIds, [2])
  assert.deepEqual(harness.expandedTurnIdsByAgent, { "agent-1": [2] })
})

test("transcript turn expansion controller ignores invalid targets", () => {
  const harness = expansionHarness({ "agent-1": [2] })

  harness.controller.setExpandedTurnState(null, 2, true)
  harness.controller.setExpandedTurnState("agent-1", null, true)

  assert.deepEqual(harness.expandedTurnIdsByAgent, { "agent-1": [2] })
})

function expansionHarness(initial: Record<string, number[]> = {}) {
  const harness = {
    expandedTurnIdsByAgent: initial,
    controller: null as ReturnType<typeof createTranscriptTurnExpansionController> | null,
  }
  harness.controller = createTranscriptTurnExpansionController({
    expandedTurnIdsForAgent: (agentId) => agentId ? harness.expandedTurnIdsByAgent[agentId] ?? [] : [],
    updateExpandedTurnIdsByAgent: (updater) => {
      harness.expandedTurnIdsByAgent = updater(harness.expandedTurnIdsByAgent)
    },
  })
  return harness as typeof harness & { controller: ReturnType<typeof createTranscriptTurnExpansionController> }
}

function entry(
  id: number,
  role: TranscriptEntry["role"],
  text: string,
  turnId: number,
): TranscriptEntry {
  return {
    id,
    role,
    text,
    turnId,
  }
}
