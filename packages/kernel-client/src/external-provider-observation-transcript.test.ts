import assert from "node:assert/strict"
import test from "node:test"

import {
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
  applyExternalProviderObservedTranscriptMetadata,
  applyExternalProviderObservedTurnMetadata,
  historyEntryExternalProviderObservedMetadata,
  transcriptExternalProviderObservedTurnMarker,
} from "./external-provider-observation.js"

test("external provider observed transcript metadata application shares live and history policy", () => {
  const target: {
    source?: string | null
    externalProvider?: string | null
    externalProviderSessionId?: string | null
    externalProviderTurnId?: string | null
    observedAtMs?: number | null
    externalObservation?: { settles_active_prompt: boolean; passive_telemetry: boolean }
  } = {
    source: "provider_output",
    externalObservation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
  }

  applyExternalProviderObservedTranscriptMetadata(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
    observedAtMs: 1_000,
    externalObservation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  })

  assert.deepEqual(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
    observedAtMs: 1_000,
    externalObservation: {
      settles_active_prompt: true,
      passive_telemetry: false,
    },
  })
})

test("external provider observed transcript metadata application preserves richer identity", () => {
  const target: {
    source?: string | null
    externalProvider?: string | null
    externalProviderSessionId?: string | null
    externalProviderTurnId?: string | null
    observedAtMs?: number | null
  } = {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
    observedAtMs: 1_000,
  }

  applyExternalProviderObservedTranscriptMetadata(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: null,
    externalProviderSessionId: null,
    externalProviderTurnId: null,
    observedAtMs: null,
  })

  assert.deepEqual(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
    observedAtMs: 1_000,
  })
})

test("external provider observed turn metadata keeps external source authoritative", () => {
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
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
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

test("external provider observed turn metadata fills null provider identity fields", () => {
  const target: {
    source?: string | null
    externalProvider?: string | null
    externalProviderSessionId?: string | null
    externalProviderTurnId?: string | null
  } = {
    externalProvider: null,
    externalProviderSessionId: null,
    externalProviderTurnId: null,
  }

  applyExternalProviderObservedTurnMetadata(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
  })

  assert.deepEqual(target, {
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "codex",
    externalProviderSessionId: "thread-1",
    externalProviderTurnId: "turn-1",
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
  assert.equal(transcriptExternalProviderObservedTurnMarker([{
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    externalProvider: "",
    externalProviderSessionId: null,
  }]), null)
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
