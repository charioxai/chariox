import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import { createAgentPaneTranscriptInteractionController } from "./agent-pane-transcript-interaction-controller.js"

test("agent pane transcript interaction controller toggles turns", () => {
  const harness = interactionHarness({
    "agent-1": [
      entry(1, "user", "prompt", { turnId: 1 }),
      entry(4, "turn_toggle", "click to expand", { turnId: 1, toggleMode: "expand" }),
      entry(2, "reasoning", "thinking", { turnId: 1, hidden: true, blobCollapsible: true }),
      entry(3, "assistant", "summary", { turnId: 1 }),
    ],
  }, { "agent-1": [1] })

  harness.controller.toggleTurn("agent-1", 1, 4)

  assert.deepEqual(harness.expandedTurnUpdates, [{
    agentId: "agent-1",
    turnId: 1,
    expanded: true,
  }])
  assert.equal(harness.committed.length, 1)
  assert.deepEqual(
    harness.committed[0]?.entries.map((candidate) => [candidate.id, candidate.role, candidate.hidden ?? false, candidate.toggleMode ?? null]),
    [
      [1, "user", false, null],
      [4, "turn_toggle", false, "collapse"],
      [2, "reasoning", false, null],
      [3, "assistant", false, null],
    ],
  )
  assert.equal(harness.reconciled.length, 1)
  assert.equal(harness.focusRetained, 1)
})

test("agent pane transcript interaction controller ignores missing turn toggles", () => {
  const harness = interactionHarness({
    "agent-1": [entry(1, "assistant", "summary", { turnId: 1 })],
  })

  harness.controller.toggleTurn("agent-1", 99)

  assert.equal(harness.committed.length, 0)
  assert.equal(harness.reconciled.length, 0)
  assert.equal(harness.focusRetained, 0)
})

test("agent pane transcript interaction controller toggles blobs", () => {
  const harness = interactionHarness({
    "agent-1": [
      entry(1, "tool", "details", {
        blobCollapsible: true,
        blobCollapsed: true,
      }),
    ],
  })

  harness.controller.toggleBlob("agent-1", 1, false)

  assert.equal(harness.committed[0]?.entries[0]?.blobCollapsed, false)
  assert.equal(harness.reconciled.length, 1)
  assert.equal(harness.focusRetained, 1)
})

test("agent pane transcript interaction controller ignores missing blob entries", () => {
  const harness = interactionHarness({
    "agent-1": [entry(1, "tool", "details")],
  })

  harness.controller.toggleBlob("agent-1", 99, false)

  assert.equal(harness.committed.length, 0)
  assert.equal(harness.reconciled.length, 0)
  assert.equal(harness.focusRetained, 0)
})

function interactionHarness(
  paneEntriesByAgent: Record<string, TranscriptEntry[]>,
  expandedTurnIdsByAgent: Record<string, number[]> = {},
) {
  const harness = {
    paneEntriesByAgent,
    expandedTurnIdsByAgent,
    expandedTurnUpdates: [] as Array<{
      agentId: string | null | undefined
      turnId: number | null | undefined
      expanded: boolean
    }>,
    committed: [] as Array<{ agentId: string; entries: TranscriptEntry[] }>,
    reconciled: [] as Array<{
      agentId: string
      currentEntries: TranscriptEntry[]
      nextEntries: TranscriptEntry[]
    }>,
    focusRetained: 0,
    controller: null as ReturnType<typeof createAgentPaneTranscriptInteractionController> | null,
  }
  harness.controller = createAgentPaneTranscriptInteractionController({
    currentAgentPaneEntries: (agentId) => harness.paneEntriesByAgent[agentId] ?? [],
    expandedTurnIdsForAgent: (agentId) => agentId ? harness.expandedTurnIdsByAgent[agentId] ?? [] : [],
    setExpandedTurnState: (agentId, turnId, expanded) => {
      harness.expandedTurnUpdates.push({ agentId, turnId, expanded })
    },
    commitAgentPaneEntries: (agentId, entries) => {
      harness.committed.push({ agentId, entries })
      harness.paneEntriesByAgent[agentId] = entries
    },
    reconcileMountedAuxiliaryTranscript: (agentId, currentEntries, nextEntries) => {
      harness.reconciled.push({ agentId, currentEntries, nextEntries })
    },
    retainPromptFocus: () => {
      harness.focusRetained += 1
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createAgentPaneTranscriptInteractionController>
  }
}

function entry(
  id: number,
  role: TranscriptEntry["role"],
  text: string,
  overrides: Partial<TranscriptEntry> = {},
): TranscriptEntry {
  return {
    id,
    role,
    text,
    ...overrides,
  }
}
