import assert from "node:assert/strict"
import test from "node:test"

import type { TranscriptEntry } from "./cli-types.js"
import {
  previewLineForHistoryEntry,
  hydrateTranscriptEntries,
  mergeAdjacentHistoryPageEntries,
  stitchPrependedHistory,
} from "./transcript-history.js"

test("hydrateTranscriptEntries reconstructs tool updates and suppresses idle status noise", () => {
  const entries = hydrateTranscriptEntries([
    {
      entry_index: 0,
      fragment_start: 0,
      fragment_end: 6,
      total_chars: 6,
      entry: { kind: "user_prompt", text: "build\n" },
    },
    {
      entry_index: 1,
      fragment_start: 0,
      fragment_end: 15,
      total_chars: 15,
      entry: { kind: "provider_reasoning", text: "thinking...\n", merge_key: "r-1" },
    },
    {
      entry_index: 2,
      fragment_start: 0,
      fragment_end: 46,
      total_chars: 46,
      entry: {
        kind: "provider_tool",
        text: JSON.stringify({
          id: "tool-1",
          tool: "bash",
          status: "running",
          input: { command: "npm test" },
        }),
      },
    },
    {
      entry_index: 3,
      fragment_start: 0,
      fragment_end: 18,
      total_chars: 18,
      entry: { kind: "provider_status", text: "OpenCode is idle." },
    },
    {
      entry_index: 4,
      fragment_start: 0,
      fragment_end: 13,
      total_chars: 13,
      entry: { kind: "provider_output", text: "all green\n", merge_key: "a-1" },
    },
  ])

  assert.deepEqual(
    entries.map((entry) => entry.role),
    ["user", "reasoning", "tool", "assistant"],
  )
  assert.equal(entries[0]?.text, "build")
  assert.match(entries[2]?.text ?? "", /\*\*bash\*\*/)
  assert.match(entries[2]?.text ?? "", /npm test/)
  assert.equal(entries[3]?.text, "all green\n")
})

test("hydrateTranscriptEntries marks only the head partial fragment as deferred after rejoin catch-up", () => {
  const entries = hydrateTranscriptEntries([
    {
      entry_index: 5,
      fragment_start: 120,
      fragment_end: 240,
      total_chars: 240,
      entry: { kind: "provider_output", text: "continued reply\n", merge_key: "reply-1" },
    },
    {
      entry_index: 6,
      fragment_start: 0,
      fragment_end: 12,
      total_chars: 12,
      entry: { kind: "notice", text: "reattached" },
    },
  ])

  assert.equal(entries[0]?.historyDeferred, true)
  assert.equal(entries[1]?.historyDeferred, undefined)
})

test("mergeAdjacentHistoryPageEntries preserves merge keys across stitched fragments", () => {
  const merged = mergeAdjacentHistoryPageEntries([
    {
      entry_index: 2,
      fragment_start: 0,
      fragment_end: 5,
      total_chars: 10,
      entry: { kind: "provider_output", text: "hello", merge_key: "a-1" },
    },
    {
      entry_index: 2,
      fragment_start: 5,
      fragment_end: 10,
      total_chars: 10,
      entry: { kind: "provider_output", text: " world", merge_key: "a-1" },
    },
  ])

  assert.equal(merged.length, 1)
  assert.equal(merged[0]?.entry.text, "hello world")
  assert.equal(merged[0]?.entry.merge_key, "a-1")
})

test("hydrateTranscriptEntries keeps repeated merge keys scoped to their turn", () => {
  const entries = hydrateTranscriptEntries([
    {
      entry_index: 0,
      fragment_start: 0,
      fragment_end: 6,
      total_chars: 6,
      entry: { kind: "user_prompt", text: "first\n", provider_run_id: "run-1" },
    },
    {
      entry_index: 1,
      fragment_start: 0,
      fragment_end: 12,
      total_chars: 12,
      entry: {
        kind: "provider_output",
        text: "first reply\n",
        provider_run_id: "run-1",
        merge_key: "assistant",
      },
    },
    {
      entry_index: 2,
      fragment_start: 0,
      fragment_end: 7,
      total_chars: 7,
      entry: { kind: "user_prompt", text: "second\n", provider_run_id: "run-2" },
    },
    {
      entry_index: 3,
      fragment_start: 0,
      fragment_end: 13,
      total_chars: 13,
      entry: {
        kind: "provider_output",
        text: "second reply\n",
        provider_run_id: "run-2",
        merge_key: "assistant",
      },
    },
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

test("hydrateTranscriptEntries keeps adjacent unkeyed assistant history entries separate", () => {
  const entries = hydrateTranscriptEntries([
    {
      entry_index: 1,
      fragment_start: 0,
      fragment_end: 12,
      total_chars: 12,
      entry: { kind: "provider_output", text: "first blob\n", provider_run_id: "run-1" },
    },
    {
      entry_index: 2,
      fragment_start: 0,
      fragment_end: 13,
      total_chars: 13,
      entry: { kind: "provider_output", text: "second blob\n", provider_run_id: "run-1" },
    },
  ])

  assert.deepEqual(entries.map((entry) => entry.role), ["assistant", "assistant"])
  assert.deepEqual(entries.map((entry) => entry.text), ["first blob\n", "second blob\n"])
  assert.deepEqual(entries.map((entry) => entry.historyEntryIndex), [1, 2])
})

test("hydrateTranscriptEntries preserves external provider observed metadata", () => {
  const entries = hydrateTranscriptEntries([
    {
      entry_index: 7,
      fragment_start: 0,
      fragment_end: 16,
      total_chars: 16,
      entry: {
        kind: "provider_output",
        text: "native reply\n",
        merge_key: "external:codex:thread-1:item-1",
        source: "external_provider_observed",
        external_provider: "codex",
        external_provider_session_id: "thread-1",
        external_provider_turn_id: "item-1",
        observed_at_ms: 123,
      },
    },
  ])

  assert.equal(entries[0]?.source, "external_provider_observed")
  assert.equal(entries[0]?.externalProvider, "codex")
  assert.equal(entries[0]?.externalProviderSessionId, "thread-1")
  assert.equal(entries[0]?.externalProviderTurnId, "item-1")
  assert.equal(entries[0]?.observedAtMs, 123)
})

test("hydrateTranscriptEntries merges external observation metadata across fragments", () => {
  const entries = hydrateTranscriptEntries([
    {
      entry_index: 7,
      fragment_start: 0,
      fragment_end: 7,
      total_chars: 12,
      entry: {
        kind: "provider_output",
        text: "native ",
        merge_key: "external:codex:thread-1:item-1",
        source: "external_provider_observed",
        external_provider: "codex",
        external_provider_session_id: "thread-1",
        external_provider_turn_id: "item-1",
        external_observation: {
          settles_active_prompt: false,
          passive_telemetry: true,
        },
      },
    },
    {
      entry_index: 7,
      fragment_start: 7,
      fragment_end: 12,
      total_chars: 12,
      entry: {
        kind: "provider_output",
        text: "reply",
        merge_key: "external:codex:thread-1:item-1",
        source: "external_provider_observed",
        external_provider: "codex",
        external_provider_session_id: "thread-1",
        external_provider_turn_id: "item-1",
        external_observation: {
          settles_active_prompt: true,
          passive_telemetry: false,
        },
      },
    },
  ])

  assert.equal(entries.length, 1)
  assert.equal(entries[0]?.text, "native reply")
  assert.deepEqual(entries[0]?.externalObservation, {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
})

test("hydrateTranscriptEntries renders only externally observed provider statuses from history", () => {
  const entries = hydrateTranscriptEntries([
    {
      entry_index: 1,
      fragment_start: 0,
      fragment_end: 31,
      total_chars: 31,
      entry: { kind: "provider_status", text: "OpenCode status: reconnecting" },
    },
    {
      entry_index: 2,
      fragment_start: 0,
      fragment_end: 39,
      total_chars: 39,
      entry: {
        kind: "provider_status",
        text: "codex event turn_aborted {\"reason\":\"user\"}",
        source: "external_provider_observed",
        external_provider: "codex",
        external_provider_session_id: "thread-1",
        external_provider_turn_id: "turn-aborted",
      },
    },
    {
      entry_index: 3,
      fragment_start: 0,
      fragment_end: 34,
      total_chars: 34,
      entry: {
        kind: "provider_status",
        text: "codex token_count {\"total\":42}",
        source: "external_provider_observed",
        external_provider: "codex",
        external_provider_session_id: "thread-1",
        external_provider_turn_id: "token-count",
        external_observation: {
          settles_active_prompt: false,
          passive_telemetry: true,
        },
      },
    },
  ])

  assert.deepEqual(entries.map((entry) => entry.text), [
    "codex event turn_aborted {\"reason\":\"user\"}",
  ])
  assert.equal(entries[0]?.role, "status")
  assert.equal(entries[0]?.source, "external_provider_observed")
})

test("previewLineForHistoryEntry suppresses non-external provider statuses", () => {
  assert.equal(previewLineForHistoryEntry({
    kind: "provider_status",
    text: "OpenCode status: reconnecting",
  }), null)
  assert.equal(previewLineForHistoryEntry({
    kind: "provider_status",
    text: "codex token_count {\"total\":42}",
    source: "external_provider_observed",
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }), null)
  assert.equal(previewLineForHistoryEntry({
    kind: "provider_status",
    text: "codex event turn_aborted {\"reason\":\"user\"}",
    source: "external_provider_observed",
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), "Stat: codex event turn_aborted {\"reason\":\"user\"}")
  assert.equal(previewLineForHistoryEntry({
    kind: "provider_status",
    text: "codex event turn_aborted {\"reason\":\"user\"}",
    source: "external_provider_observed",
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: true,
    },
  }), "Stat: codex event turn_aborted {\"reason\":\"user\"}")
})

test("hydrateTranscriptEntries preserves prompt identity, attachment identity, and external observation metadata", () => {
  const entries = hydrateTranscriptEntries([
    {
      entry_index: 0,
      fragment_start: 0,
      fragment_end: 6,
      total_chars: 6,
      entry: {
        kind: "user_prompt",
        text: "build\n",
        source_attachment_id: "attachment-1",
        attachments: [{
          url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
          mime: "image/png",
          filename: "Screenshot.png",
          preview_url: "data:image/png;base64,aW1hZ2U=",
        }],
      },
    },
    {
      entry_index: 1,
      fragment_start: 0,
      fragment_end: 13,
      total_chars: 13,
      entry: {
        kind: "provider_output",
        text: "native reply\n",
        source: "external_provider_observed",
        external_provider: "codex",
        external_provider_session_id: "thread-1",
        external_provider_turn_id: "turn-1",
        external_observation: {
          settles_active_prompt: true,
          passive_telemetry: false,
        },
      },
    },
  ], { promptId: "prompt-1" })

  assert.equal(entries[0]?.promptId, "prompt-1")
  assert.equal(entries[0]?.sourceAttachmentId, "attachment-1")
  assert.deepEqual(entries[0]?.attachments, [{
    url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
    mime: "image/png",
    filename: "Screenshot.png",
    preview_url: "data:image/png;base64,aW1hZ2U=",
  }])
  assert.equal(entries[1]?.promptId, "prompt-1")
  assert.deepEqual(entries[1]?.externalObservation, {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
})

test("mergeAdjacentHistoryPageEntries preserves attachment identity across stitched fragments", () => {
  const merged = mergeAdjacentHistoryPageEntries([
    {
      entry_index: 0,
      fragment_start: 0,
      fragment_end: 3,
      total_chars: 6,
      entry: {
        kind: "user_prompt",
        text: "hel",
        source_attachment_id: "attachment-1",
        attachments: [{
          url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
          mime: "image/png",
          filename: "Screenshot.png",
          preview_url: "data:image/png;base64,aW1hZ2U=",
        }],
      },
    },
    {
      entry_index: 0,
      fragment_start: 3,
      fragment_end: 6,
      total_chars: 6,
      entry: {
        kind: "user_prompt",
        text: "lo\n",
        source_attachment_id: "attachment-1",
        attachments: [{
          url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
          mime: "image/png",
          filename: "Screenshot.png",
          preview_url: "data:image/png;base64,aW1hZ2U=",
        }],
      },
    },
  ])

  assert.equal(merged.length, 1)
  assert.equal(merged[0]?.entry.text, "hello\n")
  assert.equal(merged[0]?.entry.source_attachment_id, "attachment-1")
  assert.deepEqual(merged[0]?.entry.attachments, [{
    url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
    mime: "image/png",
    filename: "Screenshot.png",
    preview_url: "data:image/png;base64,aW1hZ2U=",
  }])
})

test("mergeAdjacentHistoryPageEntries upgrades matching attachments without dropping extra chips", () => {
  const merged = mergeAdjacentHistoryPageEntries([
    {
      entry_index: 0,
      fragment_start: 0,
      fragment_end: 3,
      total_chars: 6,
      entry: {
        kind: "user_prompt",
        text: "hel",
        attachments: [{
          url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
          mime: "image/png",
          filename: "Screenshot.png",
          preview_url: null,
        }, {
          url: "arroba-terminal://prompt-attachment/attachment-2/notes.txt",
          mime: "text/plain",
          filename: "notes.txt",
          preview_url: null,
        }],
      },
    },
    {
      entry_index: 0,
      fragment_start: 3,
      fragment_end: 6,
      total_chars: 6,
      entry: {
        kind: "user_prompt",
        text: "lo\n",
        attachments: [{
          url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
          mime: "image/png",
          filename: "Screenshot.png",
          preview_url: "data:image/png;base64,aW1hZ2U=",
        }],
      },
    },
  ])

  assert.deepEqual(merged[0]?.entry.attachments, [{
    url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
    mime: "image/png",
    filename: "Screenshot.png",
    preview_url: "data:image/png;base64,aW1hZ2U=",
  }, {
    url: "arroba-terminal://prompt-attachment/attachment-2/notes.txt",
    mime: "text/plain",
    filename: "notes.txt",
    preview_url: null,
  }])
})

test("stitchPrependedHistory merges adjacent assistant fragments", () => {
  const stitched = stitchPrependedHistory(
    [entry(1, "assistant", "hello ", {
      historyEntryIndex: 7,
      historyFragmentStart: 0,
      historyFragmentEnd: 6,
      historyTotalChars: 11,
    })],
    [entry(2, "assistant", "world", {
      historyEntryIndex: 7,
      historyFragmentStart: 6,
      historyFragmentEnd: 11,
      historyTotalChars: 11,
    })],
  )

  assert.equal(stitched.length, 1)
  assert.equal(stitched[0]?.text, "hello world")
  assert.equal(stitched[0]?.sourceText, "hello world")
  assert.equal(stitched[0]?.historyFragmentStart, 0)
  assert.equal(stitched[0]?.historyFragmentEnd, 11)
  assert.equal(stitched[0]?.historyDeferred, undefined)
})

test("stitchPrependedHistory preserves prompt attachment metadata while merging fragments", () => {
  const attachments = [{
    url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
    mime: "image/png",
    filename: "Screenshot.png",
    preview_url: "data:image/png;base64,aW1hZ2U=",
  }]
  const stitched = stitchPrependedHistory(
    [entry(1, "user", "inspect ", {
      historyEntryIndex: 8,
      historyFragmentStart: 0,
      historyFragmentEnd: 8,
      historyTotalChars: 13,
      promptId: "prompt-1",
      sourceAttachmentId: "attachment-1",
      attachments,
    })],
    [entry(2, "user", "image", {
      historyEntryIndex: 8,
      historyFragmentStart: 8,
      historyFragmentEnd: 13,
      historyTotalChars: 13,
    })],
  )

  assert.equal(stitched.length, 1)
  assert.equal(stitched[0]?.text, "inspect image")
  assert.equal(stitched[0]?.promptId, "prompt-1")
  assert.equal(stitched[0]?.sourceAttachmentId, "attachment-1")
  assert.deepEqual(stitched[0]?.attachments, attachments)
})

test("stitchPrependedHistory preserves external observed metadata while merging fragments", () => {
  const stitched = stitchPrependedHistory(
    [entry(1, "assistant", "native ", {
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
    [entry(2, "assistant", "reply", {
      historyEntryIndex: 8,
      historyFragmentStart: 7,
      historyFragmentEnd: 12,
      historyTotalChars: 12,
    })],
  )

  assert.equal(stitched.length, 1)
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

test("stitchPrependedHistory merges external observation metadata while merging fragments", () => {
  const stitched = stitchPrependedHistory(
    [entry(1, "assistant", "native ", {
      source: "external_provider_observed",
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
      externalObservation: {
        settles_active_prompt: true,
        passive_telemetry: false,
      },
      historyEntryIndex: 8,
      historyFragmentStart: 0,
      historyFragmentEnd: 7,
      historyTotalChars: 12,
    })],
    [entry(2, "assistant", "reply", {
      source: "external_provider_observed",
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
      externalObservation: {
        settles_active_prompt: false,
        passive_telemetry: true,
      },
      historyEntryIndex: 8,
      historyFragmentStart: 7,
      historyFragmentEnd: 12,
      historyTotalChars: 12,
    })],
  )

  assert.equal(stitched.length, 1)
  assert.deepEqual(stitched[0]?.externalObservation, {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
})

test("stitchPrependedHistory ignores stray external metadata without observed source", () => {
  const stitched = stitchPrependedHistory(
    [entry(1, "assistant", "ordinary ", {
      source: "provider_output",
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
      historyFragmentEnd: 9,
      historyTotalChars: 14,
    })],
    [entry(2, "assistant", "reply", {
      historyEntryIndex: 8,
      historyFragmentStart: 9,
      historyFragmentEnd: 14,
      historyTotalChars: 14,
    })],
  )

  assert.equal(stitched.length, 1)
  assert.equal(stitched[0]?.source, "provider_output")
  assert.equal(stitched[0]?.externalProvider, undefined)
  assert.equal(stitched[0]?.externalProviderSessionId, undefined)
  assert.equal(stitched[0]?.externalProviderTurnId, undefined)
  assert.equal(stitched[0]?.observedAtMs, undefined)
  assert.equal(stitched[0]?.externalObservation, undefined)
})

test("stitchPrependedHistory rebuilds structured tool fragments", () => {
  const toolPayload = JSON.stringify({
    id: "tool-1",
    tool: "bash",
    status: "completed",
    input: { command: "pnpm test" },
    output: "ok",
  })
  const splitAt = Math.floor(toolPayload.length / 2)

  const stitched = stitchPrependedHistory(
    [entry(1, "tool", toolPayload.slice(0, splitAt), {
      sourceText: toolPayload.slice(0, splitAt),
      historyEntryIndex: 9,
      historyFragmentStart: 0,
      historyFragmentEnd: splitAt,
      historyTotalChars: toolPayload.length,
      mergeKey: "stale",
    })],
    [entry(2, "tool", toolPayload.slice(splitAt), {
      sourceText: toolPayload.slice(splitAt),
      historyEntryIndex: 9,
      historyFragmentStart: splitAt,
      historyFragmentEnd: toolPayload.length,
      historyTotalChars: toolPayload.length,
      mergeKey: "stale",
    })],
  )

  assert.equal(stitched.length, 1)
  assert.equal(stitched[0]?.mergeKey, "tool-1")
  assert.match(stitched[0]?.text ?? "", /\*\*bash\*\*/)
  assert.match(stitched[0]?.text ?? "", /pnpm test/)
  assert.equal(stitched[0]?.sourceText, toolPayload)
})

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
