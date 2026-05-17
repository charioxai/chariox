import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import { createAgentPaneTranscriptEntryController } from "./agent-pane-transcript-entry-controller.js"

test("agent pane transcript entry controller syncs visible previews", () => {
  const harness = entryHarness({
    visibleAgentId: "agent-1",
    visibleEntries: [entry(1, "assistant", "hello")],
  })

  harness.controller.syncVisibleTranscriptPreview()

  assert.equal(harness.previews["agent-1"], "Asst: hello")
})

test("agent pane transcript entry controller appends preview lines", () => {
  const harness = entryHarness({
    previews: { "agent-1": "first" },
  })

  harness.controller.appendPreview("agent-1", "\r\nsecond\r")
  harness.controller.appendPreview("agent-1", "  \n")

  assert.equal(harness.previews["agent-1"], "first\nsecond")
})

test("agent pane transcript entry controller detects trailing user prompts", () => {
  const harness = entryHarness({
    paneEntriesByAgent: {
      "agent-1": [entry(1, "user", "hello\n")],
    },
  })

  assert.equal(harness.controller.hasTrailingUserPrompt("agent-1", "hello"), true)
  assert.equal(harness.controller.hasTrailingUserPrompt("agent-1", "other"), false)
})

test("agent pane transcript entry controller appends entries with active turn ids", () => {
  const harness = entryHarness({
    paneEntriesByAgent: {
      "agent-1": [entry(1, "user", "prompt", { turnId: 9 })],
    },
  })

  harness.controller.appendEntry("agent-1", { role: "assistant", text: "reply" })

  assert.deepEqual(harness.setEntries[0]?.entries, [
    entry(1, "user", "prompt", { turnId: 9 }),
    entry(2, "assistant", "reply", { turnId: 9 }),
  ])
})

test("agent pane transcript entry controller skips duplicate notices", () => {
  const harness = entryHarness({
    paneEntriesByAgent: {
      "agent-1": [entry(1, "notice", "same", { emphasis: "warning" })],
    },
  })

  harness.controller.appendEntry("agent-1", { role: "notice", text: "same", emphasis: "warning" })

  assert.equal(harness.setEntries.length, 0)
})

function entryHarness(options: {
  paneEntriesByAgent?: Record<string, TranscriptEntry[]>
  visibleAgentId?: string | null
  visibleEntries?: TranscriptEntry[]
  previews?: Record<string, string>
} = {}) {
  const harness = {
    paneEntriesByAgent: options.paneEntriesByAgent ?? {},
    visibleAgentId: options.visibleAgentId,
    visibleEntries: options.visibleEntries ?? [],
    previews: options.previews ?? {} as Record<string, string>,
    setEntries: [] as Array<{
      agentId: string
      entries: TranscriptEntry[]
      turnIds: readonly number[] | undefined
    }>,
    controller: null as ReturnType<typeof createAgentPaneTranscriptEntryController> | null,
  }
  harness.controller = createAgentPaneTranscriptEntryController({
    currentAgentPaneEntries: (agentId) => harness.paneEntriesByAgent[agentId] ?? [],
    visibleTranscriptAgentId: () => harness.visibleAgentId,
    visibleTranscriptEntries: () => harness.visibleEntries,
    expandedTurnIdsForAgent: () => [],
    setAgentPanePreview: (agentId, text) => {
      harness.previews[agentId] = text
    },
    updateAgentPanePreviews: (updater) => {
      harness.previews = updater(harness.previews)
    },
    trimLiveAgentPaneEntries: (_agentId, entries) => entries,
    setAgentTranscriptEntries: (agentId, entries, turnIds) => {
      harness.setEntries.push({ agentId, entries, turnIds })
      harness.paneEntriesByAgent[agentId] = entries
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createAgentPaneTranscriptEntryController>
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
