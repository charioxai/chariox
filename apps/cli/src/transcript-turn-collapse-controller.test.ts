import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import {
  createTranscriptTurnCollapseController,
} from "./transcript-turn-collapse-controller.js"

test("transcript turn collapse controller collapses the latest collapsible turn", () => {
  const harness = collapseHarness()

  const nextTurnIds = harness.controller.collapseLatestTurnForAgent("agent-1", [
    entry(1, "user", "first", 1),
    entry(2, "assistant", "summary", 1),
    entry(3, "user", "second", 2),
    entry(4, "tool", "details", 2),
    entry(5, "assistant", "final", 2),
  ])

  assert.deepEqual(nextTurnIds, [2])
  assert.deepEqual(harness.collapsedTurnIdsByAgent, { "agent-1": [2] })
})

test("transcript turn collapse controller ignores invalid targets", () => {
  const harness = collapseHarness({ "agent-1": [2] })

  harness.controller.setExpandedTurnState(null, 2, true)
  harness.controller.setExpandedTurnState("agent-1", null, true)

  assert.deepEqual(harness.collapsedTurnIdsByAgent, { "agent-1": [2] })
})

test("transcript turn collapse controller stores serialized zero turn ids", () => {
  const harness = collapseHarness({ "agent-1": [0] })

  harness.controller.setExpandedTurnState("agent-1", 0, true)

  assert.deepEqual(harness.collapsedTurnIdsByAgent, {})
})

function collapseHarness(initial: Record<string, number[]> = {}) {
  const harness = {
    collapsedTurnIdsByAgent: initial,
    controller: null as ReturnType<typeof createTranscriptTurnCollapseController> | null,
  }
  harness.controller = createTranscriptTurnCollapseController({
    collapsedTurnIdsForAgent: (agentId) => agentId ? harness.collapsedTurnIdsByAgent[agentId] ?? [] : [],
    updateCollapsedTurnIdsByAgent: (updater) => {
      harness.collapsedTurnIdsByAgent = updater(harness.collapsedTurnIdsByAgent)
    },
  })
  return harness as typeof harness & { controller: ReturnType<typeof createTranscriptTurnCollapseController> }
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
