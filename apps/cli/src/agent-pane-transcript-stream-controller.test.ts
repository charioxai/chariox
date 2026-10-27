import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import { createAgentPaneTranscriptStreamController } from "./agent-pane-transcript-stream-controller.js"
import type { ToolTranscriptUpdate } from "./transcript.js"

test("agent pane transcript stream controller appends new provider chunks", () => {
  const harness = streamHarness({
    "agent-1": [entry(1, "user", "prompt", { turnId: 4 })],
  })

  harness.controller.appendProviderChunk("agent-1", "assistant", "hello\r\n")

  assert.equal(harness.setEntries.length, 1)
  assert.deepEqual(harness.setEntries[0]?.entries, [
    entry(1, "user", "prompt", { turnId: 4 }),
    entry(2, "assistant", "hello\n", { turnId: 4 }),
  ])
  assert.equal(harness.committedStreaming.length, 0)
})

test("agent pane transcript stream controller merges assistant chunks", () => {
  const harness = streamHarness({
    "agent-1": [entry(1, "assistant", "hel")],
  })

  harness.controller.appendProviderChunk("agent-1", "assistant", "lo")

  assert.equal(harness.setEntries.length, 0)
  assert.equal(harness.committedStreaming.length, 1)
  assert.equal(harness.committedStreaming[0]?.nextEntries[0]?.text, "hello")
  assert.equal(harness.committedStreaming[0]?.updatedEntryId, 1)
})

test("agent pane transcript stream controller preserves prompt identity while merging chunks", () => {
  const harness = streamHarness()

  harness.controller.appendProviderChunk("agent-1", "assistant", "hel", "reply-1", undefined, {
    promptId: "prompt-1",
    sourceAttachmentId: "attachment-1",
  })
  harness.controller.appendProviderChunk("agent-1", "assistant", "lo", "reply-1", undefined, {
    promptId: "prompt-1",
    sourceAttachmentId: "attachment-1",
  })

  const transcriptEntry = harness.paneEntriesByAgent["agent-1"]?.[0]
  assert.equal(transcriptEntry?.text, "hello")
  assert.equal(transcriptEntry?.promptId, "prompt-1")
  assert.equal(transcriptEntry?.sourceAttachmentId, "attachment-1")
  assert.equal(harness.committedStreaming[0]?.updatedEntryId, 1)
})

test("agent pane transcript stream controller scopes reused merge keys to the current turn", () => {
  const harness = streamHarness({
    "agent-1": [
      entry(1, "user", "first", { turnId: 1 }),
      entry(2, "assistant", "first reply", { turnId: 1, mergeKey: "reply" }),
      entry(3, "user", "second", { turnId: 2 }),
    ],
  })

  harness.controller.appendProviderChunk("agent-1", "assistant", "second reply", "reply")

  assert.equal(harness.committedStreaming.length, 0)
  assert.equal(harness.setEntries.length, 1)
  assert.deepEqual(harness.setEntries[0]?.entries, [
    entry(1, "user", "first", { turnId: 1 }),
    entry(2, "assistant", "first reply", { turnId: 1, mergeKey: "reply" }),
    entry(3, "user", "second", { turnId: 2 }),
    entry(4, "assistant", "second reply", { turnId: 2, mergeKey: "reply" }),
  ])
})

test("agent pane transcript stream controller tracks structured tools", () => {
  const harness = streamHarness()

  harness.controller.appendToolUpdate("agent-1", '{"id":"tool-1","tool":"bash","status":"running"}')
  harness.controller.appendToolUpdate("agent-1", '{"id":"tool-1","tool":"bash","status":"completed","output":"done"}')

  assert.equal(harness.toolState.get("tool-1")?.status, "completed")
  assert.equal(harness.setEntries.length, 1)
  assert.equal(harness.committedStreaming.length, 1)
  assert.equal(harness.committedStreaming[0]?.updatedEntryId, 1)
  assert.equal(harness.committedStreaming[0]?.nextEntries[0]?.mergeKey, "tool-1")
})

function streamHarness(paneEntriesByAgent: Record<string, TranscriptEntry[]> = {}) {
  const harness = {
    paneEntriesByAgent,
    toolState: new Map<string, ToolTranscriptUpdate>(),
    setEntries: [] as Array<{ agentId: string; entries: TranscriptEntry[] }>,
    committedStreaming: [] as Array<{
      agentId: string
      currentEntries: TranscriptEntry[]
      nextEntries: TranscriptEntry[]
      updatedEntryId: number
    }>,
    controller: null as ReturnType<typeof createAgentPaneTranscriptStreamController> | null,
  }
  harness.controller = createAgentPaneTranscriptStreamController({
    currentAgentPaneEntries: (agentId) => harness.paneEntriesByAgent[agentId] ?? [],
    trimLiveAgentPaneEntries: (_agentId, entries) => entries,
    setAgentTranscriptEntries: (agentId, entries) => {
      harness.setEntries.push({ agentId, entries })
      harness.paneEntriesByAgent[agentId] = entries
    },
    commitStreamingAgentPaneEntry: (agentId, currentEntries, nextEntries, updatedEntryId) => {
      harness.committedStreaming.push({
        agentId,
        currentEntries,
        nextEntries,
        updatedEntryId,
      })
      harness.paneEntriesByAgent[agentId] = nextEntries
    },
    toolStateForAgent: () => harness.toolState,
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createAgentPaneTranscriptStreamController>
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
