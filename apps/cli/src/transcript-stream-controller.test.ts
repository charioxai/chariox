import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import { createTranscriptStreamController } from "./transcript-stream-controller.js"
import type { ToolTranscriptUpdate } from "./transcript.js"

test("transcript stream controller appends provider chunks with active turn state", () => {
  const harness = streamHarness({
    entryCounter: 2,
    currentTurnId: 7,
  })

  harness.controller.appendProviderChunk("assistant", "hello\r\nworld")

  assert.deepEqual(harness.entries, [
    entry(3, "assistant", "hello\nworld", { turnId: 7 }),
  ])
  assert.equal(harness.cancelled, 1)
  assert.deepEqual(harness.working, [true])
  assert.deepEqual(harness.submitting, [false])
  assert.equal(harness.persisted.length, 1)
  assert.equal(harness.reconciled.length, 1)
  assert.equal(harness.enforced, 1)
  assert.equal(harness.scheduledCompletions, 1)
})

test("transcript stream controller merges assistant chunks in place", () => {
  const harness = streamHarness({
    entries: [entry(1, "assistant", "hel")],
    entryCounter: 1,
  })

  harness.controller.appendProviderChunk("assistant", "lo")

  assert.equal(harness.entries[0]?.text, "hello")
  assert.deepEqual(harness.updatedEntries, [{ id: 1, text: "hello", sourceText: undefined }])
  assert.equal(harness.reconciled.length, 0)
  assert.equal(harness.persisted.length, 1)
})

test("transcript stream controller carries prompt identity through new and merged chunks", () => {
  const harness = streamHarness({
    entryCounter: 1,
    currentTurnId: 2,
  })

  harness.controller.appendProviderChunk("assistant", "hel", "reply-1", undefined, {
    promptId: "prompt-1",
    sourceAttachmentId: "attachment-1",
  })
  harness.controller.appendProviderChunk("assistant", "lo", "reply-1", undefined, {
    promptId: "prompt-1",
    sourceAttachmentId: "attachment-1",
  })

  assert.equal(harness.entries.length, 1)
  assert.equal(harness.entries[0]?.text, "hello")
  assert.equal(harness.entries[0]?.promptId, "prompt-1")
  assert.equal(harness.entries[0]?.sourceAttachmentId, "attachment-1")
  assert.equal(harness.updatedEntries.at(-1)?.id, 2)
})

test("transcript stream controller tracks and clears active tool labels", () => {
  const harness = streamHarness()

  harness.controller.appendToolUpdate('{"id":"tool-1","tool":"bash","status":"running"}')

  assert.equal(harness.tools.get("tool-1")?.status, "running")
  assert.equal(harness.activeToolLabels.get("tool-1"), "bashing")
  assert.equal(harness.activitySynced, 1)
  assert.equal(harness.entries[0]?.role, "tool")
  assert.equal(harness.entries[0]?.mergeKey, "tool-1")

  harness.controller.appendToolUpdate('{"id":"tool-1","tool":"bash","status":"completed","output":"done"}')

  assert.equal(harness.tools.get("tool-1")?.status, "completed")
  assert.equal(harness.activeToolLabels.has("tool-1"), false)
  assert.equal(harness.entries.length, 1)
  assert.equal(harness.updatedEntries.at(-1)?.id, 1)
  assert.equal(harness.activitySynced, 2)
})

function streamHarness(options: {
  entries?: TranscriptEntry[]
  entryCounter?: number
  currentTurnId?: number | null
} = {}) {
  const harness = {
    entries: options.entries ?? [],
    entryCounter: options.entryCounter ?? 0,
    currentTurnId: options.currentTurnId ?? null,
    tools: new Map<string, ToolTranscriptUpdate>(),
    activeToolLabels: new Map<string, string>(),
    cancelled: 0,
    working: [] as boolean[],
    submitting: [] as boolean[],
    sessionChromeUpdates: 0,
    activitySynced: 0,
    persisted: [] as TranscriptEntry[][],
    reconciled: [] as Array<{ current: TranscriptEntry[]; next: TranscriptEntry[] }>,
    updatedEntries: [] as Array<{ id: number; text: string; sourceText: string | undefined }>,
    logged: [] as Array<{
      role: TranscriptEntry["role"]
      text: string
      merged: boolean
      mergeKey: string | undefined
    }>,
    enforced: 0,
    scheduledCompletions: 0,
    controller: null as ReturnType<typeof createTranscriptStreamController> | null,
  }
  harness.controller = createTranscriptStreamController({
    entries: () => harness.entries,
    setEntries: (nextEntries) => {
      harness.entries = nextEntries
    },
    entryCounter: () => harness.entryCounter,
    currentTurnId: () => harness.currentTurnId,
    tools: harness.tools,
    activeToolLabels: harness.activeToolLabels,
    cancelPendingTurnCompletion: () => {
      harness.cancelled += 1
    },
    setWorking: (value) => {
      harness.working.push(value)
    },
    setSubmitting: (value) => {
      harness.submitting.push(value)
    },
    updateSessionChrome: () => {
      harness.sessionChromeUpdates += 1
    },
    syncVisibleActivityLabel: () => {
      harness.activitySynced += 1
    },
    applyVisibleTranscriptState: (nextEntries) => {
      harness.entries = nextEntries
      harness.entryCounter = nextEntries.reduce((max, candidate) => Math.max(max, candidate.id), 0)
      return nextEntries
    },
    persistVisibleTranscriptEntries: (nextEntries) => {
      harness.persisted.push(nextEntries)
    },
    reconcileMountedTranscript: (current, next) => {
      harness.reconciled.push({ current, next })
    },
    updateTranscriptEntry: (id, text, sourceText) => {
      harness.updatedEntries.push({ id, text, sourceText })
    },
    logVisibleTranscriptOutput: (role, text, merged, mergeKey) => {
      harness.logged.push({ role, text, merged, mergeKey })
    },
    enforceTranscriptRetention: () => {
      harness.enforced += 1
    },
    maybeScheduleConfirmedTurnCompletion: () => {
      harness.scheduledCompletions += 1
    },
  })
  return harness as typeof harness & { controller: ReturnType<typeof createTranscriptStreamController> }
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
