import assert from "node:assert/strict"
import test from "node:test"

import {
  prependTranscriptEntriesWithoutDuplicateRenderableLineage,
  stripTranscriptDisplayOnlyEntries,
  transcriptEntriesContainRenderableLineage,
  transcriptEntriesShareRenderableLineage,
  transcriptEntryIsBlobCollapsible,
  transcriptEntryIsDisplayOnly,
  transcriptEntryIsRenderable,
  transcriptEntryLineageKeys,
  transcriptTurnFinalAssistantEntry,
  transcriptTurnHasCollapsibleBody,
  transcriptTurnIsCollapsible,
} from "./transcript-entry-lineage.js"

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
  assert.equal(transcriptEntryIsBlobCollapsible(turnEntries[2]!), true)
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
    source: "external_provider_observed",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }), [
    "external:codex:thread-1:turn-1:assistant",
    "text:external_provider_observed::assistant:hello",
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

test("transcript entry lineage containment ignores turn toggles", () => {
  assert.equal(transcriptEntriesContainRenderableLineage([{
    role: "assistant",
    text: "answer",
    source: "external_provider_observed",
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
    source: "external_provider_observed",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }], [{
    role: "assistant",
    text: "shorter answer from history",
    source: "external_provider_observed",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  }]), true)

  assert.equal(transcriptEntriesShareRenderableLineage([{
    role: "assistant",
    text: "agent a answer",
    source: "external_provider_observed",
    externalProvider: "codex",
    externalProviderSessionId: "thread-a",
    externalProviderTurnId: "turn-a",
  }], [{
    role: "assistant",
    text: "agent b answer",
    source: "external_provider_observed",
    externalProvider: "codex",
    externalProviderSessionId: "thread-b",
    externalProviderTurnId: "turn-b",
  }]), false)
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
