import assert from "node:assert/strict"
import test from "node:test"

import {
  EXTERNAL_PROVIDER_HISTORY_UPDATED_STATUS,
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  externalProviderObservedHistoryRefreshSignal,
  externalProviderObservedProviderStatusShouldRender,
  historyEntryExternalProviderObservedMetadata,
  mergeExternalProviderObservation,
  promptOriginExternalProviderObservedMetadata,
  sessionHistoryEntryIsExternalProviderObserved,
  transcriptExternalProviderObservedTurnMetadata,
} from "./external-provider-observation.js"

test("external provider observed predicate requires kernel observed source", () => {
  assert.equal(sessionHistoryEntryIsExternalProviderObserved({
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
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
})

test("external provider observed metadata ignores ordinary history entries", () => {
  assert.equal(historyEntryExternalProviderObservedMetadata({
    source: null,
  }), null)
})
