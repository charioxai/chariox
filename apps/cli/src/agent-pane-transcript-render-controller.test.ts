import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import { createAgentPaneTranscriptRenderController } from "./agent-pane-transcript-render-controller.js"

test("agent pane transcript render controller rebuilds empty panes", () => {
  const existing = child("existing")
  const harness = renderHarness({
    scrollboxes: new Map([["agent-1", scrollbox([existing])]]),
  })

  harness.controller.rebuildPane("agent-1")

  assert.equal(existing.destroyed, true)
  assert.deepEqual(harness.scrollboxes.get("agent-1")?.childIds(), ["empty-1"])
  assert.equal(harness.emptyRenderables.get("agent-1")?.id, "empty-1")
})

test("agent pane transcript render controller mounts entries and removes empty state", () => {
  const empty = child("empty")
  const harness = renderHarness({
    scrollboxes: new Map([["agent-1", scrollbox([empty])]]),
    emptyRenderables: new Map([["agent-1", empty]]),
  })

  harness.controller.mountEntry("agent-1", entry(1, "assistant", "hello"))

  assert.equal(empty.destroyed, true)
  assert.deepEqual(harness.scrollboxes.get("agent-1")?.childIds(), ["entry-1"])
  assert.equal(harness.entryRenderables.get("agent-1")?.get(1)?.entry.text, "hello")
})

test("agent pane transcript render controller updates mounted entries in place", () => {
  const renderable = transcriptRenderable(entry(1, "assistant", "old"))
  const pane = scrollbox([renderable.wrapper])
  const harness = renderHarness({
    scrollboxes: new Map([["agent-1", pane]]),
    entryRenderables: new Map([["agent-1", new Map([[1, renderable]])]]),
  })

  harness.controller.updateEntry("agent-1", entry(1, "assistant", "new"))

  assert.equal(renderable.entry.text, "new")
  assert.deepEqual(renderable.updates.map((item) => item.text), ["new"])
  assert.equal(harness.requestedRenderables.length, 1)
  assert.equal(harness.requestedRenderables[0], pane)
})

test("agent pane transcript render controller prunes inactive pane state", () => {
  const inactiveEntry = child("inactive-entry")
  const inactiveEmpty = child("inactive-empty")
  const activeEntry = child("active-entry")
  const activeEmpty = child("active-empty")
  const harness = renderHarness({
    activeIds: ["agent-2"],
    scrollboxes: new Map([
      ["agent-1", scrollbox([inactiveEntry, inactiveEmpty])],
      ["agent-2", scrollbox([activeEntry, activeEmpty])],
    ]),
    entryRenderables: new Map([
      ["agent-1", new Map([[1, transcriptRenderable(entry(1, "assistant", "stale"))]])],
      ["agent-2", new Map()],
    ]),
    emptyRenderables: new Map([
      ["agent-1", inactiveEmpty],
      ["agent-2", activeEmpty],
    ]),
    toolStates: new Map([
      ["agent-1", new Map([["tool", { id: "stale" }]])],
      ["agent-2", new Map([["tool", { id: "active" }]])],
    ]),
  })

  harness.controller.prunePanes({ id: "session" })

  assert.deepEqual([...harness.scrollboxes.keys()], ["agent-2"])
  assert.deepEqual([...harness.entryRenderables.keys()], ["agent-2"])
  assert.deepEqual([...harness.emptyRenderables.keys()], ["agent-2"])
  assert.deepEqual([...harness.toolStates.keys()], ["agent-2"])
  assert.equal(inactiveEntry.destroyed, true)
  assert.equal(inactiveEmpty.destroyed, true)
  assert.equal(activeEntry.destroyed, false)
  assert.equal(activeEmpty.destroyed, false)
})

function renderHarness(options: {
  activeIds?: string[]
  paneEntries?: Record<string, TranscriptEntry[]>
  scrollboxes?: Map<string, FakeScrollbox>
  entryRenderables?: Map<string, Map<number, FakeTranscriptRenderable>>
  emptyRenderables?: Map<string, FakeChild>
  toolStates?: Map<string, Map<string, { id: string }>>
} = {}) {
  const harness = {
    activeIds: options.activeIds ?? [],
    paneEntries: options.paneEntries ?? {},
    scrollboxes: options.scrollboxes ?? new Map<string, FakeScrollbox>(),
    entryRenderables: options.entryRenderables ?? new Map<string, Map<number, FakeTranscriptRenderable>>(),
    emptyRenderables: options.emptyRenderables ?? new Map<string, FakeChild>(),
    toolStates: options.toolStates ?? new Map<string, Map<string, { id: string }>>(),
    emptyCounter: 0,
    requestedRenderables: [] as Array<FakeScrollbox | undefined>,
    controller: null as ReturnType<typeof createAgentPaneTranscriptRenderController> | null,
  }
  harness.controller = createAgentPaneTranscriptRenderController({
    scrollboxes: harness.scrollboxes,
    entryRenderables: harness.entryRenderables,
    emptyRenderables: harness.emptyRenderables,
    toolStates: harness.toolStates,
    paneEntries: (agentId) => harness.paneEntries[agentId] ?? [],
    buildEmptyRenderable: () => child(`empty-${++harness.emptyCounter}`),
    buildEntryRenderable: (_agentId, nextEntry) => transcriptRenderable(nextEntry),
    renderMode: (nextEntry) => nextEntry.role,
    requestRenderable: (renderable) => {
      harness.requestedRenderables.push(renderable)
    },
    clampScrollTop: (top) => top,
    activeAgentIdsForSession: () => harness.activeIds,
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createAgentPaneTranscriptRenderController>
  }
}

type FakeChild = ReturnType<typeof child>
type FakeScrollbox = ReturnType<typeof scrollbox>
type FakeTranscriptRenderable = ReturnType<typeof transcriptRenderable>

function child(id: string) {
  return {
    id,
    destroyed: false,
    destroyRecursively() {
      this.destroyed = true
    },
  }
}

function scrollbox(initialChildren: FakeChild[] = []) {
  const children = [...initialChildren]
  return {
    scrollTop: 0,
    scrollLeft: 0,
    scrollHeight: 0,
    height: 0,
    getChildren: () => children,
    add: (renderable: FakeChild) => {
      children.push(renderable)
    },
    remove: (id: string) => {
      const index = children.findIndex((item) => item.id === id)
      if (index >= 0) {
        children.splice(index, 1)
      }
    },
    scrollTo: (position: { x: number; y: number }) => {
      void position
    },
    requestRender: () => {},
    childIds: () => children.map((item) => item.id),
  }
}

function transcriptRenderable(nextEntry: TranscriptEntry) {
  const updates: TranscriptEntry[] = []
  return {
    entry: nextEntry,
    wrapper: child(`entry-${nextEntry.id}`),
    updates,
    update(entryUpdate: TranscriptEntry) {
      this.entry = entryUpdate
      updates.push(entryUpdate)
    },
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
