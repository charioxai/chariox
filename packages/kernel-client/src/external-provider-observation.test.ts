import assert from "node:assert/strict"
import test from "node:test"

import {
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  historyEntryExternalProviderObservedMetadata,
} from "./external-provider-observation.js"

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

test("external provider observed metadata ignores ordinary history entries", () => {
  assert.equal(historyEntryExternalProviderObservedMetadata({
    source: null,
  }), null)
})
