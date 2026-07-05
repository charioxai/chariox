import assert from "node:assert/strict"
import test from "node:test"

import {
  EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS,
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  applyExternalProviderObservedTurnMetadata,
  externalProviderObservedCompletionAtMs,
  externalProviderObservedEntryBelongsToImport,
  externalProviderObservedEntryIsPassiveTelemetry,
  externalProviderObservedHistoryRefreshSignal,
  externalProviderObservedProviderStatusShouldRender,
  externalProviderObservedStatusSettlesActivePrompt,
  historyEntryExternalProviderObservedMetadata,
  mergeExternalProviderObservedHistoryFields,
  mergeExternalProviderObservedSource,
  mergeExternalProviderObservedTranscriptFields,
  mergeExternalProviderObservation,
  promptOriginExternalProviderObservedMetadata,
  sessionHistoryEntryIsExternalProviderObserved,
  transcriptExternalProviderObservedTurnMarker,
  transcriptExternalProviderObservedTurnMetadata,
  type ExternalProviderObservedMutableKernelFields,
} from "./external-provider-observation.js"

test("external provider observed predicate requires kernel observed source", () => {
  assert.equal(sessionHistoryEntryIsExternalProviderObserved({
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  }), true)
  assert.equal(sessionHistoryEntryIsExternalProviderObserved({
    source: ` ${EXTERNAL_PROVIDER_OBSERVED_SOURCE.toUpperCase()} `,
  }), true)
  assert.equal(sessionHistoryEntryIsExternalProviderObserved({
    source: null,
  }), false)
})

test("external provider observed metadata projects kernel history fields", () => {
  assert.deepEqual(historyEntryExternalProviderObservedMetadata({
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "turn-1",
    observed_at_ms: 123,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
    observedAtMs: 123,
    externalObservation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  })
})

test("external provider observed metadata normalizes nullable fields", () => {
  assert.deepEqual(historyEntryExternalProviderObservedMetadata({
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_provider: " codex ",
    external_provider_session_id: " thread-1 ",
    external_provider_turn_id: "",
    observed_at_ms: Number.NaN,
    external_observation: {
      settles_active_prompt: "true" as unknown as boolean,
      passive_telemetry: true,
    },
  }), {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: null,
    observedAtMs: null,
    externalObservation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  })
})

test("external provider observed metadata treats settlement as non-passive", () => {
  assert.deepEqual(historyEntryExternalProviderObservedMetadata({
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: true,
    },
  })?.externalObservation, {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
})

test("external provider observed metadata projects prompt-origin turn fields", () => {
  assert.deepEqual(promptOriginExternalProviderObservedMetadata({
    prompt_origin: " External ",
    external_provider: " codex ",
    external_provider_session_id: " thread-1 ",
    external_provider_turn_id: " turn-1 ",
  }), {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  })
  assert.deepEqual(promptOriginExternalProviderObservedMetadata({
    external_provider: " codex ",
    external_provider_session_id: " thread-1 ",
  }), {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: null,
  })
  assert.equal(promptOriginExternalProviderObservedMetadata({
    prompt_origin: "arroba",
    external_provider: "codex",
    external_provider_session_id: "thread-1",
  }), null)
})

test("external provider observed metadata projects transcript turn fields", () => {
  assert.deepEqual(transcriptExternalProviderObservedTurnMetadata({
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: " codex ",
    externalProviderSessionId: " thread-1 ",
    externalProviderTurnId: " turn-1 ",
  }), {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  })
  assert.deepEqual(transcriptExternalProviderObservedTurnMetadata({
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "",
    externalProviderSessionId: null,
    externalProviderTurnId: undefined,
  }), {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: null,
    externalProviderSessionId: null,
    externalProviderTurnId: null,
  })
  assert.equal(transcriptExternalProviderObservedTurnMetadata({
    source: "provider_output",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  }), null)
})

test("external provider observed history refresh signal requires observed provider status", () => {
  assert.equal(externalProviderObservedHistoryRefreshSignal({
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  }, ` ${EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS}\n`), true)
  assert.equal(externalProviderObservedHistoryRefreshSignal({
    kind: "provider_status",
    source: null,
  }, EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS), false)
  assert.equal(externalProviderObservedHistoryRefreshSignal({
    kind: "provider_output",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  }, EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS), false)
  assert.equal(externalProviderObservedHistoryRefreshSignal({
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  }, "OpenCode status: reconnecting"), false)
})

test("external provider observed status render policy requires observed non-passive status", () => {
  assert.equal(externalProviderObservedProviderStatusShouldRender({
    kind: "provider_status",
    text: "codex event turn_aborted {\"reason\":\"user\"}",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), true)
  assert.equal(externalProviderObservedProviderStatusShouldRender({
    kind: "provider_status",
    text: "codex event turn_aborted {\"reason\":\"user\"}",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: true,
    },
  }), true)
  assert.equal(externalProviderObservedProviderStatusShouldRender({
    kind: "provider_status",
    text: "codex token_count {\"total\":42}",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }), false)
  assert.equal(externalProviderObservedProviderStatusShouldRender({
    kind: "provider_status",
    text: "codex event turn_aborted {\"reason\":\"user\"}",
    source: null,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), false)
  assert.equal(externalProviderObservedProviderStatusShouldRender({
    kind: "provider_output",
    text: "codex event turn_aborted {\"reason\":\"user\"}",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), false)
})

test("external provider observed passive telemetry helper accepts history and transcript fields", () => {
  assert.equal(externalProviderObservedEntryIsPassiveTelemetry({
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }), true)
  assert.equal(externalProviderObservedEntryIsPassiveTelemetry({
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalObservation: {
      settles_active_prompt: true,
      passive_telemetry: true,
    },
  }), false)
  assert.equal(externalProviderObservedEntryIsPassiveTelemetry({
    source: "provider_output",
    externalObservation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }), false)
})

test("external provider observed settlement helper accepts provider status records and entries", () => {
  assert.equal(externalProviderObservedStatusSettlesActivePrompt({
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), true)
  assert.equal(externalProviderObservedStatusSettlesActivePrompt({
    role: "status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalObservation: {
      settles_active_prompt: true,
      passive_telemetry: true,
    },
  }), true)
  assert.equal(externalProviderObservedStatusSettlesActivePrompt({
    role: "assistant",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalObservation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), false)
  assert.equal(externalProviderObservedStatusSettlesActivePrompt({
    kind: "provider_status",
    source: "provider_output",
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  }), false)
})

test("external provider observed completion time prefers observation time then creation time", () => {
  assert.equal(externalProviderObservedCompletionAtMs({
    observedAtMs: 2_000,
    observed_at_ms: 1_500,
    createdAtMs: 1_000,
  }, () => 3_000), 2_000)
  assert.equal(externalProviderObservedCompletionAtMs({
    observed_at_ms: 1_500,
    createdAtMs: 1_000,
  }, () => 3_000), 1_500)
  assert.equal(externalProviderObservedCompletionAtMs({
    observedAtMs: Number.NaN,
    createdAtMs: 1_000,
  }, () => 3_000), 1_000)
  assert.equal(externalProviderObservedCompletionAtMs({}, () => 3_000), 3_000)
})

test("external provider observed import scoping keeps ordinary and unknown observed entries", () => {
  const externalImport = {
    external_provider: "codex",
    external_provider_session_id: "codex:thread-1",
    external_provider_session_provider_id: "thread-1",
  }
  assert.equal(externalProviderObservedEntryBelongsToImport(externalImport, {
    source: "provider_output",
    externalProvider: "opencode",
    externalProviderSessionId: "thread-2",
  }), true)
  assert.equal(externalProviderObservedEntryBelongsToImport(externalImport, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  }), true)
  assert.equal(externalProviderObservedEntryBelongsToImport(null, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  }), true)
})

test("external provider observed import scoping matches external and provider session ids", () => {
  const externalImport = {
    external_provider: "codex",
    external_provider_session_id: "codex:thread-1",
    external_provider_session_provider_id: "thread-1",
  }
  assert.equal(externalProviderObservedEntryBelongsToImport(externalImport, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "codex:thread-1",
  }), true)
  assert.equal(externalProviderObservedEntryBelongsToImport(externalImport, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  }), true)
  assert.equal(externalProviderObservedEntryBelongsToImport(externalImport, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
  }), true)
})

test("external provider observed import scoping rejects mismatched imported agents", () => {
  const externalImport = {
    external_provider: "codex",
    external_provider_session_id: "codex:thread-1",
    external_provider_session_provider_id: "thread-1",
  }
  assert.equal(externalProviderObservedEntryBelongsToImport(externalImport, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "opencode",
    externalProviderSessionId: "thread-1",
  }), false)
  assert.equal(externalProviderObservedEntryBelongsToImport(externalImport, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-2",
  }), false)
})

test("external provider observation merge preserves settlement over passive telemetry", () => {
  assert.deepEqual(mergeExternalProviderObservation({
    settles_active_prompt: false,
    passive_telemetry: true,
  }, {
    settles_active_prompt: true,
    passive_telemetry: false,
  }), {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
  assert.deepEqual(mergeExternalProviderObservation({
    settles_active_prompt: true,
    passive_telemetry: false,
  }, {
    settles_active_prompt: false,
    passive_telemetry: true,
  }), {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
  assert.deepEqual(mergeExternalProviderObservation(null, {
    settles_active_prompt: false,
    passive_telemetry: true,
  }), {
    settles_active_prompt: false,
    passive_telemetry: true,
  })
  assert.deepEqual(mergeExternalProviderObservation(null, {
    settles_active_prompt: "true",
    passive_telemetry: "yes",
  } as never), {
    settles_active_prompt: false,
    passive_telemetry: false,
  })
  assert.deepEqual(mergeExternalProviderObservation({
    settles_active_prompt: true,
    passive_telemetry: true,
  } as never, null), {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
  assert.deepEqual(mergeExternalProviderObservation({
    settles_active_prompt: "true",
    passive_telemetry: "yes",
  } as never, {
    settles_active_prompt: false,
    passive_telemetry: false,
  }), {
    settles_active_prompt: false,
    passive_telemetry: false,
  })
})

test("external provider observed source merge preserves observed source", () => {
  assert.equal(
    mergeExternalProviderObservedSource("provider_output", EXTERNAL_PROVIDER_OBSERVED_SOURCE),
    EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  )
  assert.equal(
    mergeExternalProviderObservedSource("provider_output", ` ${EXTERNAL_PROVIDER_OBSERVED_SOURCE.toUpperCase()} `),
    EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  )
  assert.equal(
    mergeExternalProviderObservedSource(EXTERNAL_PROVIDER_OBSERVED_SOURCE, "provider_output"),
    EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  )
  assert.equal(
    mergeExternalProviderObservedSource(` ${EXTERNAL_PROVIDER_OBSERVED_SOURCE.toUpperCase()} `, "provider_output"),
    EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  )
  assert.equal(mergeExternalProviderObservedSource(null, "provider_output"), "provider_output")
})

test("external provider observed history field merge preserves first stable metadata", () => {
  const target: ExternalProviderObservedMutableKernelFields & {
    kind: string
    text: string
  } = {
    kind: "provider_output",
    text: "native ",
  }
  mergeExternalProviderObservedHistoryFields(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "item-1",
    observed_at_ms: 1_000,
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  })
  mergeExternalProviderObservedHistoryFields(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_provider: "codex",
    external_provider_session_id: "thread-2",
    external_provider_turn_id: "item-2",
    observed_at_ms: 2_000,
  })

  assert.deepEqual(target, {
    kind: "provider_output",
    text: "native ",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "item-1",
    observed_at_ms: 1_000,
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  })
})

test("external provider observed history field merge canonicalizes observed source", () => {
  const target: ExternalProviderObservedMutableKernelFields = {}
  mergeExternalProviderObservedHistoryFields(target, {
    source: ` ${EXTERNAL_PROVIDER_OBSERVED_SOURCE.toUpperCase()} `,
  })
  mergeExternalProviderObservedHistoryFields(target, {
    source: "provider_output",
  })

  assert.equal(target.source, EXTERNAL_PROVIDER_OBSERVED_SOURCE)
})

test("external provider observed history field merge lets settlement override passive telemetry", () => {
  const target = {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }
  mergeExternalProviderObservedHistoryFields(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_observation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  })

  assert.deepEqual(target.external_observation, {
    settles_active_prompt: true,
    passive_telemetry: false,
  })
})

test("external provider observed transcript field merge preserves identity and settlement metadata", () => {
  const target: {
    externalProvider?: string | null
    externalProviderSessionId?: string | null
    externalProviderTurnId?: string | null
    observedAtMs?: number | null
    externalObservation?: { settles_active_prompt: boolean; passive_telemetry: boolean }
  } = {}

  mergeExternalProviderObservedTranscriptFields(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
    observedAtMs: 100,
    externalObservation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalObservation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  })

  assert.deepEqual(target, {
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
    observedAtMs: 100,
    externalObservation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  })
})

test("external provider observed turn metadata applies only missing transcript identity fields", () => {
  const target = {
    source: "provider_output",
    externalProvider: "codex",
    externalProviderSessionId: undefined as string | null | undefined,
    externalProviderTurnId: undefined as string | null | undefined,
  }

  applyExternalProviderObservedTurnMetadata(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "opencode",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  })

  assert.deepEqual(target, {
    source: "provider_output",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  })
})

test("external provider observed turn metadata ignores null provider identity fields", () => {
  const target: {
    source?: string | null
    externalProvider?: string | null
    externalProviderSessionId?: string | null
    externalProviderTurnId?: string | null
  } = {}

  applyExternalProviderObservedTurnMetadata(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: null,
    externalProviderSessionId: null,
    externalProviderTurnId: null,
  })

  assert.deepEqual(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  })
})

test("external provider observed turn marker resolves footer identity", () => {
  assert.deepEqual(transcriptExternalProviderObservedTurnMarker([{
    source: "provider_output",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  }, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: " opencode ",
    externalProviderSessionId: " thread-2 ",
  }]), {
    provider: "opencode",
    providerSessionId: "thread-2",
  })
  assert.deepEqual(transcriptExternalProviderObservedTurnMarker([{
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "",
    externalProviderSessionId: null,
  }]), {
    provider: "provider",
    providerSessionId: "unknown",
  })
  assert.equal(transcriptExternalProviderObservedTurnMarker([{
    source: "provider_output",
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
  }]), null)
})

test("external provider observed metadata ignores ordinary history entries", () => {
  assert.equal(historyEntryExternalProviderObservedMetadata({
    source: null,
  }), null)
})
