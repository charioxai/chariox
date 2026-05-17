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

function transcriptHarness(options: {
  entries?: TranscriptEntry[]
  entryCounter?: number
  currentTurnId?: number | null
}) {
  const harness = {
    entries: options.entries ?? [],
    entryCounter: options.entryCounter ?? 0,
    currentTurnId: options.currentTurnId ?? null,
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
    expandedTurnIdsForAgent: () => [],
    setExpandedTurnState: () => {},
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
