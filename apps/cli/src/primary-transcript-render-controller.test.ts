import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import { createPrimaryTranscriptRenderController } from "./primary-transcript-render-controller.js"

test("primary transcript render controller mounts entries and clears empty state", () => {
  const empty = child("empty")
  const harness = renderHarness({
    scrollbox: scrollbox([empty]),
    emptyRenderable: empty,
  })

  harness.controller.mountEntry(entry(1, "assistant", "hello"))

  assert.equal(empty.destroyed, true)
  assert.equal(harness.emptyRenderable, undefined)
  assert.deepEqual(harness.scrollbox?.childIds(), ["entry-1"])
  assert.equal(harness.requestTranscriptRenderCount, 1)
})

test("primary transcript render controller updates mounted entries in place", () => {
  const renderable = transcriptRenderable(entry(1, "assistant", "old"))
  const harness = renderHarness({
    scrollbox: scrollbox([renderable.wrapper]),
    renderables: new Map([[1, renderable]]),
  })

  harness.controller.updateEntry(1, "new", "raw")

  assert.equal(renderable.entry.text, "new")
  assert.equal(renderable.entry.sourceText, "raw")
  assert.deepEqual(renderable.updates.map((item) => item.text), ["new"])
  assert.equal(harness.requestTranscriptRenderCount, 1)
})

test("primary transcript render controller rebuilds workflow outline placeholders", () => {
  const existing = child("existing")
  const harness = renderHarness({
    scrollbox: scrollbox([existing]),
    showWorkflowOutline: true,
    workflowRenderable: child("workflow"),
    visibleEntries: [entry(1, "assistant", "ignored")],
  })

  harness.controller.rebuildTranscript()

  assert.equal(existing.destroyed, true)
  assert.equal(harness.emptyRenderable?.id, "workflow")
  assert.deepEqual(harness.scrollbox?.childIds(), ["workflow"])
  assert.deepEqual(harness.scrollbox?.scrollPositions, [{ x: 0, y: 0 }])
  assert.equal(harness.rendererRenderCount, 1)
})

test("primary transcript render controller reconciles by rebuilding on workflow screen", () => {
  const harness = renderHarness({
    scrollbox: scrollbox(),
    workflowScreenActive: true,
    showWorkflowOutline: true,
    workflowRenderable: child("workflow"),
  })

  harness.controller.reconcileMountedTranscript([], [entry(1, "assistant", "hello")])

  assert.deepEqual(harness.scrollbox?.childIds(), ["workflow"])
})

function renderHarness(options: {
  scrollbox?: FakeScrollbox
  emptyRenderable?: FakeChild
  renderables?: Map<number, FakeTranscriptRenderable>
  visibleEntries?: TranscriptEntry[]
  workflowScreenActive?: boolean
  showWorkflowOutline?: boolean
  workflowRenderable?: FakeChild
} = {}) {
  const harness = {
    scrollbox: options.scrollbox,
    emptyRenderable: options.emptyRenderable,
    renderables: options.renderables ?? new Map<number, FakeTranscriptRenderable>(),
    visibleEntries: options.visibleEntries ?? [],
    workflowScreenActive: options.workflowScreenActive ?? false,
    showWorkflowOutline: options.showWorkflowOutline ?? false,
    workflowRenderable: options.workflowRenderable ?? child("workflow"),
    emptyCounter: 0,
    requestTranscriptRenderCount: 0,
    rendererRenderCount: 0,
    lastScrollTop: null as number | null,
    logs: [] as string[],
    controller: null as ReturnType<typeof createPrimaryTranscriptRenderController> | null,
  }
  harness.controller = createPrimaryTranscriptRenderController({
    getScrollbox: () => harness.scrollbox,
    getEmptyRenderable: () => harness.emptyRenderable,
    setEmptyRenderable: (renderable) => {
      harness.emptyRenderable = renderable
    },
    renderables: harness.renderables,
    visibleEntries: () => harness.visibleEntries,
    workflowScreenActive: () => harness.workflowScreenActive,
    showWorkflowOutline: () => harness.showWorkflowOutline,
    buildWorkflowRenderable: () => harness.workflowRenderable,
    buildEmptyRenderable: () => child(`empty-${++harness.emptyCounter}`),
    buildEntryRenderable: (nextEntry) => transcriptRenderable(nextEntry),
    renderMode: (nextEntry) => nextEntry.role,
    requestTranscriptRender: () => {
      harness.requestTranscriptRenderCount += 1
    },
    requestRendererRender: () => {
      harness.rendererRenderCount += 1
    },
    shouldResetEmptyScrollTop: () => true,
    clampScrollTop: (top) => top,
    setLastScrollTop: (scrollTop) => {
      harness.lastScrollTop = scrollTop
    },
    logViewDebug: (phase) => {
      harness.logs.push(phase)
    },
  })
  return harness as typeof harness & {
    controller: ReturnType<typeof createPrimaryTranscriptRenderController>
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
    scrollPositions: [] as Array<{ x: number; y: number }>,
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
    scrollTo(position: { x: number; y: number }) {
      this.scrollTop = position.y
      this.scrollLeft = position.x
      this.scrollPositions.push(position)
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
