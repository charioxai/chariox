import assert from "node:assert/strict"
import test from "node:test"

import {
  previewLineForSessionHistoryEntry,
  previewLineForTranscriptEntry,
  sessionHistoryEntryPreviewLabel,
  transcriptEntryPreviewLabel,
} from "./session-history-preview.js"

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
    source: "external_provider_observed",
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
    source: "external_provider_observed",
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
