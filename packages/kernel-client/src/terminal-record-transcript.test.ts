import assert from "node:assert/strict"
import test from "node:test"

import {
  terminalRecordIsPassiveExternalProviderTelemetry,
  terminalRecordPromptHistoryText,
  terminalRecordProviderStatusShouldRender,
  terminalRecordTranscriptProjection,
  terminalRecordTranscriptMetadata,
  transcriptEntryWithTerminalMetadata,
} from "./terminal-record-transcript.js"

test("terminalRecordTranscriptMetadata projects prompt and source attachment identity", () => {
  assert.deepEqual(terminalRecordTranscriptMetadata({
    kind: "prompt_echo",
    prompt_id: "prompt-1",
    source_attachment_id: "attachment-1",
  }), {
    promptId: "prompt-1",
    sourceAttachmentId: "attachment-1",
  })
})

test("terminalRecordTranscriptMetadata projects external observed metadata", () => {
  assert.deepEqual(terminalRecordTranscriptMetadata({
    kind: "provider_output",
    source: "external_provider_observed",
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "item-1",
    observed_at_ms: 123,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), {
    source: "external_provider_observed",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "item-1",
    observedAtMs: 123,
    externalObservation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  })
})

test("transcriptEntryWithTerminalMetadata applies only present metadata", () => {
  const entry = transcriptEntryWithTerminalMetadata({
    role: "assistant",
    text: "reply",
    sourceAttachmentId: "existing",
  }, {
    promptId: "prompt-1",
    source: "external_provider_observed",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  })

  assert.deepEqual(entry, {
    role: "assistant",
    text: "reply",
    sourceAttachmentId: "existing",
    promptId: "prompt-1",
    source: "external_provider_observed",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  })
})

test("terminalRecordProviderStatusShouldRender uses external observed status policy", () => {
  assert.equal(terminalRecordProviderStatusShouldRender({
    kind: "provider_status",
    source: "external_provider_observed",
    external_provider: "opencode",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "reconnecting",
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: false,
    },
  }, "OpenCode status: reconnecting", () => false), true)

  assert.equal(terminalRecordProviderStatusShouldRender({
    kind: "provider_status",
    source: "external_provider_observed",
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "token-count",
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }, "codex token_count", () => true), false)
})

test("terminalRecordProviderStatusShouldRender delegates ordinary statuses to fallback", () => {
  assert.equal(terminalRecordProviderStatusShouldRender({
    kind: "provider_status",
  }, "OpenCode is thinking", () => true), true)
  assert.equal(terminalRecordProviderStatusShouldRender({
    kind: "provider_status",
  }, "OpenCode is idle", () => false), false)
})

test("terminalRecordIsPassiveExternalProviderTelemetry follows observed metadata", () => {
  assert.equal(terminalRecordIsPassiveExternalProviderTelemetry({
    kind: "provider_status",
    source: "external_provider_observed",
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }), true)
  assert.equal(terminalRecordIsPassiveExternalProviderTelemetry({
    kind: "provider_status",
    source: "external_provider_observed",
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), false)
})

test("terminalRecordTranscriptProjection classifies external history refresh without transcript work", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: "provider_status",
    source: "external_provider_observed",
  }, "external_provider_history_updated", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => true,
  })

  assert.equal(projection.historyRefreshSignal, true)
  assert.equal(projection.passiveExternalTelemetry, false)
  assert.equal(projection.startsStreaming, false)
  assert.equal(projection.marksAgentBusy, false)
  assert.equal(projection.transcriptRole, "status")
  assert.equal(projection.statusMergeKey, null)
})

test("terminalRecordTranscriptProjection suppresses passive external telemetry", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: "provider_status",
    source: "external_provider_observed",
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "token-count",
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }, "codex token_count", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => true,
  })

  assert.equal(projection.passiveExternalTelemetry, true)
  assert.equal(projection.startsStreaming, false)
  assert.equal(projection.marksAgentBusy, false)
  assert.equal(projection.renderProviderStatus, false)
  assert.equal(projection.metadata.externalProvider, "codex")
})

test("terminalRecordTranscriptProjection keeps idle provider status from marking a turn busy", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: "provider_status",
  }, "OpenCode is idle.", {
    isProviderIdleStatus: () => true,
    shouldRenderProviderStatus: () => true,
  })

  assert.equal(projection.providerStatusIdle, true)
  assert.equal(projection.startsStreaming, true)
  assert.equal(projection.marksAgentBusy, false)
  assert.equal(projection.renderProviderStatus, true)
})

test("terminalRecordTranscriptProjection keeps ordinary status merge separate from external statuses", () => {
  const ordinary = terminalRecordTranscriptProjection({
    kind: "provider_status",
  }, "OpenCode is thinking", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => true,
  })
  const external = terminalRecordTranscriptProjection({
    kind: "provider_status",
    source: "EXTERNAL_PROVIDER_OBSERVED",
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "status-1",
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: false,
    },
  }, "codex event status", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => true,
  })

  assert.equal(ordinary.statusMergeKey, "__provider_status__")
  assert.equal(external.statusMergeKey, null)
  assert.equal(external.metadata.source, "external_provider_observed")
})

test("terminalRecordTranscriptProjection maps transcript roles, merge keys, and normalized errors", () => {
  const assistant = terminalRecordTranscriptProjection({
    kind: "provider_output",
    merge_key: "reply-1",
  }, "hello", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => false,
  })
  assert.equal(assistant.transcriptRole, "assistant")
  assert.equal(assistant.mergeKey, "reply-1")
  assert.equal(assistant.startsStreaming, true)
  assert.equal(assistant.marksAgentBusy, true)

  const error = terminalRecordTranscriptProjection({
    kind: "provider_error",
  }, "failed\r\n", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => false,
  })
  assert.equal(error.transcriptRole, "error")
  assert.equal(error.transcriptText, "failed")
})

test("terminalRecordPromptHistoryText only accepts user prompt terminal records", () => {
  assert.equal(terminalRecordPromptHistoryText({
    kind: "prompt_echo",
  }, "hello"), "hello")
  assert.equal(terminalRecordPromptHistoryText({
    kind: "provider_output",
  }, "hello"), null)
  assert.equal(terminalRecordPromptHistoryText({
    kind: "provider_status",
    source: "external_provider_observed",
  }, "external_provider_history_updated"), null)
  assert.equal(terminalRecordPromptHistoryText({
    kind: "prompt_echo",
    source: "external_provider_observed",
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }, "token count"), null)
})
