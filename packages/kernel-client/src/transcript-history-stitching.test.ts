import assert from "node:assert/strict"
import test from "node:test"

import {
  stitchPrependedTranscriptHistory,
  type TranscriptHistoryStitchEntry,
} from "./transcript-history-stitching.js"
import {
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
} from "./external-provider-observation.js"

test("stitchPrependedTranscriptHistory merges adjacent assistant fragments", () => {
  const stitched = stitchPrependedTranscriptHistory(
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

test("stitchPrependedTranscriptHistory preserves prompt attachment metadata", () => {
  const attachments = [{
    url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
    mime: "image/png",
    filename: "Screenshot.png",
    preview_url: "data:image/png;base64,aW1hZ2U=",
  }]
  const stitched = stitchPrependedTranscriptHistory(
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
      promptId: "prompt-1",
      sourceAttachmentId: "attachment-1",
    })],
  )

  assert.equal(stitched.length, 1)
  assert.equal(stitched[0]?.text, "inspect image")
  assert.equal(stitched[0]?.promptId, "prompt-1")
  assert.equal(stitched[0]?.sourceAttachmentId, "attachment-1")
  assert.deepEqual(stitched[0]?.attachments, attachments)
})

test("stitchPrependedTranscriptHistory preserves prompt ownership metadata", () => {
  const stitched = stitchPrependedTranscriptHistory(
    [entry(1, "user", "external ", {
      historyEntryIndex: 8,
      historyFragmentStart: 0,
      historyFragmentEnd: 9,
      historyTotalChars: 15,
      promptId: "prompt-1",
      promptOrigin: "external",
      historyTurnCompletedAtMs: null,
      historyTurnLifecycle: "open",
    })],
    [entry(2, "user", "prompt", {
      historyEntryIndex: 8,
      historyFragmentStart: 9,
      historyFragmentEnd: 15,
      historyTotalChars: 15,
      promptId: "prompt-1",
      promptOrigin: "external",
      historyTurnCompletedAtMs: null,
      historyTurnLifecycle: "open",
    })],
  )

  assert.equal(stitched.length, 1)
  assert.equal(stitched[0]?.text, "external prompt")
  assert.equal(stitched[0]?.promptId, "prompt-1")
  assert.equal(stitched[0]?.promptOrigin, "external")
  assert.equal(stitched[0]?.historyTurnCompletedAtMs, null)
  assert.equal(stitched[0]?.historyTurnLifecycle, "open")
})

test("stitchPrependedTranscriptHistory does not merge conflicting prompt ownership metadata", () => {
  const stitched = stitchPrependedTranscriptHistory(
    [entry(1, "user", "arroba ", {
      historyEntryIndex: 8,
      historyFragmentStart: 0,
      historyFragmentEnd: 7,
      historyTotalChars: 13,
      promptOrigin: "external",
      historyTurnCompletedAtMs: null,
      historyTurnLifecycle: "open",
    })],
    [entry(2, "user", "prompt", {
      historyEntryIndex: 8,
      historyFragmentStart: 7,
      historyFragmentEnd: 13,
      historyTotalChars: 13,
      promptOrigin: "arroba",
      historyTurnCompletedAtMs: 1_000,
      historyTurnLifecycle: "completed",
    })],
  )

  assert.equal(stitched.length, 2)
  assert.equal(stitched[0]?.text, "arroba ")
  assert.equal(stitched[0]?.promptOrigin, "external")
  assert.equal(stitched[1]?.text, "prompt")
  assert.equal(stitched[1]?.promptOrigin, "arroba")
})

test("stitchPrependedTranscriptHistory merges external observed metadata", () => {
  const stitched = stitchPrependedTranscriptHistory(
    [entry(1, "assistant", "native ", {
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
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
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
      externalObservation: {
        settles_active_prompt: true,
        passive_telemetry: false,
      },
      historyEntryIndex: 8,
      historyFragmentStart: 7,
      historyFragmentEnd: 12,
      historyTotalChars: 12,
    })],
  )

  assert.equal(stitched.length, 1)
  assert.equal(stitched[0]?.source, EXTERNAL_PROVIDER_OBSERVED_SOURCE)
  assert.deepEqual(stitched[0]?.externalObservation, {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
})

test("stitchPrependedTranscriptHistory does not merge stale external source fragments", () => {
  const stitched = stitchPrependedTranscriptHistory(
    [entry(1, "assistant", "native ", {
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
      observedAtMs: 1_000,
      historyEntryIndex: 8,
      historyFragmentStart: 0,
      historyFragmentEnd: 7,
      historyTotalChars: 12,
    })],
    [entry(2, "assistant", "reply", {
      source: "provider_output",
      historyEntryIndex: 8,
      historyFragmentStart: 7,
      historyFragmentEnd: 12,
      historyTotalChars: 12,
    })],
  )

  assert.equal(stitched.length, 2)
  assert.equal(stitched[0]?.source, EXTERNAL_PROVIDER_OBSERVED_SOURCE)
  assert.equal(stitched[0]?.externalProvider, "codex")
  assert.equal(stitched[0]?.externalProviderSessionId, "thread-1")
  assert.equal(stitched[0]?.externalProviderTurnId, "turn-1")
  assert.equal(stitched[0]?.observedAtMs, 1_000)
  assert.equal(stitched[1]?.source, "provider_output")
  assert.equal(stitched[1]?.externalProvider, undefined)
})

test("stitchPrependedTranscriptHistory does not merge sparse null external identity fragments", () => {
  const stitched = stitchPrependedTranscriptHistory(
    [entry(1, "assistant", "native ", {
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
      observedAtMs: 1_000,
      historyEntryIndex: 8,
      historyFragmentStart: 0,
      historyFragmentEnd: 7,
      historyTotalChars: 12,
    })],
    [entry(2, "assistant", "reply", {
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      externalProvider: null,
      externalProviderSessionId: null,
      externalProviderTurnId: null,
      observedAtMs: null,
      historyEntryIndex: 8,
      historyFragmentStart: 7,
      historyFragmentEnd: 12,
      historyTotalChars: 12,
    })],
  )

  assert.equal(stitched.length, 2)
  assert.equal(stitched[0]?.source, EXTERNAL_PROVIDER_OBSERVED_SOURCE)
  assert.equal(stitched[0]?.externalProvider, "codex")
  assert.equal(stitched[0]?.externalProviderSessionId, "thread-1")
  assert.equal(stitched[0]?.externalProviderTurnId, "turn-1")
  assert.equal(stitched[0]?.observedAtMs, 1_000)
  assert.equal(stitched[1]?.externalProvider, null)
})

test("stitchPrependedTranscriptHistory does not recover external identity split across fragments", () => {
  const stitched = stitchPrependedTranscriptHistory(
    [entry(1, "assistant", "native ", {
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
      observedAtMs: 1_000,
      historyEntryIndex: 8,
      historyFragmentStart: 0,
      historyFragmentEnd: 7,
      historyTotalChars: 12,
    })],
    [entry(2, "assistant", "reply", {
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      historyEntryIndex: 8,
      historyFragmentStart: 7,
      historyFragmentEnd: 12,
      historyTotalChars: 12,
    })],
  )

  assert.equal(stitched.length, 2)
  assert.equal(stitched[0]?.source, undefined)
  assert.equal(stitched[0]?.externalProvider, "codex")
  assert.equal(stitched[0]?.externalProviderSessionId, "thread-1")
  assert.equal(stitched[0]?.externalProviderTurnId, "turn-1")
  assert.equal(stitched[0]?.observedAtMs, 1_000)
  assert.equal(stitched[1]?.source, EXTERNAL_PROVIDER_OBSERVED_SOURCE)
  assert.equal(stitched[1]?.externalProvider, undefined)
})

test("stitchPrependedTranscriptHistory does not merge stray external metadata without observed source", () => {
  const stitched = stitchPrependedTranscriptHistory(
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

  assert.equal(stitched.length, 2)
  assert.equal(stitched[0]?.source, "provider_output")
  assert.equal(stitched[0]?.externalProvider, "codex")
  assert.equal(stitched[0]?.externalProviderSessionId, "thread-1")
  assert.equal(stitched[0]?.externalProviderTurnId, "turn-1")
  assert.equal(stitched[0]?.observedAtMs, 1_000)
  assert.equal(stitched[1]?.externalProvider, undefined)
})

test("stitchPrependedTranscriptHistory does not merge conflicting external turn fragments", () => {
  const stitched = stitchPrependedTranscriptHistory(
    [entry(1, "assistant", "native ", {
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-1",
      historyEntryIndex: 8,
      historyFragmentStart: 0,
      historyFragmentEnd: 7,
      historyTotalChars: 12,
    })],
    [entry(2, "assistant", "reply", {
      source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
      externalProvider: "codex",
      externalProviderSessionId: "thread-1",
      externalProviderTurnId: "turn-2",
      externalObservation: {
        settles_active_prompt: true,
        passive_telemetry: false,
      },
      historyEntryIndex: 8,
      historyFragmentStart: 7,
      historyFragmentEnd: 12,
      historyTotalChars: 12,
    })],
  )

  assert.equal(stitched.length, 2)
  assert.equal(stitched[0]?.text, "native ")
  assert.equal(stitched[0]?.externalProviderTurnId, "turn-1")
  assert.equal(stitched[1]?.text, "reply")
  assert.equal(stitched[1]?.externalProviderTurnId, "turn-2")
  assert.deepEqual(stitched[1]?.externalObservation, {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
})

test("stitchPrependedTranscriptHistory rebuilds structured tool fragments", () => {
  const toolPayload = JSON.stringify({
    id: "tool-1",
    tool: "bash",
    status: "completed",
    input: { command: "pnpm test" },
    output: "ok",
  })
  const splitAt = Math.floor(toolPayload.length / 2)

  const stitched = stitchPrependedTranscriptHistory(
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
  role: string,
  text: string,
  overrides: Partial<TranscriptHistoryStitchEntry> = {},
): TranscriptHistoryStitchEntry {
  return {
    id,
    role,
    text,
    ...overrides,
  }
}
