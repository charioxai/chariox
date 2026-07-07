import assert from "node:assert/strict"
import test from "node:test"

import {
  appendTranscriptPreviewLine,
  formatSessionHistoryPreview,
  formatTranscriptPreview,
  previewLineForSessionHistoryEntry,
  previewLineForTerminalRecord,
  previewLineForTranscriptEntry,
  sessionHistoryEntryPreviewLabel,
  transcriptEntryPreviewLabel,
} from "./session-history-preview.js"
import {
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
} from "./external-provider-observation.js"

test("session history preview labels all history entry kinds", () => {
  assert.equal(sessionHistoryEntryPreviewLabel("user_prompt"), "You")
  assert.equal(sessionHistoryEntryPreviewLabel("provider_reasoning"), "Think")
  assert.equal(sessionHistoryEntryPreviewLabel("provider_tool"), "Tool")
  assert.equal(sessionHistoryEntryPreviewLabel("provider_error"), "Err")
  assert.equal(sessionHistoryEntryPreviewLabel("provider_status"), "Stat")
  assert.equal(sessionHistoryEntryPreviewLabel("notice"), "Note")
  assert.equal(sessionHistoryEntryPreviewLabel("provider_output"), "Asst")
})

test("session history preview suppresses empty and passive provider status entries", () => {
  assert.equal(previewLineForSessionHistoryEntry({
    kind: "provider_output",
    text: " \r\n ",
  }), null)
  assert.equal(previewLineForSessionHistoryEntry({
    kind: "provider_status",
    text: "OpenCode status: reconnecting",
  }), null)
  assert.equal(previewLineForSessionHistoryEntry({
    kind: "provider_status",
    text: "codex token_count {\"total\":42}",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }), null)
})

test("session history preview renders visible first line", () => {
  assert.equal(previewLineForSessionHistoryEntry({
    kind: "provider_output",
    text: "first\r\nsecond",
  }), "Asst: first")
  assert.equal(previewLineForSessionHistoryEntry({
    kind: "provider_status",
    text: "codex event turn_aborted {\"reason\":\"user\"}",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: true,
    },
  }), "Stat: codex event turn_aborted {\"reason\":\"user\"}")
})

test("transcript preview labels and suppresses non-content entries", () => {
  assert.equal(transcriptEntryPreviewLabel("user"), "You")
  assert.equal(transcriptEntryPreviewLabel("assistant"), "Asst")
  assert.equal(previewLineForTranscriptEntry({
    role: "assistant",
    text: "answer\nmore",
  }), "Asst: answer")
  assert.equal(previewLineForTranscriptEntry({
    role: "assistant",
    text: "answer",
    hidden: true,
  }), null)
  assert.equal(previewLineForTranscriptEntry({
    role: "turn_toggle",
    text: "click to expand",
  }), null)
})

test("transcript preview formatting keeps the latest lines", () => {
  assert.equal(appendTranscriptPreviewLine("one\ntwo", "three", 2), "two\nthree")
  assert.equal(appendTranscriptPreviewLine("", "first", 14), "first")

  assert.equal(formatTranscriptPreview([
    { role: "user", text: "prompt" },
    { role: "turn_toggle", text: "click to expand" },
    { role: "assistant", text: "answer" },
    { role: "assistant", text: "hidden", hidden: true },
  ], 1), "Asst: answer")
})

test("session history preview formatting merges adjacent history and keeps latest lines", () => {
  assert.equal(formatSessionHistoryPreview([
    {
      entry_index: 1,
      fragment_start: 0,
      fragment_end: 5,
      total_chars: 11,
      entry: { kind: "provider_output", text: "hello" },
    },
    {
      entry_index: 2,
      fragment_start: 0,
      fragment_end: 6,
      total_chars: 6,
      entry: { kind: "user_prompt", text: "question" },
    },
  ], 1), "You: question")
})

test("terminal record preview maps provider record kinds to transcript labels", () => {
  assert.equal(previewLineForTerminalRecord("prompt_echo", "hello\nagain"), "You: hello")
  assert.equal(previewLineForTerminalRecord("provider_reasoning", "thinking"), "Think: thinking")
  assert.equal(previewLineForTerminalRecord("provider_tool", "tool"), "Tool: tool")
  assert.equal(previewLineForTerminalRecord("provider_error", "error"), "Err: error")
  assert.equal(previewLineForTerminalRecord("provider_status", "status"), "Stat: status")
  assert.equal(previewLineForTerminalRecord("provider_output", "answer"), "Asst: answer")
  assert.equal(previewLineForTerminalRecord("provider_output", " \n "), "")
})
