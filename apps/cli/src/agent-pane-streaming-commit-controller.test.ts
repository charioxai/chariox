import assert from "node:assert/strict"
import test from "node:test"

import { createAgentPaneStreamingCommitController } from "./agent-pane-streaming-commit-controller.js"
import type { TranscriptEntry } from "./cli-types.js"

test("agent pane streaming commit updates primary split transcript", () => {
  const harness = streamingCommitHarness({
    split: true,
    primaryAgentId: "agent-1",
  })

  harness.controller.commitStreamingEntry("agent-1", [entry(1, "assistant", "old")], [
    entry(1, "assistant", "new"),
  ], 1)

  assert.deepEqual(harness.committed, [{
    agentId: "agent-1",
    entries: [displayEntry(1, "assistant", "new")],
  }])
  assert.deepEqual(harness.replaced, [{
    agentId: "agent-1",
    entries: [displayEntry(1, "assistant", "new")],
  }])
  assert.deepEqual(harness.updatedAuxiliary, [])
  assert.deepEqual(harness.reconciledAuxiliary, [])
})

test("agent pane streaming commit updates visible auxiliary entry", () => {
  const harness = streamingCommitHarness({
    split: true,
    auxiliaryAgentIds: ["agent-2"],
  })

  harness.controller.commitStreamingEntry("agent-2", [entry(1, "assistant", "old")], [
    entry(1, "assistant", "new"),
  ], 1)

  assert.deepEqual(harness.committed, [{
    agentId: "agent-2",
    entries: [displayEntry(1, "assistant", "new")],
  }])
  assert.deepEqual(harness.updatedAuxiliary, [{
    agentId: "agent-2",
    entry: displayEntry(1, "assistant", "new"),
  }])
  assert.deepEqual(harness.reconciledAuxiliary, [])
})

test("agent pane streaming commit reconciles auxiliary pane when updated entry is trimmed", () => {
  const harness = streamingCommitHarness({
    split: true,
    auxiliaryAgentIds: ["agent-2"],
    trim: (_agentId, entries) => entries.filter((entry) => entry.id !== 1),
  })

  harness.controller.commitStreamingEntry("agent-2", [entry(1, "assistant", "old")], [
    entry(1, "assistant", "new"),
    entry(2, "assistant", "kept"),
  ], 1)

  assert.deepEqual(harness.updatedAuxiliary, [])
  assert.deepEqual(harness.reconciledAuxiliary, [{
    agentId: "agent-2",
    currentEntries: [entry(1, "assistant", "old")],
    nextEntries: [displayEntry(2, "assistant", "kept")],
  }])
})

function streamingCommitHarness(options: {
  split?: boolean
  primaryAgentId?: string | null
  auxiliaryAgentIds?: string[]
  trim?: (agentId: string, entries: TranscriptEntry[]) => TranscriptEntry[]
} = {}) {
  const harness = {
    committed: [] as Array<{ agentId: string; entries: TranscriptEntry[] }>,
    replaced: [] as Array<{ agentId: string; entries: TranscriptEntry[] }>,
    updatedAuxiliary: [] as Array<{ agentId: string; entry: TranscriptEntry }>,
    reconciledAuxiliary: [] as Array<{
      agentId: string
      currentEntries: TranscriptEntry[]
      nextEntries: TranscriptEntry[]
    }>,
    controller: null as ReturnType<typeof createAgentPaneStreamingCommitController> | null,
  }
  harness.controller = createAgentPaneStreamingCommitController({
    trimLiveAgentPaneEntries: options.trim ?? ((_agentId, entries) => entries),
    expandedTurnIdsForAgent: () => [],
    commitAgentPaneEntries: (agentId, entries) => {
      harness.committed.push({ agentId, entries })
    },
    splitAgentResponseMode: () => Boolean(options.split),
    getResponsePrimaryAgentId: () => options.primaryAgentId ?? null,
    replaceTranscriptEntries: (entries, agentId) => {
      harness.replaced.push({ agentId, entries })
    },
    visibleAuxiliaryAgentIds: () => options.auxiliaryAgentIds ?? [],
    updateAuxiliaryTranscriptEntry: (agentId, entry) => {
      harness.updatedAuxiliary.push({ agentId, entry })
    },
    reconcileMountedAuxiliaryTranscript: (agentId, currentEntries, nextEntries) => {
      harness.reconciledAuxiliary.push({ agentId, currentEntries, nextEntries })
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createAgentPaneStreamingCommitController>
  }
}

function entry(id: number, role: TranscriptEntry["role"], text: string): TranscriptEntry {
  return {
    id,
    role,
    text,
  }
}

function displayEntry(id: number, role: TranscriptEntry["role"], text: string): TranscriptEntry {
  return {
    ...entry(id, role, text),
    hidden: false,
  }
}
