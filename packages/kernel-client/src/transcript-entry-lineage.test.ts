import assert from "node:assert/strict"
import test from "node:test"

import {
  prependTranscriptEntriesWithoutDuplicateRenderableLineage,
  stripTranscriptDisplayOnlyEntries,
  transcriptEntriesContainRenderableLineage,
  transcriptEntriesShareRenderableLineage,
  transcriptEntryDeduplicationKeys,
  transcriptEntryIsBlobCollapsible,
  transcriptEntryIsDisplayOnly,
  transcriptEntryIsRenderable,
  transcriptEntryLineageKeys,
  transcriptTurnFinalAssistantEntry,
  transcriptTurnHasCollapsibleBody,
  transcriptTurnIsCollapsible,
} from "./transcript-entry-lineage.js"
import {
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
} from "./external-provider-observation.js"

test("transcript display-only helpers classify turn toggles as non-renderable lineage", () => {
  const assistant = { role: "assistant", text: "answer" }
  const toggle = { role: "turn_toggle", text: "click to expand" }

  assert.equal(transcriptEntryIsDisplayOnly(toggle), true)
  assert.equal(transcriptEntryIsRenderable(toggle), false)
  assert.equal(transcriptEntryIsDisplayOnly(assistant), false)
  assert.equal(transcriptEntryIsRenderable(assistant), true)
  assert.deepEqual(stripTranscriptDisplayOnlyEntries([assistant, toggle]), [assistant])
})

test("transcript display helpers classify collapsible blobs and turns", () => {
  const turnEntries = [
    { id: 1, role: "user", text: "prompt", turnId: 1 },
    { id: 2, role: "reasoning", text: "thinking", turnId: 1 },
    { id: 3, role: "tool", text: "tool", turnId: 1 },
    { id: 4, role: "assistant", text: "summary", turnId: 1 },
  ]

  assert.equal(transcriptEntryIsBlobCollapsible(turnEntries[0]!), false)
  assert.equal(transcriptEntryIsBlobCollapsible(turnEntries[1]!), true)
  assert.equal(transcriptEntryIsBlobCollapsible(turnEntries[2]!), true)
  assert.equal(transcriptEntryIsBlobCollapsible({ id: 5, role: "error", text: "visible error" }), false)
  assert.equal(transcriptEntryIsBlobCollapsible({ id: 5, role: "assistant", text: "blob", historyBlobId: "blob-1" }), true)
  assert.equal(transcriptTurnFinalAssistantEntry(turnEntries), turnEntries[3])
  assert.equal(transcriptTurnHasCollapsibleBody(turnEntries, 4), true)
  assert.equal(transcriptTurnIsCollapsible(turnEntries), true)
  assert.equal(transcriptTurnIsCollapsible(turnEntries, 1), false)
  assert.equal(transcriptTurnIsCollapsible([
    { id: 1, role: "user", text: "prompt", turnId: 2 },
    { id: 2, role: "assistant", text: "summary", turnId: 2 },
  ]), false)
})

test("transcript entry lineage keys prefer durable external observed identity", () => {
  assert.deepEqual(transcriptEntryLineageKeys({
    role: "assistant",
    text: "hello",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }), [
    "external:codex:thread-1:turn-1:assistant",
    "text:external_provider_observed:external:codex:thread-1:turn-1:assistant:hello",
  ])
})

test("transcript entry lineage keys normalize external observed identity", () => {
  assert.deepEqual(transcriptEntryLineageKeys({
    role: "assistant",
    text: "hello",
    source: " EXTERNAL_PROVIDER_OBSERVED ",
    externalProvider: " CODEX ",
    externalProviderSessionId: " thread-1 ",
    externalProviderTurnId: " turn-1 ",
  }), [
    "external:codex:thread-1:turn-1:assistant",
    "text:external_provider_observed:external:codex:thread-1:turn-1:assistant:hello",
  ])
})

test("transcript entry lineage keys ignore provider-only external observed identity", () => {
  assert.deepEqual(transcriptEntryLineageKeys({
    role: "assistant",
    text: "hello",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
  }), [
    "text:external_provider_observed::assistant:hello",
  ])
})

test("transcript entry lineage keys ignore partial external observed identities", () => {
  assert.deepEqual(transcriptEntryLineageKeys({
    role: "assistant",
    text: "hello",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProviderSessionId: "thread-1",
  }), [
    "text:external_provider_observed::assistant:hello",
  ])
  assert.deepEqual(transcriptEntryLineageKeys({
    role: "assistant",
    text: "hello",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  }), [
    "text:external_provider_observed::assistant:hello",
  ])
})

test("transcript entry lineage keys treat external-looking prompt ids as opaque prompt identity", () => {
  assert.deepEqual(transcriptEntryLineageKeys({
    role: "assistant",
    text: "hello",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    promptId: "external:codex:thread-1:turn-1",
  }), [
    "text:external_provider_observed:prompt:external:codex:thread-1:turn-1:assistant:hello",
  ])
})

test("transcript entry lineage keys include durable history blob identity", () => {
  assert.deepEqual(transcriptEntryLineageKeys({
    role: "assistant",
    text: "expanded blob",
    turnId: 3,
    historyBlobSourceAgentId: "agent-a",
    historyBlobSourceId: "blob-1",
  }), [
    "blob:agent-a:blob-1:3:assistant",
    "turn::3:assistant",
    "text::3:assistant:expanded blob",
  ])
})

test("transcript entry lineage keys prefer prompt identity over display turn identity", () => {
  assert.deepEqual(transcriptEntryLineageKeys({
    role: "assistant",
    text: "answer",
    turnId: 3,
    promptId: "prompt-1",
  }), [
    "turn::prompt:prompt-1:assistant",
    "text::prompt:prompt-1:assistant:answer",
  ])
})

test("transcript entry deduplication keys avoid broad turn-only identity", () => {
  assert.deepEqual(transcriptEntryDeduplicationKeys({
    role: "assistant",
    text: "hello",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }), [
    "text:external_provider_observed:external:codex:thread-1:turn-1:assistant:hello",
  ])

  assert.deepEqual(transcriptEntryDeduplicationKeys({
    role: "tool",
    text: "expanded blob",
    turnId: 3,
    historyBlobSourceAgentId: "agent-a",
    historyBlobSourceId: "blob-1",
  }), [
    "blob:agent-a:blob-1:3:tool",
    "text::3:tool:expanded blob",
  ])
})

test("transcript entry lineage distinguishes reused display turns by prompt identity", () => {
  assert.equal(transcriptEntriesShareRenderableLineage([{
    role: "assistant",
    text: "same text",
    turnId: 1,
    promptId: "prompt-a",
  }], [{
    role: "assistant",
    text: "same text",
    turnId: 1,
    promptId: "prompt-b",
  }]), false)
})

test("transcript entry lineage containment ignores turn toggles", () => {
  assert.equal(transcriptEntriesContainRenderableLineage([{
    role: "assistant",
    text: "answer",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }], [{
    role: "turn_toggle",
    text: "collapsed",
  }]), true)
})

test("transcript entry lineage detects shared refreshed/current entries", () => {
  assert.equal(transcriptEntriesShareRenderableLineage([{
    role: "assistant",
    text: "live answer",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }], [{
    role: "assistant",
    text: "shorter answer from history",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }]), true)

  assert.equal(transcriptEntriesShareRenderableLineage([{
    role: "assistant",
    text: "agent a answer",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-a",
    externalProviderTurnId: "turn-a",
  }], [{
    role: "assistant",
    text: "agent b answer",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-b",
    externalProviderTurnId: "turn-b",
  }]), false)
})

test("transcript entry lineage matches refreshed external entries with normalized identity", () => {
  assert.equal(transcriptEntriesShareRenderableLineage([{
    role: "assistant",
    text: "live answer",
    source: " EXTERNAL_PROVIDER_OBSERVED ",
    externalProvider: " CODEX ",
    externalProviderSessionId: " thread-1 ",
    externalProviderTurnId: " turn-1 ",
  }], [{
    role: "assistant",
    text: "history answer",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }]), true)
})

test("transcript entry lineage rejects blob fallback when exact external identities conflict", () => {
  const current = [{
    role: "assistant",
    text: "same blob text",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
    historyBlobSourceAgentId: "agent-a",
    historyBlobSourceId: "provider-blob-1",
    turnId: 1,
  }]
  const refreshed = [{
    role: "assistant",
    text: "same blob text",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-2",
    historyBlobSourceAgentId: "agent-a",
    historyBlobSourceId: "provider-blob-1",
    turnId: 1,
  }]

  assert.equal(transcriptEntriesShareRenderableLineage(current, refreshed), false)
  assert.equal(transcriptEntriesContainRenderableLineage(current, refreshed), false)
})

test("transcript entry lineage prepending skips duplicates without dropping unique older entries", () => {
  const entries = prependTranscriptEntriesWithoutDuplicateRenderableLineage([{
    role: "assistant",
    text: "older unique",
    turnId: 1,
  }, {
    role: "assistant",
    text: "current duplicate",
    turnId: 2,
  }, {
    role: "turn_toggle",
    text: "toggle",
    turnId: 2,
  }], [{
    role: "assistant",
    text: "current duplicate",
    turnId: 2,
  }])

  assert.deepEqual(entries.map((entry) => entry.text), [
    "older unique",
    "toggle",
    "current duplicate",
  ])
})

test("transcript entry lineage prepending keeps blob fallback entries with conflicting external identities", () => {
  const entries = prependTranscriptEntriesWithoutDuplicateRenderableLineage([{
    role: "assistant",
    text: "same blob text",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
    historyBlobSourceAgentId: "agent-a",
    historyBlobSourceId: "provider-blob-1",
    turnId: 1,
  }], [{
    role: "assistant",
    text: "same blob text",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-2",
    historyBlobSourceAgentId: "agent-a",
    historyBlobSourceId: "provider-blob-1",
    turnId: 1,
  }])

  assert.deepEqual(entries.map((entry) => entry.externalProviderTurnId), [
    "turn-1",
    "turn-2",
  ])
})

test("transcript entry lineage prepending preserves distinct external blobs in the same turn", () => {
  const entries = prependTranscriptEntriesWithoutDuplicateRenderableLineage([{
    role: "assistant",
    text: "first assistant blob",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }, {
    role: "assistant",
    text: "second assistant blob",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }], [{
    role: "assistant",
    text: "second assistant blob",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }])

  assert.deepEqual(entries.map((entry) => entry.text), [
    "first assistant blob",
    "second assistant blob",
  ])
})

test("transcript entry lineage prepending preserves repeated external text from different provider turns", () => {
  const entries = prependTranscriptEntriesWithoutDuplicateRenderableLineage([{
    role: "assistant",
    text: "Done",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }, {
    role: "assistant",
    text: "Done",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-2",
  }], [{
    role: "assistant",
    text: "Done",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-2",
  }])

  assert.deepEqual(entries.map((entry) => entry.externalProviderTurnId), [
    "turn-1",
    "turn-2",
  ])
})

test("transcript entry lineage prepending preserves distinct ordinary blobs in the same turn", () => {
  const entries = prependTranscriptEntriesWithoutDuplicateRenderableLineage([{
    role: "tool",
    text: "first tool result",
    turnId: 7,
  }, {
    role: "tool",
    text: "second tool result",
    turnId: 7,
  }], [{
    role: "tool",
    text: "second tool result",
    turnId: 7,
  }])

  assert.deepEqual(entries.map((entry) => entry.text), [
    "first tool result",
    "second tool result",
  ])
})
