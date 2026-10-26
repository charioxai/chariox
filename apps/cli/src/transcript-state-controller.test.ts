import assert from "node:assert/strict"
import test from "node:test"

import { hydrateSessionHistoryOutlineAgentEntries } from "@chariox/kernel-client/session-history-transcript"
import type { SessionHistoryBlobContent, TranscriptEntry } from "./cli-types.js"
import { createTranscriptStateController } from "./transcript-state-controller.js"

test("transcript state controller appends entries with ids and active turn ids", () => {
  const harness = transcriptHarness({
    entries: [entry(1, "user", "hello", { turnId: 1 })],
    entryCounter: 1,
    currentTurnId: 2,
  })

  const appended = harness.controller.appendEntry({ role: "assistant", text: "reply" })

  assert.equal(appended?.id, 2)
  assert.equal(appended?.turnId, 2)
  assert.deepEqual(harness.entries.map((candidate) => candidate.id), [1, 2])
  assert.equal(harness.entryCounter, 2)
  assert.equal(harness.persisted.length, 1)
  assert.equal(harness.reconciled.length, 1)
  assert.equal(harness.enforced, 1)
})

test("transcript state controller appends entries using runtime counters", () => {
  const harness = transcriptHarness({
    entries: [entry(1, "user", "hello", { turnId: 1 })],
    entryCounter: 10,
    currentTurnId: 2,
  })

  const appended = harness.controller.appendEntry({ role: "assistant", text: "reply" })

  assert.equal(appended?.id, 11)
  assert.equal(appended?.turnId, 2)
  assert.deepEqual(harness.entries.map((candidate) => candidate.id), [1, 11])
  assert.equal(harness.entryCounter, 11)
})

test("transcript state controller skips consecutive duplicate notices", () => {
  const harness = transcriptHarness({
    entries: [entry(1, "notice", "same", { emphasis: "warning" })],
    entryCounter: 1,
  })

  const appended = harness.controller.appendEntry({ role: "notice", text: "same", emphasis: "warning" })

  assert.equal(appended, null)
  assert.deepEqual(harness.entries.map((candidate) => candidate.id), [1])
  assert.equal(harness.persisted.length, 0)
})

test("transcript state controller toggles turns through shared display state", () => {
  const harness = transcriptHarness({
    entries: [
      entry(1, "user", "prompt", { turnId: 1 }),
      entry(-1, "turn_toggle", "click to expand", { turnId: 1, toggleMode: "expand" }),
      entry(2, "reasoning", "thinking", { turnId: 1, hidden: true, blobCollapsible: true }),
      entry(3, "assistant", "summary", { turnId: 1 }),
    ],
    collapsedTurnIds: [1],
  })

  harness.controller.toggleTurn(1, -1)

  assert.deepEqual(harness.expandedTurnUpdates, [{
    agentId: "agent-1",
    turnId: 1,
    expanded: true,
  }])
  assert.deepEqual(
    harness.entries.map((candidate) => [candidate.id, candidate.role, candidate.hidden ?? false, candidate.toggleMode ?? null]),
    [
      [1, "user", false, null],
      [-1, "turn_toggle", false, "collapse"],
      [2, "reasoning", false, null],
      [3, "assistant", false, null],
    ],
  )
  assert.equal(harness.entryCounter, 3)
  assert.equal(harness.persisted.length, 1)
  assert.equal(harness.reconciled.length, 1)
  assert.equal(harness.focusRetained, 1)
})

// MP-08 / MP-10 (panes R11): Ctrl+O opens every collapsed tool / thought row,
// then a second press collapses them all.
test("transcript state controller expands every collapsed row, then collapses them all", () => {
  const harness = transcriptHarness({
    entries: [
      entry(1, "user", "prompt", { turnId: 1 }),
      entry(2, "tool", "ls", { turnId: 1, blobCollapsible: true, blobCollapsed: true }),
      entry(3, "reasoning", "thinking", { turnId: 1, blobCollapsible: true, blobCollapsed: true }),
      entry(4, "assistant", "done", { turnId: 1 }),
    ],
  })

  harness.controller.toggleAllBlobs()
  assert.deepEqual(
    harness.entries.filter((item) => item.blobCollapsible).map((item) => [item.id, item.blobCollapsed]),
    [[2, false], [3, false]],
  )

  harness.controller.toggleAllBlobs()
  assert.deepEqual(
    harness.entries.filter((item) => item.blobCollapsible).map((item) => [item.id, item.blobCollapsed]),
    [[2, true], [3, true]],
  )
})

test("transcript state controller toggles serialized zero turn ids", () => {
  const harness = transcriptHarness({
    entries: [
      entry(1, "user", "prompt", { turnId: 0 }),
      entry(0, "turn_toggle", "click to expand", { turnId: 0, toggleMode: "expand" }),
      entry(2, "reasoning", "thinking", { turnId: 0, hidden: true, blobCollapsible: true }),
      entry(3, "assistant", "summary", { turnId: 0 }),
    ],
    collapsedTurnIds: [0],
  })

  harness.controller.toggleTurn(0, 0)

  assert.deepEqual(harness.expandedTurnUpdates, [{
    agentId: "agent-1",
    turnId: 0,
    expanded: true,
  }])
  assert.equal(harness.persisted.length, 1)
  assert.equal(harness.reconciled.length, 1)
  assert.equal(harness.focusRetained, 1)
})

test("transcript state controller toggles blob state and preserves focus", () => {
  const harness = transcriptHarness({
    entries: [
      entry(1, "tool", "large", {
        blobCollapsible: true,
        blobCollapsed: true,
      }),
    ],
    entryCounter: 1,
  })

  harness.controller.toggleBlob(1, false)

  assert.equal(harness.entries[0]?.blobCollapsed, false)
  assert.equal(harness.entryCounter, 1)
  assert.equal(harness.focusRetained, 1)
  assert.equal(harness.persisted.length, 1)
  assert.equal(harness.reconciled.length, 1)
})

test("transcript state controller ignores missing blob entries", () => {
  const harness = transcriptHarness({
    entries: [entry(1, "tool", "large")],
  })

  harness.controller.toggleBlob(99, false)

  assert.deepEqual(harness.entries.map((candidate) => candidate.id), [1])
  assert.equal(harness.persisted.length, 0)
  assert.equal(harness.reconciled.length, 0)
  assert.equal(harness.focusRetained, 0)
})

test("transcript state controller hydrates overlapping blob loads after ids shift", async () => {
  // MP-08/MP-10: a first blob that expands to several entries renumbers the
  // transcript before a second, still-loading blob resolves.
  const page = (index: number, kind: "provider_tool" | "provider_reasoning" | "provider_output", text: string) => ({
    entry_index: index,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: { kind, text },
  })
  const blob = (blobId: string, kind: "provider_tool" | "provider_reasoning", start: number, count: number) => ({
    blob_id: blobId,
    kind,
    title: kind,
    summary: blobId,
    sequence_start: start,
    sequence_end: start + count - 1,
    entry_count: count,
    total_chars: 80,
    timestamp_ms: start,
  })
  const tool = (id: string) => JSON.stringify({ id, tool: "ocr", status: "completed", output: id })
  const pending = new Map<string, (content: SessionHistoryBlobContent) => void>()
  const harness = transcriptHarness({
    entries: hydrateSessionHistoryOutlineAgentEntries({
      agent_id: "agent-1",
      turns: [{
        turn_id: "turn-1",
        prompt_id: "prompt-1",
        started_at_ms: 1,
        lifecycle: "completed",
        completed_at_ms: 5,
        user_prompt: page(0, "provider_output", "read the page\n"),
        entries: [],
        summary: page(4, "provider_output", "Linux\n"),
        blobs: [blob("blob-tools", "provider_tool", 1, 2), blob("blob-reasoning", "provider_reasoning", 3, 1)],
      }],
      next_cursor: null,
    } as never) as TranscriptEntry[],
    loadHistoryBlobContent: (_agentId, blobId) => new Promise((resolve) => pending.set(blobId, resolve)),
  })
  const placeholder = (blobId: string) => harness.entries.find((candidate) => candidate.historyBlobId === blobId)
  harness.controller.toggleBlob(placeholder("blob-tools")!.id, false)
  harness.controller.toggleBlob(placeholder("blob-reasoning")!.id, false)

  pending.get("blob-tools")!({ blob_id: "blob-tools", entries: [page(1, "provider_tool", tool("tool-1")), page(2, "provider_tool", tool("tool-2"))] } as never)
  await new Promise((resolve) => setImmediate(resolve))
  pending.get("blob-reasoning")!({ blob_id: "blob-reasoning", entries: [page(3, "provider_reasoning", "checked the article title\n")] } as never)
  await new Promise((resolve) => setImmediate(resolve))

  assert.equal(harness.entries.some((candidate) => candidate.historyBlobLoading === true), false)
  assert.ok(harness.entries.some((candidate) => candidate.role === "reasoning" && /checked the article title/.test(candidate.text)))
})

function transcriptHarness(options: {
  entries?: TranscriptEntry[]
  entryCounter?: number
  currentTurnId?: number | null
  collapsedTurnIds?: number[]
  loadHistoryBlobContent?: (agentId: string, blobId: string) => Promise<SessionHistoryBlobContent>
  reconcileInPlace?: boolean
}) {
  const harness = {
    entries: options.entries ?? [],
    entryCounter: options.entryCounter ?? 0,
    currentTurnId: options.currentTurnId ?? null,
    collapsedTurnIds: options.collapsedTurnIds ?? [],
    expandedTurnUpdates: [] as Array<{
      agentId: string | null | undefined
      turnId: number
      expanded: boolean
    }>,
    persisted: [] as TranscriptEntry[][],
    reconciled: [] as Array<{ current: TranscriptEntry[]; next: TranscriptEntry[] }>,
    enforced: 0,
    focusRetained: 0,
    controller: null as ReturnType<typeof createTranscriptStateController> | null,
  }
  harness.controller = createTranscriptStateController({
    entries: () => harness.entries,
    setEntries: (nextEntries) => {
      if (options.reconcileInPlace) {
        for (const next of nextEntries) {
          const current = harness.entries.find((item) => item.id === next.id)
          if (current) Object.assign(current, next)
        }
      } else {
        harness.entries = nextEntries
      }
    },
    entryCounter: () => harness.entryCounter,
    setEntryCounter: (value) => {
      harness.entryCounter = value
    },
    currentTurnId: () => harness.currentTurnId,
    visibleTranscriptAgentId: () => "agent-1",
    collapsedTurnIdsForAgent: () => harness.collapsedTurnIds,
    setExpandedTurnState: (agentId, turnId, expanded) => {
      harness.expandedTurnUpdates.push({ agentId, turnId, expanded })
    },
    persistVisibleTranscriptEntries: (entries) => {
      harness.persisted.push(entries)
    },
    reconcileMountedTranscript: (current, next) => {
      harness.reconciled.push({ current, next })
    },
    retainPromptFocus: () => {
      harness.focusRetained += 1
    },
    enforceTranscriptRetention: () => {
      harness.enforced += 1
    },
    ...(options.loadHistoryBlobContent ? { loadHistoryBlobContent: options.loadHistoryBlobContent } : {}),
  })
  return harness as typeof harness & { controller: ReturnType<typeof createTranscriptStateController> }
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
  } as TranscriptEntry
}

// MP-08 / MP-10, #987: one detail action commits one projection.
test("Ctrl+O projects all loaded rows once", () => {
  const harness = transcriptHarness({ entries: [
    entry(1, "tool", "a", { blobCollapsible: true, blobCollapsed: true }),
    entry(2, "reasoning", "b", { blobCollapsible: true, blobCollapsed: true }),
  ] })
  harness.controller.toggleAllBlobs()
  assert.equal(harness.reconciled.length, 1)
})

// MP-08 / MP-10: Solid reconcile updates existing rows in place. The mounted
// comparison must retain the state that was visible before that write.
test("disclosure retains the pre-update render snapshot with in-place stores", () => {
  const harness = transcriptHarness({
    entries: [entry(1, "tool", "actual file output", { blobCollapsible: true, blobCollapsed: true })],
    reconcileInPlace: true,
  })
  harness.controller.toggleBlob(1, false)
  assert.equal(harness.entries[0]?.blobCollapsed, false)
  assert.equal(harness.reconciled[0]?.current[0]?.blobCollapsed, true)
  assert.equal(harness.reconciled[0]?.next[0]?.blobCollapsed, false)
  harness.controller.toggleBlob(1, true)
  assert.equal(harness.reconciled[1]?.current[0]?.blobCollapsed, false)
  assert.equal(harness.reconciled[1]?.next[0]?.blobCollapsed, true)
})
