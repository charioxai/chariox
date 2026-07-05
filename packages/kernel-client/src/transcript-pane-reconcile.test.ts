import assert from "node:assert/strict"
import test from "node:test"

import {
  reconcileMountedTranscriptPane,
  transcriptEntriesShareMountedPrefix,
  type TranscriptPaneEntry,
  type TranscriptPaneRenderable,
} from "./transcript-pane-reconcile.js"

function entry(id: number, role: string, text: string, turnId = 1): TranscriptPaneEntry {
  return { id, role, text, turnId }
}

test("transcriptEntriesShareMountedPrefix compares toggles by state and content", () => {
  assert.equal(
    transcriptEntriesShareMountedPrefix(
      { ...entry(1, "turn_toggle", "click to collapse"), toggleMode: "collapse" },
      { ...entry(2, "turn_toggle", "click to collapse"), toggleMode: "collapse" },
    ),
    true,
  )
  assert.equal(
    transcriptEntriesShareMountedPrefix(
      { ...entry(1, "turn_toggle", "click to collapse"), toggleMode: "collapse" },
      { ...entry(1, "turn_toggle", "click to expand"), toggleMode: "expand" },
    ),
    false,
  )
  assert.equal(transcriptEntriesShareMountedPrefix(entry(1, "assistant", "a"), entry(2, "assistant", "a")), false)
})

test("reconcileMountedTranscriptPane preserves mounted prefix and repaints only changed suffix", () => {
  const harness = createHarness([
    [1, entry(1, "user", "prompt")],
    [2, { ...entry(2, "turn_toggle", "click to collapse"), toggleMode: "collapse" }],
    [3, entry(3, "assistant", "summary")],
  ])
  const currentEntries = [
    entry(1, "user", "prompt"),
    { ...entry(2, "turn_toggle", "click to collapse"), toggleMode: "collapse" as const },
    entry(3, "assistant", "summary"),
  ]
  const nextEntries = [
    entry(1, "user", "prompt"),
    { ...entry(4, "turn_toggle", "click to expand"), toggleMode: "expand" as const },
    entry(5, "assistant", "summary"),
  ]

  reconcileMountedTranscriptPane({
    scrollbox: harness.scrollbox,
    currentEntries,
    nextEntries,
    renderables: harness.renderables,
    clampScrollTop: (value) => value,
    rebuild: harness.rebuild,
    mountEntry: harness.mountEntry,
  })

  assert.deepEqual(harness.removed, ["w2", "w3"])
  assert.deepEqual(harness.destroyed, ["w2", "w3"])
  assert.deepEqual(harness.mounted, [4, 5])
  assert.equal(harness.renderables.has(1), true)
  assert.equal(harness.renderables.has(2), false)
  assert.equal(harness.renderables.has(3), false)
  assert.equal(harness.scrollbox.scrollTop, 12)
  assert.equal(harness.scrollbox.requestRenderCount, 1)
})

test("reconcileMountedTranscriptPane remounts turn toggles when mode changes with the same id", () => {
  const harness = createHarness([
    [1, entry(1, "user", "prompt")],
    [5, { ...entry(5, "turn_toggle", "click to collapse"), toggleMode: "collapse" }],
  ])

  reconcileMountedTranscriptPane({
    scrollbox: harness.scrollbox,
    currentEntries: [
      entry(1, "user", "prompt"),
      { ...entry(5, "turn_toggle", "click to collapse"), toggleMode: "collapse" as const },
    ],
    nextEntries: [
      entry(1, "user", "prompt"),
      { ...entry(5, "turn_toggle", "click to expand"), toggleMode: "expand" as const },
    ],
    renderables: harness.renderables,
    clampScrollTop: (value) => value,
    rebuild: harness.rebuild,
    mountEntry: harness.mountEntry,
  })

  assert.deepEqual(harness.removed, ["w5"])
  assert.deepEqual(harness.destroyed, ["w5"])
  assert.deepEqual(harness.mounted, [5])
  assert.equal(harness.renderables.get(5)?.entry.toggleMode, "expand")
  assert.equal(harness.scrollbox.requestRenderCount, 1)
})

test("reconcileMountedTranscriptPane ignores hidden and deferred entries when diffing mounted children", () => {
  const harness = createHarness([
    [1, entry(1, "user", "prompt")],
    [2, entry(2, "assistant", "visible")],
  ])

  reconcileMountedTranscriptPane({
    scrollbox: harness.scrollbox,
    currentEntries: [
      entry(1, "user", "prompt"),
      { ...entry(99, "reasoning", "old hidden"), hidden: true },
      entry(2, "assistant", "visible"),
    ],
    nextEntries: [
      entry(1, "user", "prompt"),
      { ...entry(100, "reasoning", "new deferred"), historyDeferred: true },
      entry(2, "assistant", "visible"),
    ],
    renderables: harness.renderables,
    clampScrollTop: (value) => value,
    rebuild: harness.rebuild,
    mountEntry: harness.mountEntry,
  })

  assert.deepEqual(harness.removed, [])
  assert.deepEqual(harness.mounted, [])
  assert.equal(harness.scrollbox.requestRenderCount, 1)
})

test("reconcileMountedTranscriptPane rebuilds when scrollbox is absent or next entries are empty", () => {
  let rebuilds = 0
  reconcileMountedTranscriptPane({
    scrollbox: undefined,
    currentEntries: [],
    nextEntries: [entry(1, "assistant", "next")],
    renderables: new Map(),
    clampScrollTop: (value) => value,
    rebuild: () => {
      rebuilds += 1
    },
    mountEntry: () => {},
  })
  reconcileMountedTranscriptPane({
    scrollbox: createHarness([]).scrollbox,
    currentEntries: [entry(1, "assistant", "current")],
    nextEntries: [],
    renderables: new Map(),
    clampScrollTop: (value) => value,
    rebuild: () => {
      rebuilds += 1
    },
    mountEntry: () => {},
  })

  assert.equal(rebuilds, 2)
})

function createHarness(initialEntries: Array<[number, TranscriptPaneEntry]>) {
  const removed: string[] = []
  const destroyed: string[] = []
  const mounted: number[] = []
  const renderables = new Map<number, TranscriptPaneRenderable<TranscriptPaneEntry>>(
    initialEntries.map(([id, initialEntry]) => [
      id,
      {
        entry: initialEntry,
        wrapper: {
          id: `w${id}`,
          destroyRecursively: () => destroyed.push(`w${id}`),
        },
      },
    ]),
  )
  const scrollbox = {
    scrollTop: 12,
    scrollLeft: 0,
    scrollHeight: 80,
    height: 20,
    remove(id: string) {
      removed.push(id)
    },
    scrollTo(position: { x: number; y: number }) {
      this.scrollTop = position.y
    },
    requestRenderCount: 0,
    requestRender() {
      this.requestRenderCount += 1
    },
  }
  return {
    removed,
    destroyed,
    mounted,
    renderables,
    scrollbox,
    rebuild: () => {
      throw new Error("should not rebuild")
    },
    mountEntry: (nextEntry: TranscriptPaneEntry) => {
      mounted.push(nextEntry.id)
      renderables.set(nextEntry.id, {
        entry: nextEntry,
        wrapper: {
          id: `w${nextEntry.id}`,
          destroyRecursively: () => destroyed.push(`w${nextEntry.id}`),
        },
      })
    },
  }
}
