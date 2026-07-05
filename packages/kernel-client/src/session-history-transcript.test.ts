import assert from "node:assert/strict"
import test from "node:test"

import type { SessionHistoryEntry, SessionHistoryPageEntry } from "./kernel-types.js"
import {
  hydrateSessionHistoryTranscriptEntries,
  mergePrependedHistoryTranscriptFragments,
  stitchPrependedHistoryTranscript,
  type SessionHistoryTranscriptEntry,
} from "./session-history-transcript.js"

test("session history transcript hydration reconstructs tools and suppresses ordinary status noise", () => {
  const entries = hydrateSessionHistoryTranscriptEntries([
    pageEntry(0, "user_prompt", "build\n"),
    pageEntry(1, "provider_reasoning", "thinking...\n", { merge_key: "r-1" }),
    pageEntry(2, "provider_tool", JSON.stringify({
      id: "tool-1",
      tool: "bash",
      status: "running",
      input: { command: "npm test" },
    })),
    pageEntry(3, "provider_status", "OpenCode is idle."),
    pageEntry(4, "provider_output", "all green\n", { merge_key: "a-1" }),
  ])

  assert.deepEqual(entries.map((entry) => entry.role), ["user", "reasoning", "tool", "assistant"])
  assert.equal(entries[0]?.text, "build")
  assert.match(entries[2]?.text ?? "", /\*\*bash\*\*/)
  assert.match(entries[2]?.text ?? "", /npm test/)
  assert.equal(entries[3]?.text, "all green\n")
})

test("session history transcript hydration keeps repeated merge keys scoped to turns", () => {
  const entries = hydrateSessionHistoryTranscriptEntries([
    pageEntry(0, "user_prompt", "first\n", { provider_run_id: "run-1" }),
    pageEntry(1, "provider_output", "first reply\n", {
      provider_run_id: "run-1",
      merge_key: "assistant",
    }),
    pageEntry(2, "user_prompt", "second\n", { provider_run_id: "run-2" }),
    pageEntry(3, "provider_output", "second reply\n", {
      provider_run_id: "run-2",
      merge_key: "assistant",
    }),
  ])

  assert.deepEqual(entries.map((entry) => entry.text), [
    "first",
    "first reply\n",
    "second",
    "second reply\n",
  ])
  assert.deepEqual(entries.map((entry) => entry.turnId), [1, 1, 3, 3])
  assert.deepEqual(entries.map((entry) => entry.providerRunId), ["run-1", "run-1", "run-2", "run-2"])
})

test("session history transcript hydration preserves external observed metadata", () => {
  const entries = hydrateSessionHistoryTranscriptEntries([
    pageEntry(7, "provider_output", "native ", {
      merge_key: "external:codex:thread-1:item-1",
      source: "external_provider_observed",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "item-1",
      observed_at_ms: 123,
      external_observation: {
        settles_active_prompt: false,
        passive_telemetry: true,
      },
    }, { fragmentStart: 0, fragmentEnd: 7, totalChars: 12 }),
    pageEntry(7, "provider_output", "reply", {
      merge_key: "external:codex:thread-1:item-1",
      source: "external_provider_observed",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "item-1",
      external_observation: {
        settles_active_prompt: true,
        passive_telemetry: false,
      },
    }, { fragmentStart: 7, fragmentEnd: 12, totalChars: 12 }),
  ])

  assert.equal(entries.length, 1)
  assert.equal(entries[0]?.text, "native reply")
  assert.equal(entries[0]?.source, "external_provider_observed")
  assert.equal(entries[0]?.externalProvider, "codex")
  assert.equal(entries[0]?.externalProviderSessionId, "thread-1")
  assert.equal(entries[0]?.externalProviderTurnId, "item-1")
  assert.equal(entries[0]?.observedAtMs, 123)
  assert.deepEqual(entries[0]?.externalObservation, {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
})

test("session history transcript hydration renders only non-passive external provider statuses", () => {
  const entries = hydrateSessionHistoryTranscriptEntries([
    pageEntry(1, "provider_status", "OpenCode status: reconnecting"),
    pageEntry(2, "provider_status", "codex event turn_aborted {\"reason\":\"user\"}", {
      source: "external_provider_observed",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "turn-aborted",
    }),
    pageEntry(3, "provider_status", "codex token_count {\"total\":42}", {
      source: "external_provider_observed",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "token-count",
      external_observation: {
        settles_active_prompt: false,
        passive_telemetry: true,
      },
    }),
  ])

  assert.deepEqual(entries.map((entry) => entry.text), [
    "codex event turn_aborted {\"reason\":\"user\"}",
  ])
  assert.equal(entries[0]?.role, "status")
  assert.equal(entries[0]?.source, "external_provider_observed")
})

test("session history transcript hydration preserves prompt attachment and prompt identity", () => {
  const attachments = [{
    url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
    mime: "image/png",
    filename: "Screenshot.png",
    preview_url: "data:image/png;base64,aW1hZ2U=",
  }]
  const entries = hydrateSessionHistoryTranscriptEntries([
    pageEntry(0, "user_prompt", "build\n", {
      source_attachment_id: "attachment-1",
      attachments,
    }),
    pageEntry(1, "provider_output", "native reply\n", {
      source: "external_provider_observed",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "turn-1",
      external_observation: {
        settles_active_prompt: true,
        passive_telemetry: false,
      },
    }),
  ], { promptId: "prompt-1" })

  assert.equal(entries[0]?.promptId, "prompt-1")
  assert.equal(entries[0]?.sourceAttachmentId, "attachment-1")
  assert.deepEqual(entries[0]?.attachments, attachments)
  assert.equal(entries[1]?.promptId, "prompt-1")
  assert.deepEqual(entries[1]?.externalObservation, {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
})

test("shared history transcript stitch helpers preserve metadata", () => {
  const stitched = stitchPrependedHistoryTranscript(
    [transcriptEntry(1, "assistant", "native ", {
      source: "external_provider_observed",
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
      observedAtMs: 1_000,
      externalObservation: {
        settles_active_prompt: true,
        passive_telemetry: false,
      },
      historyEntryIndex: 8,
      historyFragmentStart: 0,
      historyFragmentEnd: 7,
      historyTotalChars: 12,
    })],
    [transcriptEntry(2, "assistant", "reply", {
      historyEntryIndex: 8,
      historyFragmentStart: 7,
      historyFragmentEnd: 12,
      historyTotalChars: 12,
    })],
  )

  assert.equal(stitched.length, 1)
  assert.equal(stitched[0]?.text, "native reply")
  assert.equal(stitched[0]?.source, "external_provider_observed")
  assert.equal(stitched[0]?.externalProvider, "codex")
  assert.equal(stitched[0]?.externalProviderSessionId, "thread-1")
  assert.equal(stitched[0]?.externalProviderTurnId, "turn-1")
  assert.equal(stitched[0]?.observedAtMs, 1_000)
  assert.deepEqual(stitched[0]?.externalObservation, {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
})

test("shared history transcript fragment merge rebuilds structured tool fragments", () => {
  const toolPayload = JSON.stringify({
    id: "tool-1",
    tool: "bash",
    status: "completed",
    input: { command: "pnpm test" },
    output: "ok",
  })
  const splitAt = Math.floor(toolPayload.length / 2)

  const merged = mergePrependedHistoryTranscriptFragments(
    transcriptEntry(1, "tool", toolPayload.slice(0, splitAt), {
      sourceText: toolPayload.slice(0, splitAt),
      historyEntryIndex: 9,
      historyFragmentStart: 0,
      historyFragmentEnd: splitAt,
      historyTotalChars: toolPayload.length,
      mergeKey: "stale",
    }),
    transcriptEntry(2, "tool", toolPayload.slice(splitAt), {
      sourceText: toolPayload.slice(splitAt),
      historyEntryIndex: 9,
      historyFragmentStart: splitAt,
      historyFragmentEnd: toolPayload.length,
      historyTotalChars: toolPayload.length,
      mergeKey: "stale",
    }),
  )

  assert.equal(merged.mergeKey, "tool-1")
  assert.match(merged.text, /\*\*bash\*\*/)
  assert.match(merged.text, /pnpm test/)
  assert.equal(merged.sourceText, toolPayload)
})

function pageEntry(
  entryIndex: number,
  kind: SessionHistoryEntry["kind"],
  text: string,
  overrides: Partial<SessionHistoryEntry> = {},
  pageOverrides: {
    fragmentStart?: number
    fragmentEnd?: number
    totalChars?: number
  } = {},
): SessionHistoryPageEntry {
  return {
    entry_index: entryIndex,
    fragment_start: pageOverrides.fragmentStart ?? 0,
    fragment_end: pageOverrides.fragmentEnd ?? text.length,
    total_chars: pageOverrides.totalChars ?? text.length,
    entry: {
      kind,
      text,
      ...overrides,
    },
  }
}

function transcriptEntry(
  id: number,
  role: SessionHistoryTranscriptEntry["role"],
  text: string,
  overrides: Partial<SessionHistoryTranscriptEntry> = {},
): SessionHistoryTranscriptEntry {
  return {
    id,
    role,
    text,
    ...overrides,
  }
}
