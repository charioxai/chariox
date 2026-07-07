import assert from "node:assert/strict"
import test from "node:test"

import type {
  SessionHistoryEntry,
  SessionHistoryOutlineAgent,
  SessionHistoryPageEntry,
} from "./kernel-types.js"
import {
  hydrateSessionHistoryOutlineAgentEntries,
  hydrateSessionHistoryTranscriptEntries,
  mergePrependedHistoryTranscriptFragments,
  replaceSessionHistoryBlobPlaceholder,
  resolveSessionHistoryBlobLoadTarget,
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

test("session history transcript hydration scopes repeated merge keys by provider run before a turn exists", () => {
  const entries = hydrateSessionHistoryTranscriptEntries([
    pageEntry(0, "provider_output", "first partial\n", {
      provider_run_id: "run-1",
      merge_key: "assistant",
    }),
    pageEntry(1, "provider_output", "second partial\n", {
      provider_run_id: "run-2",
      merge_key: "assistant",
    }),
  ])

  assert.deepEqual(entries.map((entry) => entry.text), [
    "first partial\n",
    "second partial\n",
  ])
  assert.deepEqual(entries.map((entry) => entry.providerRunId), ["run-1", "run-2"])
})

test("session history transcript hydration keeps adjacent unkeyed assistant entries separate", () => {
  const entries = hydrateSessionHistoryTranscriptEntries([
    pageEntry(1, "provider_output", "first blob\n", { provider_run_id: "run-1" }),
    pageEntry(2, "provider_output", "second blob\n", { provider_run_id: "run-1" }),
  ])

  assert.deepEqual(entries.map((entry) => entry.role), ["assistant", "assistant"])
  assert.deepEqual(entries.map((entry) => entry.text), ["first blob\n", "second blob\n"])
  assert.deepEqual(entries.map((entry) => entry.historyEntryIndex), [1, 2])
})

test("session history transcript hydration marks only head partial fragment deferred", () => {
  const entries = hydrateSessionHistoryTranscriptEntries([
    pageEntry(5, "provider_output", "continued reply\n", {
      merge_key: "reply-1",
    }, {
      fragmentStart: 120,
      fragmentEnd: 240,
      totalChars: 240,
    }),
    pageEntry(6, "notice", "reattached"),
  ])

  assert.equal(entries[0]?.historyDeferred, true)
  assert.equal(entries[1]?.historyDeferred, undefined)
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

test("session history transcript hydration recovers external observed metadata from merge key", () => {
  const entries = hydrateSessionHistoryTranscriptEntries([
    pageEntry(7, "provider_output", "native reply", {
      merge_key: "external:codex:thread-1:item-1",
      source: "external_provider_observed",
    }),
  ])

  assert.equal(entries.length, 1)
  assert.equal(entries[0]?.source, "external_provider_observed")
  assert.equal(entries[0]?.externalProvider, "codex")
  assert.equal(entries[0]?.externalProviderSessionId, "thread-1")
  assert.equal(entries[0]?.externalProviderTurnId, "item-1")
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
    pageEntry(4, "provider_status", "codex token_count {\"total\":43}", {
      source: "external_provider_observed",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "legacy-token-count",
    }),
  ])

  assert.deepEqual(entries.map((entry) => entry.text), [
    "codex event turn_aborted {\"reason\":\"user\"}",
  ])
  assert.equal(entries[0]?.role, "status")
  assert.equal(entries[0]?.source, "external_provider_observed")
})

test("session history transcript hydration keeps multiple external provider statuses", () => {
  const entries = hydrateSessionHistoryTranscriptEntries([
    pageEntry(0, "user_prompt", "external prompt\n", {
      source: "external_provider_observed",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "turn-1",
    }),
    pageEntry(1, "provider_status", "codex turn started", {
      source: "external_provider_observed",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "status-started",
      external_observation: {
        settles_active_prompt: false,
        passive_telemetry: false,
      },
    }),
    pageEntry(2, "provider_status", "codex turn aborted", {
      source: "external_provider_observed",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "status-aborted",
      external_observation: {
        settles_active_prompt: true,
        passive_telemetry: false,
      },
    }),
  ])

  const statuses = entries.filter((entry) => entry.role === "status")
  assert.deepEqual(statuses.map((entry) => entry.text), [
    "codex turn started",
    "codex turn aborted",
  ])
  assert.deepEqual(statuses.map((entry) => entry.externalProviderTurnId), [
    "status-started",
    "status-aborted",
  ])
  assert.deepEqual(statuses.map((entry) => entry.externalObservation), [
    { settles_active_prompt: false, passive_telemetry: false },
    { settles_active_prompt: true, passive_telemetry: false },
  ])
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

test("session history outline hydration carries prompt identity into entries and blob placeholders", () => {
  const entries = hydrateSessionHistoryOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      prompt_origin: "external",
      external_provider: " codex ",
      external_provider_session_id: " thread-1 ",
      external_provider_turn_id: " user-1 ",
      started_at_ms: 1,
      completed_at_ms: 2,
      user_prompt: pageEntry(0, "user_prompt", "build\n"),
      entries: [pageEntry(1, "provider_reasoning", "thinking\n")],
      summary: pageEntry(2, "provider_output", "done\n"),
      blobs: [blob("blob-1", "provider_tool", 3, "tool", "1 tool called")],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  assert.equal(entries.find((entry) => entry.role === "user")?.promptId, "prompt-1")
  assert.equal(entries.find((entry) => entry.role === "user")?.promptOrigin, "external")
  assert.equal(entries.find((entry) => entry.role === "reasoning")?.promptId, "prompt-1")
  assert.equal(entries.find((entry) => entry.role === "reasoning")?.promptOrigin, "external")
  assert.equal(entries.find((entry) => entry.historyBlobId === "blob-1")?.promptId, "prompt-1")
  assert.equal(entries.find((entry) => entry.historyBlobId === "blob-1")?.promptOrigin, "external")
  const prompt = entries.find((entry) => entry.role === "user")
  assert.equal(prompt?.source, "external_provider_observed")
  assert.equal(prompt?.externalProvider, "codex")
  assert.equal(prompt?.externalProviderSessionId, "thread-1")
  assert.equal(prompt?.externalProviderTurnId, "user-1")
  const placeholder = entries.find((entry) => entry.historyBlobId === "blob-1")
  assert.equal(placeholder?.source, "external_provider_observed")
  assert.equal(placeholder?.externalProvider, "codex")
  assert.equal(placeholder?.externalProviderSessionId, "thread-1")
  assert.equal(placeholder?.externalProviderTurnId, "user-1")
})

test("session history outline hydration maps blob kinds to stable transcript roles", () => {
  const entries = hydrateSessionHistoryOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      started_at_ms: 1,
      completed_at_ms: 2,
      user_prompt: pageEntry(0, "user_prompt", "prompt\n"),
      entries: [],
      summary: null,
      blobs: [
        blob("blob-output", "provider_output", 1, "assistant", "assistant summary"),
        blob("blob-reasoning", "provider_reasoning", 2, "reasoning", "reasoning summary"),
        blob("blob-tool", "provider_tool", 3, "tool", "tool summary"),
        blob("blob-error", "provider_error", 4, "error", "error summary"),
        blob("blob-status", "provider_status", 5, "status", "status summary"),
        blob("blob-notice", "notice", 6, "notice", "notice summary"),
      ],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  assert.deepEqual(
    entries
      .filter((entry) => entry.historyBlobId)
      .map((entry) => [entry.historyBlobId, entry.role]),
    [
      ["blob-output", "tool"],
      ["blob-reasoning", "reasoning"],
      ["blob-tool", "tool"],
      ["blob-error", "error"],
      ["blob-status", "status"],
      ["blob-notice", "status"],
    ],
  )
})

test("session history blob load target resolves only expandable unloaded placeholders", () => {
  const placeholder = transcriptEntry(1, "tool", "placeholder", {
    historyBlobId: "blob-1",
    historyBlobAgentId: "agent-1",
  })

  assert.deepEqual(resolveSessionHistoryBlobLoadTarget(placeholder, false), {
    agentId: "agent-1",
    blobId: "blob-1",
  })
  const { historyBlobAgentId: _agentId, ...withoutAgentId } = placeholder
  const { historyBlobId: _blobId, ...withoutBlobId } = placeholder
  assert.equal(resolveSessionHistoryBlobLoadTarget(placeholder, true), null)
  assert.equal(resolveSessionHistoryBlobLoadTarget({ ...placeholder, historyBlobLoaded: true }, false), null)
  assert.equal(resolveSessionHistoryBlobLoadTarget({ ...placeholder, historyBlobLoading: true }, false), null)
  assert.equal(resolveSessionHistoryBlobLoadTarget(withoutAgentId, false), null)
  assert.equal(resolveSessionHistoryBlobLoadTarget(withoutBlobId, false), null)
  assert.equal(resolveSessionHistoryBlobLoadTarget(null, false), null)
})

test("session history outline hydration orders turns and keeps stable turn ids", () => {
  const currentOnlyEntries = hydrateSessionHistoryOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [outlineTurn(20, "prompt-current", "current prompt\n", "current reply\n")],
    next_cursor: null,
  })
  const currentOnlyTurnId = currentOnlyEntries.find((entry) => entry.promptId === "prompt-current" && entry.role === "user")?.turnId

  const prependedEntries = hydrateSessionHistoryOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [
      outlineTurn(10, "prompt-older", "older prompt\n", "older reply\n"),
      outlineTurn(20, "prompt-current", "current prompt\n", "current reply\n"),
    ],
    next_cursor: null,
  })
  const semanticEntries = prependedEntries.filter((entry) => entry.text !== "click to collapse")
  const prependedTurnId = prependedEntries.find((entry) => entry.promptId === "prompt-current" && entry.role === "user")?.turnId

  assert.deepEqual(semanticEntries.map((entry) => entry.text.trim()), [
    "older prompt",
    "older reply",
    "current prompt",
    "current reply",
  ])
  assert.equal(currentOnlyTurnId, 21)
  assert.equal(prependedTurnId, currentOnlyTurnId)
})

test("session history outline hydration keeps incomplete external turns active", () => {
  const entries = hydrateSessionHistoryOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      prompt_origin: "external",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "user-1",
      started_at_ms: 1,
      completed_at_ms: null,
      user_prompt: pageEntry(0, "user_prompt", "external prompt\n"),
      entries: [pageEntry(1, "provider_reasoning", "still thinking\n")],
      summary: pageEntry(2, "provider_output", "partial assistant\n"),
      blobs: [blob("blob-1", "provider_tool", 3, "tool", "running tool")],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  assert.equal(entries.find((entry) => entry.role === "turn_toggle"), undefined)
  assert.deepEqual(entries.filter((entry) => !entry.hidden).map((entry) => entry.role), [
    "user",
    "reasoning",
    "assistant",
    "tool",
  ])
  assert.equal(entries.find((entry) => entry.role === "user")?.historyTurnCompletedAtMs, null)
  assert.equal(entries.find((entry) => entry.role === "assistant")?.historyTurnCompletedAtMs, null)
})

test("session history outline hydration treats invalid completion markers as incomplete", () => {
  const entries = hydrateSessionHistoryOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      started_at_ms: 1,
      completed_at_ms: Number.NaN,
      user_prompt: pageEntry(0, "user_prompt", "external prompt\n"),
      entries: [pageEntry(1, "provider_reasoning", "still thinking\n")],
      summary: pageEntry(2, "provider_output", "partial assistant\n"),
      blobs: [blob("blob-1", "provider_tool", 3, "tool", "running tool")],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  assert.equal(entries.find((entry) => entry.role === "turn_toggle"), undefined)
  assert.deepEqual(entries.filter((entry) => !entry.hidden).map((entry) => entry.role), [
    "user",
    "reasoning",
    "assistant",
    "tool",
  ])
  assert.equal(entries.find((entry) => entry.role === "user")?.historyTurnCompletedAtMs, null)
  assert.equal(entries.find((entry) => entry.role === "assistant")?.historyTurnCompletedAtMs, null)
})

test("session history outline hydration projects sparse external turn metadata", () => {
  const entries = hydrateSessionHistoryOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      prompt_origin: " External ",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: null,
      started_at_ms: 1,
      completed_at_ms: 2,
      user_prompt: pageEntry(0, "user_prompt", "external prompt\n"),
      entries: [pageEntry(1, "provider_output", "external reply\n")],
      summary: null,
      blobs: [blob("blob-1", "provider_reasoning", 2, "thinking", "reasoning")],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  const prompt = entries.find((entry) => entry.role === "user")
  const assistant = entries.find((entry) => entry.role === "assistant")
  const placeholder = entries.find((entry) => entry.historyBlobId === "blob-1")
  assert.equal(prompt?.source, "external_provider_observed")
  assert.equal(prompt?.promptOrigin, "external")
  assert.equal(assistant?.source, "external_provider_observed")
  assert.equal(assistant?.promptOrigin, "external")
  assert.equal(placeholder?.source, "external_provider_observed")
  assert.equal(placeholder?.promptOrigin, "external")
  assert.equal(prompt?.externalProvider, "codex")
  assert.equal(prompt?.externalProviderSessionId, "thread-1")
  assert.equal(prompt?.externalProviderTurnId, undefined)
})

test("session history outline hydration does not infer external ownership for arroba-origin turns", () => {
  const entries = hydrateSessionHistoryOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      prompt_origin: "arroba",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "user-1",
      started_at_ms: 1,
      completed_at_ms: 2,
      user_prompt: pageEntry(0, "user_prompt", "arroba prompt\n"),
      entries: [pageEntry(1, "provider_output", "arroba reply\n")],
      summary: null,
      blobs: [blob("blob-1", "provider_tool", 2, "tool", "1 tool called")],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)

  const prompt = entries.find((entry) => entry.role === "user")
  const assistant = entries.find((entry) => entry.role === "assistant")
  const placeholder = entries.find((entry) => entry.historyBlobId === "blob-1")
  assert.equal(prompt?.source, undefined)
  assert.equal(prompt?.promptOrigin, "arroba")
  assert.equal(assistant?.source, undefined)
  assert.equal(assistant?.promptOrigin, "arroba")
  assert.equal(placeholder?.source, undefined)
  assert.equal(placeholder?.promptOrigin, "arroba")
  assert.equal(prompt?.externalProvider, undefined)
  assert.equal(placeholder?.externalProviderSessionId, undefined)
})

test("session history blob replacement keeps incomplete external turns active", () => {
  const entries = hydrateSessionHistoryOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      prompt_origin: "external",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "user-1",
      started_at_ms: 1,
      completed_at_ms: null,
      user_prompt: pageEntry(0, "user_prompt", "external prompt\n"),
      entries: [pageEntry(1, "provider_reasoning", "still thinking\n")],
      summary: pageEntry(3, "provider_output", "partial assistant\n"),
      blobs: [blob("blob-1", "provider_tool", 2, "tool", "running tool")],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)
  const placeholder = entries.find((entry) => entry.historyBlobId === "blob-1")
  assert.ok(placeholder)

  const replaced = replaceSessionHistoryBlobPlaceholder(
    entries,
    placeholder.id,
    {
      blob_id: "blob-1",
      entries: [pageEntry(2, "provider_tool", JSON.stringify({
        id: "tool-1",
        tool: "bash",
        status: "running",
        output: "",
      }))],
    },
    [],
  )

  assert.equal(replaced.find((entry) => entry.role === "turn_toggle"), undefined)
  assert.deepEqual(replaced.filter((entry) => !entry.hidden).map((entry) => entry.role), [
    "user",
    "reasoning",
    "tool",
    "assistant",
  ])
  assert.equal(replaced.find((entry) => entry.role === "tool")?.historyTurnCompletedAtMs, null)
})

test("session history blob replacement preserves prompt and external turn metadata", () => {
  const entries = hydrateSessionHistoryOutlineAgentEntries({
    agent_id: "agent-1",
    turns: [{
      turn_id: "turn-1",
      prompt_id: "prompt-1",
      prompt_origin: "external",
      external_provider: "codex",
      external_provider_session_id: "thread-1",
      external_provider_turn_id: "user-1",
      started_at_ms: 1,
      completed_at_ms: 2,
      user_prompt: pageEntry(0, "user_prompt", "build\n"),
      entries: [],
      summary: pageEntry(2, "provider_output", "done\n"),
      blobs: [blob("blob-1", "provider_tool", 1, "tool", "1 tool called")],
    }],
    next_cursor: null,
  } satisfies SessionHistoryOutlineAgent)
  const placeholder = entries.find((entry) => entry.historyBlobId === "blob-1")
  assert.ok(placeholder)

  const replaced = replaceSessionHistoryBlobPlaceholder(
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

  const tool = replaced.find((entry) => entry.role === "tool")
  assert.equal(tool?.promptId, "prompt-1")
  assert.equal(tool?.promptOrigin, "external")
  assert.equal(tool?.source, "external_provider_observed")
  assert.equal(tool?.externalProvider, "codex")
  assert.equal(tool?.externalProviderSessionId, "thread-1")
  assert.equal(tool?.externalProviderTurnId, "user-1")
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

function outlineTurn(
  entryIndex: number,
  promptId: string,
  promptText: string,
  replyText: string,
) {
  return {
    turn_id: `turn-${entryIndex}`,
    prompt_id: promptId,
    started_at_ms: entryIndex,
    completed_at_ms: entryIndex + 1,
    user_prompt: pageEntry(entryIndex, "user_prompt", promptText),
    entries: [pageEntry(entryIndex + 1, "provider_output", replyText)],
    summary: null,
    blobs: [],
  }
}

function blob(
  blobId: string,
  kind: SessionHistoryEntry["kind"],
  sequenceStart: number,
  title: string,
  summary: string,
) {
  return {
    blob_id: blobId,
    kind,
    title,
    summary,
    sequence_start: sequenceStart,
    sequence_end: sequenceStart,
    entry_count: 1,
    total_chars: 80,
    timestamp_ms: sequenceStart,
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
