import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import { createTranscriptStateController } from "./transcript-state-controller.js"

test("transcript state controller appends entries with ids and active turn ids", () => {
  const harness = transcriptHarness({
    entries: [entry(1, "user", "hello", { turnId: 1 })],
    entryCounter: 1,
    currentTurnId: 2,
  })

  const appended = harness.controller.appendEntry({ role: "assistant", text: "reply" })

  assert.equal(appended?.id, 2)
  assert.equal(appended?.turnId, 2)
  assert.deepEqual(harness.entries.map((candidate) => candidate.id), [1, 2])
  assert.equal(harness.entryCounter, 2)
  assert.equal(harness.persisted.length, 1)
  assert.equal(harness.reconciled.length, 1)
  assert.equal(harness.enforced, 1)
})

test("transcript state controller appends entries using runtime counters", () => {
  const harness = transcriptHarness({
    entries: [entry(1, "user", "hello", { turnId: 1 })],
    entryCounter: 10,
    currentTurnId: 2,
  })

  const appended = harness.controller.appendEntry({ role: "assistant", text: "reply" })

  assert.equal(appended?.id, 11)
  assert.equal(appended?.turnId, 2)
  assert.deepEqual(harness.entries.map((candidate) => candidate.id), [1, 11])
  assert.equal(harness.entryCounter, 11)
})

test("transcript state controller skips consecutive duplicate notices", () => {
  const harness = transcriptHarness({
    entries: [entry(1, "notice", "same", { emphasis: "warning" })],
    entryCounter: 1,
  })

  const appended = harness.controller.appendEntry({ role: "notice", text: "same", emphasis: "warning" })

  assert.equal(appended, null)
  assert.deepEqual(harness.entries.map((candidate) => candidate.id), [1])
  assert.equal(harness.persisted.length, 0)
})

test("transcript state controller toggles turns through shared display state", () => {
  const harness = transcriptHarness({
    entries: [
      entry(1, "user", "prompt", { turnId: 1 }),
      entry(4, "turn_toggle", "click to expand", { turnId: 1, toggleMode: "expand" }),
      entry(2, "reasoning", "thinking", { turnId: 1, hidden: true, blobCollapsible: true }),
      entry(3, "assistant", "summary", { turnId: 1 }),
    ],
    expandedTurnIds: [1],
  })

  harness.controller.toggleTurn(1, 4)

  assert.deepEqual(harness.expandedTurnUpdates, [{
    agentId: "agent-1",
    turnId: 1,
    expanded: true,
  }])
  assert.deepEqual(
    harness.entries.map((candidate) => [candidate.id, candidate.role, candidate.hidden ?? false, candidate.toggleMode ?? null]),
    [
      [1, "user", false, null],
      [4, "turn_toggle", false, "collapse"],
      [2, "reasoning", false, null],
      [3, "assistant", false, null],
    ],
  )
  assert.equal(harness.entryCounter, 4)
  assert.equal(harness.persisted.length, 1)
  assert.equal(harness.reconciled.length, 1)
  assert.equal(harness.focusRetained, 1)
})

test("transcript state controller toggles blob state and preserves focus", () => {
  const harness = transcriptHarness({
    entries: [
      entry(1, "tool", "large", {
        blobCollapsible: true,
        blobCollapsed: true,
      }),
    ],
    entryCounter: 1,
  })

  harness.controller.toggleBlob(1, false)

  assert.equal(harness.entries[0]?.blobCollapsed, false)
  assert.equal(harness.entryCounter, 1)
  assert.equal(harness.focusRetained, 1)
  assert.equal(harness.persisted.length, 1)
  assert.equal(harness.reconciled.length, 1)
})

test("transcript state controller ignores missing blob entries", () => {
  const harness = transcriptHarness({
    entries: [entry(1, "tool", "large")],
  })

  harness.controller.toggleBlob(99, false)

  assert.deepEqual(harness.entries.map((candidate) => candidate.id), [1])
  assert.equal(harness.persisted.length, 0)
  assert.equal(harness.reconciled.length, 0)
  assert.equal(harness.focusRetained, 0)
})

function transcriptHarness(options: {
  entries?: TranscriptEntry[]
  entryCounter?: number
  currentTurnId?: number | null
  expandedTurnIds?: number[]
}) {
  const harness = {
    entries: options.entries ?? [],
    entryCounter: options.entryCounter ?? 0,
    currentTurnId: options.currentTurnId ?? null,
    expandedTurnIds: options.expandedTurnIds ?? [],
    expandedTurnUpdates: [] as Array<{
      agentId: string | null | undefined
      turnId: number
      expanded: boolean
    }>,
    persisted: [] as TranscriptEntry[][],
    reconciled: [] as Array<{ current: TranscriptEntry[]; next: TranscriptEntry[] }>,
    enforced: 0,
    focusRetained: 0,
    controller: null as ReturnType<typeof createTranscriptStateController> | null,
  }
  harness.controller = createTranscriptStateController({
    entries: () => harness.entries,
    setEntries: (nextEntries) => {
      harness.entries = nextEntries
    },
    entryCounter: () => harness.entryCounter,
    setEntryCounter: (value) => {
      harness.entryCounter = value
    },
    currentTurnId: () => harness.currentTurnId,
    visibleTranscriptAgentId: () => "agent-1",
    expandedTurnIdsForAgent: () => harness.expandedTurnIds,
    setExpandedTurnState: (agentId, turnId, expanded) => {
      harness.expandedTurnUpdates.push({ agentId, turnId, expanded })
    },
    persistVisibleTranscriptEntries: (entries) => {
      harness.persisted.push(entries)
    },
    reconcileMountedTranscript: (current, next) => {
      harness.reconciled.push({ current, next })
    },
    retainPromptFocus: () => {
      harness.focusRetained += 1
    },
    enforceTranscriptRetention: () => {
      harness.enforced += 1
    },
  })
  return harness as typeof harness & { controller: ReturnType<typeof createTranscriptStateController> }
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
  } as TranscriptEntry
}
