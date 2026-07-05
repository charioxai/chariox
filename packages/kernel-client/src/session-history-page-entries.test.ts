import assert from "node:assert/strict"
import test from "node:test"

import type { SessionHistoryEntry, SessionHistoryPageEntry } from "./kernel-types.js"
import {
  cloneSessionHistoryEntry,
  mergeAdjacentSessionHistoryPageEntries,
} from "./session-history-page-entries.js"

test("adjacent session history page entries merge only touching fragments of the same kind", () => {
  const merged = mergeAdjacentSessionHistoryPageEntries([
    pageEntry(1, 0, 5, { kind: "provider_output", text: "hello" }),
    pageEntry(1, 5, 11, { kind: "provider_output", text: " world" }),
    pageEntry(1, 11, 12, { kind: "provider_tool", text: "{}" }),
  ])

  assert.equal(merged.length, 2)
  assert.deepEqual(merged[0], pageEntry(1, 0, 11, {
    kind: "provider_output",
    text: "hello world",
  }))
  assert.deepEqual(merged[1], pageEntry(1, 11, 12, {
    kind: "provider_tool",
    text: "{}",
  }))
})

test("adjacent session history page entries preserve identity fields and richer attachments", () => {
  const merged = mergeAdjacentSessionHistoryPageEntries([
    pageEntry(7, 0, 4, {
      kind: "user_prompt",
      text: "look",
      agent_id: "agent-1",
      provider_run_id: "run-1",
      merge_key: "prompt-1",
      source_attachment_id: "attachment-1",
      attachments: [{
        url: "file:///tmp/image.png",
        mime: "image/png",
        filename: "attachment",
      }],
    }),
    pageEntry(7, 4, 8, {
      kind: "user_prompt",
      text: " now",
      attachments: [{
        url: "file:///tmp/image.png",
        mime: "image/png",
        filename: "image.png",
        preview_url: "file:///tmp/preview.png",
      }],
    }),
  ])

  assert.deepEqual(merged[0]?.entry, {
    kind: "user_prompt",
    text: "look now",
    agent_id: "agent-1",
    provider_run_id: "run-1",
    merge_key: "prompt-1",
    source_attachment_id: "attachment-1",
    attachments: [{
      url: "file:///tmp/image.png",
      mime: "image/png",
      filename: "image.png",
      preview_url: "file:///tmp/preview.png",
    }],
  })
})

test("adjacent session history page entries upgrade matching attachments without dropping extra chips", () => {
  const merged = mergeAdjacentSessionHistoryPageEntries([
    pageEntry(0, 0, 3, {
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
    }),
    pageEntry(0, 3, 6, {
      kind: "user_prompt",
      text: "lo\n",
      attachments: [{
        url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
        mime: "image/png",
        filename: "Screenshot.png",
        preview_url: "data:image/png;base64,aW1hZ2U=",
      }],
    }),
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

test("adjacent session history page entries merge external observation settlement metadata", () => {
  const merged = mergeAdjacentSessionHistoryPageEntries([
    pageEntry(3, 0, 4, {
      kind: "provider_status",
      text: "work",
      source: "external_provider_observed",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "turn-1",
      observed_at_ms: 1_000,
      external_observation: {
        settles_active_prompt: false,
        passive_telemetry: true,
      },
    }),
    pageEntry(3, 4, 8, {
      kind: "provider_status",
      text: " done",
      external_observation: {
        settles_active_prompt: true,
        passive_telemetry: false,
      },
    }),
  ])

  assert.deepEqual(merged[0]?.entry, {
    kind: "provider_status",
    text: "work done",
    source: "external_provider_observed",
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "turn-1",
    observed_at_ms: 1_000,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  })
})

test("session history entry clone does not reuse attachment objects", () => {
  const attachment = {
    url: "file:///tmp/a.txt",
    mime: "text/plain",
    filename: "a.txt",
  }
  const cloned = cloneSessionHistoryEntry({
    kind: "user_prompt",
    text: "open",
    attachments: [attachment],
    timestamp_ms: 10,
  })

  assert.deepEqual(cloned, {
    kind: "user_prompt",
    text: "open",
    attachments: [attachment],
    timestamp_ms: 10,
  })
  assert.notEqual(cloned.attachments?.[0], attachment)
})

function pageEntry(
  entryIndex: number,
  fragmentStart: number,
  fragmentEnd: number,
  entry: SessionHistoryEntry,
): SessionHistoryPageEntry {
  return {
    entry_index: entryIndex,
    fragment_start: fragmentStart,
    fragment_end: fragmentEnd,
    total_chars: fragmentEnd,
    entry,
  }
}
