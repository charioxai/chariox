import assert from "node:assert/strict"
import test from "node:test"

import type { SessionHistoryOutlineAgent } from "./cli-types.js"
import {
  hydrateOutlineAgentEntries,
  replaceHistoryBlobPlaceholder,
} from "./session-history-outline.js"

test("hydrateOutlineAgentEntries carries turn prompt identity into entries and blob placeholders", () => {
  const entries = hydrateOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      external_provider: " codex ",
      external_provider_session_id: " thread-1 ",
      external_provider_turn_id: " user-1 ",
      started_at_ms: 1,
      user_prompt: pageEntry(0, "user_prompt", "build\n"),
      entries: [pageEntry(1, "provider_reasoning", "thinking\n")],
      summary: pageEntry(2, "provider_output", "done\n"),
      blobs: [{
        blob_id: "blob-1",
        kind: "provider_tool",
        title: "tool",
        summary: "1 tool called",
        sequence_start: 3,
        sequence_end: 4,
        entry_count: 1,
        total_chars: 80,
        timestamp_ms: 1,
      }],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  assert.equal(entries.find((entry) => entry.role === "user")?.promptId, "prompt-1")
  assert.equal(entries.find((entry) => entry.role === "reasoning")?.promptId, "prompt-1")
  assert.equal(entries.find((entry) => entry.historyBlobId === "blob-1")?.promptId, "prompt-1")
  const prompt = entries.find((entry) => entry.role === "user")
  assert.equal(prompt?.source, "external_provider_observed")
  assert.equal(prompt?.externalProvider, "codex")
  assert.equal(prompt?.externalProviderSessionId, "thread-1")
  assert.equal(prompt?.externalProviderTurnId, "user-1")
  const blob = entries.find((entry) => entry.historyBlobId === "blob-1")
  assert.equal(blob?.source, "external_provider_observed")
  assert.equal(blob?.externalProvider, "codex")
  assert.equal(blob?.externalProviderSessionId, "thread-1")
  assert.equal(blob?.externalProviderTurnId, "user-1")
})

test("hydrateOutlineAgentEntries orders turns and turn items by history sequence", () => {
  const entries = hydrateOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-late",
      prompt_id: "prompt-late",
      started_at_ms: 20,
      user_prompt: pageEntry(20, "user_prompt", "late prompt\n"),
      entries: [pageEntry(22, "provider_output", "late reply\n")],
      summary: pageEntry(24, "provider_output", "late summary\n"),
      blobs: [{
        blob_id: "blob-late-tool",
        kind: "provider_tool",
        title: "late tool",
        summary: "tool after reply",
        sequence_start: 23,
        sequence_end: 23,
        entry_count: 1,
        total_chars: 20,
        timestamp_ms: 23,
      }],
    }, {
      turn_id: "turn-early",
      prompt_id: "prompt-early",
      started_at_ms: 10,
      user_prompt: pageEntry(10, "user_prompt", "early prompt\n"),
      entries: [pageEntry(12, "provider_output", "early reply\n")],
      summary: null,
      blobs: [{
        blob_id: "blob-early-reasoning",
        kind: "provider_reasoning",
        title: "early thinking",
        summary: "reasoning before reply",
        sequence_start: 11,
        sequence_end: 11,
        entry_count: 1,
        total_chars: 20,
        timestamp_ms: 11,
      }],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  const semanticEntries = entries.filter((entry) => entry.text !== "click to collapse")
  assert.deepEqual(semanticEntries.map((entry) => (entry.text || entry.blobTitle)?.trim()), [
    "early prompt",
    "early thinking",
    "early reply",
    "late prompt",
    "late reply",
    "late tool",
    "late summary",
  ])
  assert.deepEqual(semanticEntries.map((entry) => entry.promptId), [
    "prompt-early",
    "prompt-early",
    "prompt-early",
    "prompt-late",
    "prompt-late",
    "prompt-late",
    "prompt-late",
  ])
})

test("hydrateOutlineAgentEntries keeps turn ids stable when older turns are prepended", () => {
  const currentOnlyEntries = hydrateOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-current",
      prompt_id: "prompt-current",
      started_at_ms: 20,
      user_prompt: pageEntry(20, "user_prompt", "current prompt\n"),
      entries: [pageEntry(21, "provider_output", "current reply\n")],
      summary: null,
      blobs: [],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)
  const currentOnlyTurnId = currentOnlyEntries.find((entry) => entry.promptId === "prompt-current" && entry.role === "user")?.turnId

  const prependedEntries = hydrateOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-older",
      prompt_id: "prompt-older",
      started_at_ms: 10,
      user_prompt: pageEntry(10, "user_prompt", "older prompt\n"),
      entries: [pageEntry(11, "provider_output", "older reply\n")],
      summary: null,
      blobs: [],
    }, {
      turn_id: "turn-current",
      prompt_id: "prompt-current",
      started_at_ms: 20,
      user_prompt: pageEntry(20, "user_prompt", "current prompt\n"),
      entries: [pageEntry(21, "provider_output", "current reply\n")],
      summary: null,
      blobs: [],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)
  const prependedTurnId = prependedEntries.find((entry) => entry.promptId === "prompt-current" && entry.role === "user")?.turnId

  assert.equal(currentOnlyTurnId, 21)
  assert.equal(prependedTurnId, currentOnlyTurnId)
})

test("hydrateOutlineAgentEntries preserves prompt attachments and external observation metadata", () => {
  const entries = hydrateOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "user-1",
      started_at_ms: 1,
      user_prompt: pageEntry(0, "user_prompt", "inspect\n", {
        attachments: [{
          url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
          mime: "image/png",
          filename: "Screenshot.png",
          preview_url: "data:image/png;base64,aW1hZ2U=",
        }],
      }),
      entries: [pageEntry(1, "provider_output", "native reply\n", {
        source: "external_provider_observed",
        external_provider: "codex",
        external_provider_session_id: "thread-1",
        external_provider_turn_id: "msg-1",
        external_observation: {
          settles_active_prompt: true,
          passive_telemetry: false,
        },
      })],
      summary: null,
      blobs: [],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  const prompt = entries.find((entry) => entry.role === "user")
  assert.deepEqual(prompt?.attachments, [{
    url: "arroba-terminal://prompt-attachment/attachment-1/Screenshot.png",
    mime: "image/png",
    filename: "Screenshot.png",
    preview_url: "data:image/png;base64,aW1hZ2U=",
  }])
  const assistant = entries.find((entry) => entry.role === "assistant")
  assert.equal(assistant?.source, "external_provider_observed")
  assert.equal(assistant?.externalProvider, "codex")
  assert.equal(assistant?.externalProviderSessionId, "thread-1")
  assert.equal(assistant?.externalProviderTurnId, "msg-1")
  assert.equal(prompt?.externalProviderTurnId, "user-1")
  assert.deepEqual(assistant?.externalObservation, {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
})

test("hydrateOutlineAgentEntries uses prompt origin to mark external turns with sparse metadata", () => {
  const entries = hydrateOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      prompt_origin: " External ",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: null,
      started_at_ms: 1,
      user_prompt: pageEntry(0, "user_prompt", "external prompt\n"),
      entries: [pageEntry(1, "provider_output", "external reply\n")],
      summary: null,
      blobs: [{
        blob_id: "blob-1",
        kind: "provider_reasoning",
        title: "thinking",
        summary: "reasoning",
        sequence_start: 2,
        sequence_end: 3,
        entry_count: 1,
        total_chars: 80,
        timestamp_ms: 1,
      }],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  const prompt = entries.find((entry) => entry.role === "user")
  const assistant = entries.find((entry) => entry.role === "assistant")
  const blob = entries.find((entry) => entry.historyBlobId === "blob-1")
  assert.equal(prompt?.source, "external_provider_observed")
  assert.equal(assistant?.source, "external_provider_observed")
  assert.equal(blob?.source, "external_provider_observed")
  assert.equal(prompt?.externalProvider, "codex")
  assert.equal(prompt?.externalProviderSessionId, "thread-1")
  assert.equal(prompt?.externalProviderTurnId, undefined)
})

test("hydrateOutlineAgentEntries does not infer external ownership for arroba-origin turns", () => {
  const entries = hydrateOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      prompt_origin: "arroba",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "user-1",
      started_at_ms: 1,
      user_prompt: pageEntry(0, "user_prompt", "arroba prompt\n"),
      entries: [pageEntry(1, "provider_output", "arroba reply\n")],
      summary: null,
      blobs: [{
        blob_id: "blob-1",
        kind: "provider_tool",
        title: "tool",
        summary: "1 tool called",
        sequence_start: 2,
        sequence_end: 3,
        entry_count: 1,
        total_chars: 80,
        timestamp_ms: 1,
      }],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  const prompt = entries.find((entry) => entry.role === "user")
  const assistant = entries.find((entry) => entry.role === "assistant")
  const blob = entries.find((entry) => entry.historyBlobId === "blob-1")
  assert.equal(prompt?.source, undefined)
  assert.equal(assistant?.source, undefined)
  assert.equal(blob?.source, undefined)
  assert.equal(prompt?.externalProvider, undefined)
  assert.equal(blob?.externalProviderSessionId, undefined)
})

test("replaceHistoryBlobPlaceholder keeps prompt identity when expanding blob content", () => {
  const entries = hydrateOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "user-1",
      started_at_ms: 1,
      user_prompt: pageEntry(0, "user_prompt", "build\n"),
      entries: [],
      summary: pageEntry(2, "provider_output", "done\n"),
      blobs: [{
        blob_id: "blob-1",
        kind: "provider_tool",
        title: "tool",
        summary: "1 tool called",
        sequence_start: 1,
        sequence_end: 2,
        entry_count: 1,
        total_chars: 80,
        timestamp_ms: 1,
      }],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)
  const placeholder = entries.find((entry) => entry.historyBlobId === "blob-1")
  assert.ok(placeholder)

  const replaced = replaceHistoryBlobPlaceholder(
    entries,
    placeholder.id,
    {
      blob_id: "blob-1",
      entries: [pageEntry(1, "provider_tool", JSON.stringify({
        id: "tool-1",
        tool: "bash",
        status: "completed",
        output: "ok",
      }))],
    },
    [],
  )

  assert.equal(replaced.find((entry) => entry.role === "tool")?.promptId, "prompt-1")
  const tool = replaced.find((entry) => entry.role === "tool")
  assert.equal(tool?.source, "external_provider_observed")
  assert.equal(tool?.externalProvider, "codex")
  assert.equal(tool?.externalProviderSessionId, "thread-1")
  assert.equal(tool?.externalProviderTurnId, "user-1")
})

function pageEntry(
  entryIndex: number,
  kind: "user_prompt" | "provider_output" | "provider_reasoning" | "provider_tool",
  text: string,
  overrides: Record<string, unknown> = {},
) {
  return {
    entry_index: entryIndex,
    fragment_start: 0,
    fragment_end: text.length,
    total_chars: text.length,
    entry: { kind, text, ...overrides },
  }
}
