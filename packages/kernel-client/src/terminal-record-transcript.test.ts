import assert from "node:assert/strict"
import test from "node:test"

import {
  EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS,
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
} from "./external-provider-observation.js"
import {
  PROVIDER_TERMINAL_OUTPUT_KIND,
  terminalRecordIsPassiveExternalProviderTelemetry,
  terminalRecordPromptHistoryText,
  terminalRecordProviderStatusShouldRender,
  terminalRecordShouldRenderInAgentPane,
  terminalRecordIsSteeringPrompt,
  terminalRecordTranscriptProjection,
  terminalRecordTranscriptMetadata,
  transcriptEntryWithTerminalMetadata,
} from "./terminal-record-transcript.js"

test("provider terminal bytes stay outside semantic transcript and activity", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: PROVIDER_TERMINAL_OUTPUT_KIND,
  }, "\u001b[2JClaude fullscreen redraw", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => true,
  })

  assert.equal(projection.transcriptRole, null)
  assert.equal(projection.transcriptText, "")
  assert.equal(projection.startsStreaming, false)
  assert.equal(projection.marksAgentBusy, false)
  assert.equal(projection.updatesProviderActivity, false)
  assert.equal(projection.appendsLiveTranscript, false)
  assert.equal(projection.renderInAgentPane, false)
  assert.equal(projection.append, false)
  assert.equal(projection.replace, false)
})

test("terminalRecordTranscriptMetadata projects prompt and source attachment identity", () => {
  assert.deepEqual(terminalRecordTranscriptMetadata({
    kind: "prompt_echo",
    prompt_id: "prompt-1",
    prompt_origin: " External ",
    source_attachment_id: "attachment-1",
  }), {
    promptId: "prompt-1",
    promptOrigin: "external",
    sourceAttachmentId: "attachment-1",
  })

  assert.deepEqual(terminalRecordTranscriptMetadata({
    kind: "prompt_echo",
    prompt_id: "external:codex:thread-1:turn-1",
  }), {
    promptId: "external:codex:thread-1:turn-1",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  })

  assert.deepEqual(terminalRecordTranscriptMetadata({
    kind: "prompt_echo",
    external_provider: "codex",
    external_provider_session_id: "thread-1",
  }), {
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  })

  assert.equal(terminalRecordTranscriptMetadata({
    kind: "prompt_echo",
    prompt_id: "external:codex:thread-1:turn-1",
    prompt_origin: "arroba",
  }).promptOrigin, "arroba")
})

test("terminalRecordTranscriptMetadata preserves explicit null prompt identity", () => {
  assert.deepEqual(terminalRecordTranscriptMetadata({
    kind: "provider_output",
    prompt_id: null,
    prompt_origin: null,
    source_attachment_id: null,
  }), {
    promptId: null,
    promptOrigin: null,
    sourceAttachmentId: null,
  })
})

test("terminalRecordTranscriptMetadata normalizes malformed prompt identity to null", () => {
  assert.deepEqual(terminalRecordTranscriptMetadata({
    kind: "provider_output",
    prompt_id: 42,
    prompt_origin: true,
    source_attachment_id: false,
  } as unknown as Parameters<typeof terminalRecordTranscriptMetadata>[0]), {
    promptId: null,
    promptOrigin: null,
    sourceAttachmentId: null,
  })
})

test("terminalRecordTranscriptMetadata projects external observed metadata", () => {
  assert.deepEqual(terminalRecordTranscriptMetadata({
    kind: "provider_output",
    prompt_origin: "external",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "item-1",
    observed_at_ms: 123,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), {
    promptOrigin: "external",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
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
    promptOrigin: "arroba",
    sourceAttachmentId: "existing",
  }, {
    promptId: "prompt-1",
    promptOrigin: "external",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  })

  assert.deepEqual(entry, {
    role: "assistant",
    text: "reply",
    sourceAttachmentId: "existing",
    promptId: "prompt-1",
    promptOrigin: "external",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  })
})

test("terminalRecordProviderStatusShouldRender uses external observed status policy", () => {
  assert.equal(terminalRecordProviderStatusShouldRender({
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
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
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
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
  assert.equal(terminalRecordProviderStatusShouldRender({
    kind: "provider_output",
  }, "ordinary output", () => true), false)
})

test("terminalRecordIsPassiveExternalProviderTelemetry follows observed metadata", () => {
  assert.equal(terminalRecordIsPassiveExternalProviderTelemetry({
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }), true)
  assert.equal(terminalRecordIsPassiveExternalProviderTelemetry({
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), false)
})

test("terminalRecordTranscriptProjection classifies external history refresh without transcript work", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  }, EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS, {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => true,
  })

  assert.equal(projection.historyRefreshSignal, true)
  assert.equal(projection.passiveExternalTelemetry, false)
  assert.equal(projection.startsStreaming, false)
  assert.equal(projection.marksAgentBusy, false)
  assert.equal(projection.updatesProviderActivity, false)
  assert.equal(projection.appendsLiveTranscript, false)
  assert.equal(projection.renderProviderStatus, false)
  assert.equal(projection.transcriptRole, "status")
  assert.equal(projection.statusMergeKey, null)
  assert.equal(projection.renderInAgentPane, false)
  assert.equal(projection.append, false)
  assert.equal(projection.replace, false)
})

test("terminalRecordTranscriptProjection suppresses passive external telemetry", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
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
  assert.equal(projection.updatesProviderActivity, false)
  assert.equal(projection.appendsLiveTranscript, false)
  assert.equal(projection.renderProviderStatus, false)
  assert.equal(projection.metadata.externalProvider, "codex")
  assert.equal(projection.renderInAgentPane, false)
  assert.equal(projection.append, false)
  assert.equal(projection.replace, false)

  const unmarkedProjection = terminalRecordTranscriptProjection({
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "token-count-unmarked",
  }, "codex token_count {\"total\":43}", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => true,
  })
  assert.equal(unmarkedProjection.passiveExternalTelemetry, false)
  assert.equal(unmarkedProjection.startsStreaming, true)
  assert.equal(unmarkedProjection.appendsLiveTranscript, true)
  assert.equal(unmarkedProjection.renderProviderStatus, true)
})

test("terminalRecordTranscriptProjection does not treat ordinary status as external telemetry", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: "provider_status",
    external_provider: "codex",
  }, "codex token_count {\"total\":42}", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => true,
  })

  assert.equal(projection.passiveExternalTelemetry, false)
  assert.equal(projection.renderProviderStatus, true)
  assert.equal(projection.renderInAgentPane, false)
  assert.equal(projection.startsStreaming, true)
  assert.equal(projection.updatesProviderActivity, true)
  assert.equal(projection.appendsLiveTranscript, false)
  assert.equal(projection.statusMergeKey, "__provider_status__")
})

test("terminalRecordTranscriptProjection keeps idle provider status out of live turn state", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: "provider_status",
  }, "OpenCode is idle.", {
    isProviderIdleStatus: () => true,
    shouldRenderProviderStatus: () => true,
  })

  assert.equal(projection.providerStatusIdle, true)
  assert.equal(projection.startsStreaming, false)
  assert.equal(projection.marksAgentBusy, false)
  assert.equal(projection.updatesProviderActivity, false)
  assert.equal(projection.appendsLiveTranscript, false)
  assert.equal(projection.renderProviderStatus, false)
  assert.equal(projection.renderInAgentPane, false)
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
  assert.equal(external.metadata.source, EXTERNAL_PROVIDER_OBSERVED_SOURCE)
})

test("terminalRecordTranscriptProjection maps transcript roles, merge keys, and normalized errors", () => {
  const userPrompt = terminalRecordTranscriptProjection({
    kind: "user_prompt",
  }, "build it", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => false,
  })
  assert.equal(userPrompt.transcriptRole, "user")
  assert.equal(userPrompt.startsStreaming, false)
  assert.equal(userPrompt.marksAgentBusy, false)
  assert.equal(userPrompt.updatesProviderActivity, false)
  assert.equal(userPrompt.appendsLiveTranscript, true)
  assert.equal(userPrompt.renderInAgentPane, true)
  assert.equal(userPrompt.append, false)
  assert.equal(userPrompt.replace, false)

  const steeringPrompt = terminalRecordTranscriptProjection({
    kind: "prompt_echo",
    merge_key: "steering-prompt:prompt-1",
  }, "steer now", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => false,
  })
  assert.equal(steeringPrompt.steeringPrompt, true)
  assert.equal(terminalRecordIsSteeringPrompt({
    kind: "prompt_echo",
    merge_key: "steering-prompt:prompt-1",
  }), true)
  assert.equal(terminalRecordIsSteeringPrompt({
    kind: "provider_output",
    merge_key: "steering-prompt:prompt-1",
  }), false)

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
  assert.equal(assistant.updatesProviderActivity, false)
  assert.equal(assistant.appendsLiveTranscript, true)
  assert.equal(assistant.renderInAgentPane, true)
  assert.equal(assistant.append, true)
  assert.equal(assistant.replace, false)

  const error = terminalRecordTranscriptProjection({
    kind: "provider_error",
  }, "failed\r\n", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => false,
  })
  assert.equal(error.transcriptRole, "error")
  assert.equal(error.transcriptText, "failed")
  assert.equal(error.renderInAgentPane, true)
  assert.equal(error.append, false)
  assert.equal(error.replace, false)
})

test("terminalRecordTranscriptProjection keeps ordinary provider status activity separate from transcript admission", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: "provider_status",
  }, "OpenCode is thinking", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => true,
  })

  assert.equal(projection.updatesProviderActivity, true)
  assert.equal(projection.renderProviderStatus, true)
  assert.equal(projection.appendsLiveTranscript, false)
  assert.equal(projection.renderInAgentPane, false)
  assert.equal(projection.append, false)
  assert.equal(projection.replace, false)
})

test("terminalRecordTranscriptProjection admits useful external provider statuses as transcript entries", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }, "codex task_complete", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => true,
  })

  assert.equal(projection.updatesProviderActivity, true)
  assert.equal(projection.renderProviderStatus, true)
  assert.equal(projection.appendsLiveTranscript, true)
  assert.equal(projection.renderInAgentPane, true)
  assert.equal(projection.append, false)
  assert.equal(projection.replace, true)
})

test("terminalRecordTranscriptProjection keeps non-rendered provider status out of transcript admission", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: "provider_status",
  }, "OpenCode task_complete", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => false,
  })

  assert.equal(projection.updatesProviderActivity, true)
  assert.equal(projection.renderProviderStatus, false)
  assert.equal(projection.appendsLiveTranscript, false)
  assert.equal(projection.renderInAgentPane, false)
  assert.equal(projection.append, false)
  assert.equal(projection.replace, false)
})

test("terminalRecordTranscriptProjection keeps notices out of terminal record transcript admission", () => {
  const projection = terminalRecordTranscriptProjection({
    kind: "notice",
  }, "reattached", {
    isProviderIdleStatus: () => false,
    shouldRenderProviderStatus: () => true,
  })

  assert.equal(terminalRecordShouldRenderInAgentPane("notice", "reattached"), false)
  assert.equal(projection.startsStreaming, false)
  assert.equal(projection.marksAgentBusy, false)
  assert.equal(projection.updatesProviderActivity, false)
  assert.equal(projection.appendsLiveTranscript, false)
  assert.equal(projection.renderInAgentPane, false)
  assert.equal(projection.append, false)
  assert.equal(projection.replace, false)
})

test("terminalRecordPromptHistoryText only accepts user prompt terminal records", () => {
  assert.equal(terminalRecordPromptHistoryText({
    kind: "prompt_echo",
  }, "hello"), "hello")
  assert.equal(terminalRecordPromptHistoryText({
    kind: "user_prompt",
  }, "from history"), "from history")
  assert.equal(terminalRecordPromptHistoryText({
    kind: "provider_output",
  }, "hello"), null)
  assert.equal(terminalRecordPromptHistoryText({
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  }, EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS), null)
  assert.equal(terminalRecordPromptHistoryText({
    kind: "prompt_echo",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }, "token count"), null)
})
