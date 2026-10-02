import assert from "node:assert/strict"
import test from "node:test"
import { Renderable, ScrollBoxRenderable } from "@opentui/core"
import { createTestRenderer } from "@opentui/core/testing"
import { transcriptRetentionSlice } from "@chariox/kernel-client/transcript-entry-state"
import { createAgentPaneStreamingCommitController } from "./agent-pane-streaming-commit-controller.js"
import { createAgentPaneTranscriptRenderController } from "./agent-pane-transcript-render-controller.js"
import { createAgentPaneTranscriptStreamController } from "./agent-pane-transcript-stream-controller.js"
import { buildTranscriptEntryRenderable, transcriptRenderMode } from "./transcript-render.js"
import { createTranscriptSyntaxStyle } from "./theme.js"
import type { TranscriptEntry } from "./cli-types.js"

test("merged auxiliary output releases evicted renderables across 1,000 streams", async () => {
  const harness = await createTestRenderer({ width: 80, height: 24, useThread: false })
  const scrollbox = new ScrollBoxRenderable(harness.renderer, { width: 80, height: 20 })
  harness.renderer.root.add(scrollbox)
  const initialRenderableCount = Renderable.renderablesByNumber.size
  const syntax = createTranscriptSyntaxStyle()
  const agentId = "auxiliary"
  let entries: TranscriptEntry[] = []
  const renderables = new Map<string, Map<number, ReturnType<typeof buildTranscriptEntryRenderable>>>()
  const pane = createAgentPaneTranscriptRenderController({
    scrollboxes: new Map([[agentId, scrollbox]]),
    entryRenderables: renderables,
    emptyRenderables: new Map(),
    toolStates: new Map(),
    paneEntries: () => entries,
    buildEmptyRenderable: () => { throw new Error("unexpected empty pane") },
    buildEntryRenderable: (_agentId, entry) => buildTranscriptEntryRenderable(
      harness.renderer, entry, syntax, () => {}, () => {},
    ),
    renderMode: transcriptRenderMode,
    requestRenderable: (renderable) => renderable?.requestRender(),
    clampScrollTop: (top, height, viewport) => Math.max(0, Math.min(top, height - viewport)),
    activeAgentIdsForSession: () => [agentId],
  })
  const trim = (_agentId: string, next: TranscriptEntry[]) =>
    transcriptRetentionSlice(next, { maxEntries: 400, maxChars: 256 }).kept
  const commit = createAgentPaneStreamingCommitController({
    trimLiveAgentPaneEntries: trim,
    collapsedTurnIdsForAgent: () => [],
    commitAgentPaneEntries: (_agentId, next) => { entries = next },
    splitAgentResponseMode: () => true,
    getResponsePrimaryAgentId: () => "primary",
    replaceTranscriptEntries: () => { throw new Error("unexpected primary update") },
    visibleAuxiliaryAgentIds: () => [agentId],
    updateAuxiliaryTranscriptEntry: pane.updateEntry,
    reconcileMountedAuxiliaryTranscript: pane.reconcileMountedTranscript,
  })
  const stream = createAgentPaneTranscriptStreamController({
    currentAgentPaneEntries: () => entries,
    trimLiveAgentPaneEntries: trim,
    setAgentTranscriptEntries: (_agentId, next) => {
      const previous = entries
      entries = next
      pane.reconcileMountedTranscript(agentId, previous, next)
    },
    commitStreamingAgentPaneEntry: commit.commitStreamingEntry,
    toolStateForAgent: () => new Map(),
  })
  try {
    for (let index = 0; index < 1_000; index += 1) {
      // The first chunk fits alongside the previous entry. Only its continuation
      // exceeds the budget, exercising the merged-output path rather than append.
      stream.appendProviderChunk(agentId, "assistant", "a".repeat(32), `stream-${index}`)
      stream.appendProviderChunk(agentId, "assistant", "b".repeat(128), `stream-${index}`)
      if (index % 50 === 0) await harness.renderOnce()
    }
    await harness.renderOnce()
    assert.equal(entries.length, 1)
    assert.equal(renderables.get(agentId)?.size, entries.length)
    assert.equal(scrollbox.getChildren().length, entries.length)
    assert.equal(
      renderables.get(agentId)?.values().next().value?.entry.text,
      "a".repeat(32) + "b".repeat(128),
    )
    pane.clearPane(agentId)
    assert.equal(scrollbox.getChildren().length, 0)
    assert.equal(renderables.has(agentId), false)
    assert.equal(Renderable.renderablesByNumber.size, initialRenderableCount)
  } finally {
    harness.renderer.destroy()
    syntax.destroy()
  }
})
