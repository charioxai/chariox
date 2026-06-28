import assert from "node:assert/strict"
import test from "node:test"

import {
  prependTranscriptEntriesWithoutDuplicateRenderableLineage,
  stripTranscriptDisplayOnlyEntries,
  transcriptEntriesContainRenderableLineage,
  transcriptEntriesShareRenderableLineage,
  transcriptEntryIsDisplayOnly,
  transcriptEntryIsRenderable,
  transcriptEntryLineageKeys,
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
