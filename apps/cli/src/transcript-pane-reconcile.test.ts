import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import { reconcileMountedTranscriptPane } from "./transcript-pane-reconcile.js"

function entry(id: number, role: TranscriptEntry["role"], text: string, turnId = 1): TranscriptEntry {
  return { id, role, text, turnId }
}

test("reconcileMountedTranscriptPane preserves mounted prefix and repaints only changed suffix", () => {
  const removed: Array<string | number> = []
  const destroyed: Array<string | number> = []
  const mounted: number[] = []
  const renderables = new Map([
    [1, { entry: entry(1, "user", "prompt"), wrapper: { id: "w1", destroyRecursively: () => destroyed.push("w1") } }],
    [2, { entry: entry(2, "turn_toggle", "click to collapse"), wrapper: { id: "w2", destroyRecursively: () => destroyed.push("w2") } }],
    [3, { entry: entry(3, "assistant", "summary"), wrapper: { id: "w3", destroyRecursively: () => destroyed.push("w3") } }],
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
  const scrollbox = {
    scrollTop: 12,
    scrollLeft: 0,
    scrollHeight: 80,
    height: 20,
    remove(id: string | number) {
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

  reconcileMountedTranscriptPane({
    scrollbox,
    currentEntries,
    nextEntries,
    renderables,
    clampScrollTop: (value) => value,
    rebuild: () => {
      throw new Error("should not rebuild")
    },
    mountEntry: (nextEntry) => {
      mounted.push(nextEntry.id)
      renderables.set(nextEntry.id, {
        entry: nextEntry,
        wrapper: { id: `w${nextEntry.id}`, destroyRecursively: () => destroyed.push(`w${nextEntry.id}`) },
      })
    },
  })

  assert.deepEqual(removed, ["w2", "w3"])
  assert.deepEqual(destroyed, ["w2", "w3"])
  assert.deepEqual(mounted, [4, 5])
  assert.equal(renderables.has(1), true)
  assert.equal(renderables.has(2), false)
  assert.equal(renderables.has(3), false)
  assert.equal(scrollbox.scrollTop, 12)
  assert.equal(scrollbox.requestRenderCount, 1)
})

test("reconcileMountedTranscriptPane remounts turn toggles when mode changes with the same id", () => {
  const removed: Array<string | number> = []
  const destroyed: Array<string | number> = []
  const mounted: number[] = []
  const renderables = new Map([
    [1, { entry: entry(1, "user", "prompt"), wrapper: { id: "w1", destroyRecursively: () => destroyed.push("w1") } }],
    [5, { entry: { ...entry(5, "turn_toggle", "click to collapse"), toggleMode: "collapse" as const }, wrapper: { id: "w5", destroyRecursively: () => destroyed.push("w5") } }],
  ])
  const scrollbox = {
    scrollTop: 0,
    scrollLeft: 0,
    scrollHeight: 80,
    height: 20,
    remove(id: string | number) {
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

  reconcileMountedTranscriptPane({
    scrollbox,
    currentEntries: [
      entry(1, "user", "prompt"),
      { ...entry(5, "turn_toggle", "click to collapse"), toggleMode: "collapse" as const },
    ],
    nextEntries: [
      entry(1, "user", "prompt"),
      { ...entry(5, "turn_toggle", "click to expand"), toggleMode: "expand" as const },
    ],
    renderables,
    clampScrollTop: (value) => value,
    rebuild: () => {
      throw new Error("should not rebuild")
    },
    mountEntry: (nextEntry) => {
      mounted.push(nextEntry.id)
      renderables.set(nextEntry.id, {
        entry: nextEntry,
        wrapper: { id: `next-${nextEntry.id}`, destroyRecursively: () => destroyed.push(`next-${nextEntry.id}`) },
      })
    },
  })

  assert.deepEqual(removed, ["w5"])
  assert.deepEqual(destroyed, ["w5"])
  assert.deepEqual(mounted, [5])
  assert.equal(renderables.get(5)?.entry.toggleMode, "expand")
  assert.equal(scrollbox.requestRenderCount, 1)
})
