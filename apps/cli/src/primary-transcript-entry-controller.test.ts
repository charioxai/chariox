import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import { createPrimaryTranscriptEntryController } from "./primary-transcript-entry-controller.js"

test("primary transcript entry controller replaces entries and preserves bottom scroll", () => {
  const pane = scrollbox({ scrollTop: 80, scrollHeight: 100, height: 20 })
  const harness = entryHarness({
    scrollbox: pane,
    visibleAgentId: "agent-1",
    rebuildTranscript: () => {
      pane.scrollHeight = 140
    },
  })

  harness.controller.replaceEntries([
    entry(1, "user", "prompt", { turnId: 3 }),
    entry(2, "assistant", "reply", { turnId: 3 }),
  ])

  assert.equal(harness.toolsCleared, 1)
  assert.deepEqual(harness.entries.map((item) => [item.id, item.role, item.text]), [
    [1, "user", "prompt"],
    [2, "assistant", "reply"],
  ])
  assert.equal(harness.entryCounter, 2)
  assert.equal(harness.currentTurnId, 3)
  assert.equal(harness.nextTurnId, 4)
  assert.equal(harness.mountedAgentId, "agent-1")
  assert.deepEqual(pane.scrollPositions, [{ x: 0, y: 120 }])
  assert.equal(harness.lastScrollTop, 120)
  assert.deepEqual(harness.previewSyncs, [{ agentId: "agent-1", entryCount: 2 }])
})

test("primary transcript entry controller prepends stitched history and restores scroll", async () => {
  const pane = scrollbox({ scrollTop: 12, scrollHeight: 80, height: 20 })
  const harness = entryHarness({
    scrollbox: pane,
    visibleAgentId: "agent-1",
    entries: [
      entry(2, "assistant", "world", {
        historyEntryIndex: 7,
        historyFragmentStart: 6,
        historyFragmentEnd: 11,
        historyTotalChars: 11,
      }),
    ],
  })

  await harness.controller.prependEntries([
    entry(1, "assistant", "hello ", {
      historyEntryIndex: 7,
      historyFragmentStart: 0,
      historyFragmentEnd: 6,
      historyTotalChars: 11,
    }),
  ])

  assert.deepEqual(harness.entries.map((item) => item.text), ["hello world"])
  assert.equal(harness.entryCounter, 2)
  assert.equal(harness.restoreRequests.length, 1)
  assert.equal(harness.restoreRequests[0]?.previousScrollTop, 12)
  assert.equal(harness.restoreRequests[0]?.previousScrollHeight, 80)
  assert.equal(harness.restoreRequests[0]?.previousViewportHeight, 20)
})

function entryHarness(options: {
  scrollbox?: FakeScrollbox
  visibleAgentId?: string | null
  entries?: TranscriptEntry[]
  rebuildTranscript?: () => void
} = {}) {
  const harness = {
    scrollbox: options.scrollbox,
    visibleAgentId: options.visibleAgentId ?? null,
    entries: options.entries ?? [] as TranscriptEntry[],
    entryCounter: 0,
    currentTurnId: null as number | null,
    nextTurnId: 0,
    mountedAgentId: null as string | null,
    lastScrollTop: 0,
    toolsCleared: 0,
    restoreRequests: [] as Array<{
      scrollbox: FakeScrollbox
      previousScrollTop: number
      previousScrollHeight: number
      previousViewportHeight: number
    }>,
    previewSyncs: [] as Array<{ agentId: string | null | undefined; entryCount: number }>,
    controller: null as ReturnType<typeof createPrimaryTranscriptEntryController> | null,
  }
  harness.controller = createPrimaryTranscriptEntryController({
    getScrollbox: () => harness.scrollbox,
    getEntries: () => harness.entries,
    getVisibleTranscriptAgentId: () => harness.visibleAgentId,
    collapsedTurnIdsForAgent: () => [],
    clearToolState: () => {
      harness.toolsCleared += 1
    },
    setEntries: (entries) => {
      harness.entries = entries
    },
    setEntryCounter: (counter) => {
      harness.entryCounter = counter
    },
    setCurrentTurnId: (turnId) => {
      harness.currentTurnId = turnId
    },
    setNextTurnId: (turnId) => {
      harness.nextTurnId = turnId
    },
    setMountedTranscriptAgentId: (agentId) => {
      harness.mountedAgentId = agentId
    },
    setLastScrollTop: (scrollTop) => {
      harness.lastScrollTop = scrollTop
    },
    rebuildTranscript: options.rebuildTranscript ?? (() => {}),
    syncVisibleTranscriptPreview: (agentId, entries) => {
      harness.previewSyncs.push({ agentId, entryCount: entries.length })
    },
    restorePrependedHistory: async (request) => {
      harness.restoreRequests.push({
        scrollbox: request.scrollbox as FakeScrollbox,
        previousScrollTop: request.previousScrollTop,
        previousScrollHeight: request.previousScrollHeight,
        previousViewportHeight: request.previousViewportHeight,
      })
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createPrimaryTranscriptEntryController>
  }
}

type FakeScrollbox = ReturnType<typeof scrollbox>

function scrollbox(options: {
  scrollTop: number
  scrollHeight: number
  height: number
}) {
  return {
    scrollTop: options.scrollTop,
    scrollLeft: 0,
    scrollHeight: options.scrollHeight,
    height: options.height,
    scrollPositions: [] as Array<{ x: number; y: number }>,
    scrollTo(position: { x: number; y: number }) {
      this.scrollLeft = position.x
      this.scrollTop = position.y
      this.scrollPositions.push(position)
    },
    requestRender: () => {},
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
