import assert from "node:assert/strict"
import test from "node:test"

import { createAssistantMessageCompletionController } from "./assistant-message-completion-controller.js"
import type { TranscriptEntry } from "./cli-types.js"

test("assistant completion controller finalizes the visible transcript turn", () => {
  const harness = completionHarness({
    entries: turnEntries(),
    collapsedTurnIdsByAgent: { "agent-1": [1, 3] },
  })

  harness.controller.markCompleted("agent-1")

  assert.deepEqual(harness.collapsedTurnIdsByAgent["agent-1"], [1, 3])
  assert.deepEqual(harness.setEntryBatches.at(-1)?.filter((entry) => !entry.hidden).map((entry) => [entry.role, entry.text]), [
    ["user", "Investigate"],
    ["turn_toggle", "click to expand"],
    ["assistant", "done"],
  ])
  assert.deepEqual(harness.setEntryBatches.at(-1)?.map((entry) => [entry.role, entry.text, entry.hidden ?? false]), [
    ["user", "Investigate", false],
    ["turn_toggle", "click to expand", false],
    ["tool", "tool output", true],
    ["assistant", "done", false],
  ])
  assert.equal(harness.entryCounter, 4)
  assert.equal(harness.persistedEntries.at(-1)?.length, 4)
  assert.deepEqual(harness.reconciled, [{ current: ["Investigate", "tool output", "done"], next: ["Investigate", "click to expand", "tool output", "done"] }])
  assert.deepEqual(harness.calls.slice(-3), ["busy:agent-1", "turn:confirm", "turn:schedule"])
})

test("assistant completion controller finalizes an off-focus split pane", () => {
  const harness = completionHarness({
    split: true,
    visibleAgentId: "agent-1",
    entries: turnEntries("visible"),
    paneEntries: {
      "agent-2": turnEntries("auxiliary"),
    },
    collapsedTurnIdsByAgent: { "agent-2": [1] },
  })

  harness.controller.markCompleted("agent-2")

  assert.deepEqual(harness.collapsedTurnIdsByAgent["agent-2"], [1])
  assert.deepEqual(harness.agentTranscriptEntries, [{
    agentId: "agent-2",
    entries: ["auxiliary", "click to expand", "tool output", "done"],
    turnIds: [1],
  }])
  assert.deepEqual(harness.setEntryBatches, [])
  assert.deepEqual(harness.calls.slice(-3), ["busy:agent-2", "turn:confirm", "turn:schedule"])
})

test("assistant completion controller still clears busy state and confirms without an agent", () => {
  const harness = completionHarness({
    visibleAgentId: null,
    entries: [],
  })

  harness.controller.markCompleted(null)

  assert.deepEqual(harness.collapsedTurnIdsByAgent, {})
  assert.deepEqual(harness.setEntryBatches, [])
  assert.deepEqual(harness.calls, ["busy:null", "turn:confirm", "turn:schedule"])
})

function completionHarness(options: {
  split?: boolean
  visibleAgentId?: string | null
  entries?: TranscriptEntry[]
  paneEntries?: Record<string, TranscriptEntry[]>
  collapsedTurnIdsByAgent?: Record<string, number[]>
} = {}) {
  const harness = {
    split: options.split ?? false,
    visibleAgentId: options.visibleAgentId === undefined ? "agent-1" : options.visibleAgentId,
    entries: options.entries ?? [],
    paneEntries: options.paneEntries ?? {},
    collapsedTurnIdsByAgent: options.collapsedTurnIdsByAgent ?? {},
    setEntryBatches: [] as TranscriptEntry[][],
    persistedEntries: [] as TranscriptEntry[][],
    reconciled: [] as Array<{ current: string[]; next: string[] }>,
    agentTranscriptEntries: [] as Array<{ agentId: string; entries: string[]; turnIds?: readonly number[] }>,
    entryCounter: 0,
    calls: [] as string[],
    controller: null as ReturnType<typeof createAssistantMessageCompletionController> | null,
  }
  harness.controller = createAssistantMessageCompletionController({
    entries: () => harness.entries,
    visibleTranscriptAgentId: () => harness.visibleAgentId,
    splitAgentResponseMode: () => harness.split,
    currentAgentPaneEntries: (agentId) => harness.paneEntries[agentId] ?? [],
    collapsedTurnIdsForAgent: (agentId) => agentId ? harness.collapsedTurnIdsByAgent[agentId] ?? [] : [],
    setCollapsedTurnIdsForAgent: (agentId, turnIds) => {
      harness.collapsedTurnIdsByAgent[agentId] = turnIds
    },
    setEntries: (entries) => {
      harness.entries = entries
      harness.setEntryBatches.push(entries)
    },
    setEntryCounter: (value) => {
      harness.entryCounter = value
    },
    persistVisibleTranscriptEntries: (entries) => {
      harness.persistedEntries.push(entries)
    },
    reconcileMountedTranscript: (currentEntries, nextEntries) => {
      harness.reconciled.push({
        current: currentEntries.map((entry) => entry.text),
        next: nextEntries.map((entry) => entry.text),
      })
    },
    setAgentTranscriptEntries: (agentId, entries, turnIds) => {
      const record: { agentId: string; entries: string[]; turnIds?: readonly number[] } = {
        agentId,
        entries: entries.map((entry) => entry.text),
      }
      if (turnIds !== undefined) {
        record.turnIds = turnIds
      }
      harness.agentTranscriptEntries.push(record)
    },
    clearAgentBusy: (agentId) => {
      harness.calls.push(`busy:${agentId ?? "null"}`)
    },
    confirmTurnCompletion: () => {
      harness.calls.push("turn:confirm")
    },
    maybeScheduleConfirmedTurnCompletion: () => {
      harness.calls.push("turn:schedule")
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createAssistantMessageCompletionController>
  }
}

function turnEntries(promptText = "Investigate"): TranscriptEntry[] {
  return [
    { id: 1, role: "user", text: promptText, turnId: 1 },
    { id: 2, role: "tool", text: "tool output", turnId: 1 },
    { id: 3, role: "assistant", text: "done", turnId: 1 },
  ]
}
