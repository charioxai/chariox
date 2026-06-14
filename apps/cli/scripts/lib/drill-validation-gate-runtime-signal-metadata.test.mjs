import assert from "node:assert/strict"
import test from "node:test"

import {
  runtimeSignalMetadataForValidationGateAggregate,
  runtimeSignalMetadataForValidationGateReport,
} from "./drill-validation-gate-runtime-signal-metadata.mjs"

test("builds runtime signal metadata for validation gate reports", () => {
  const metadata = runtimeSignalMetadataForValidationGateReport({
    checks: {
      artifacts: { aggregate: { runtimeSignals: { "session-authority": 1 } } },
      failures: { aggregate: { runtimeSignals: { "relay-target-freshness": 1 } } },
      matrices: {
        aggregate: {
          runtimeSignals: { "provider-run-lifecycle": 2 },
          runtimeSignalScenarios: { "workspace-live-sync-state": [] },
        },
      },
    },
  })

  assert.deepEqual(metadata, {
    runtimeSignals: "provider-run-lifecycle,relay-target-freshness,session-authority,workspace-live-sync-state",
  })
})

test("builds runtime signal metadata for validation gate aggregates", () => {
  const metadata = runtimeSignalMetadataForValidationGateAggregate({
    coverage: {
      artifactRuntimeSignals: { "session-authority": 1 },
      failureRuntimeSignals: { "relay-target-freshness": 1 },
    },
    matrixRuntimeSignalSources: {
      "provider-run-lifecycle": [],
      "workspace-live-sync-state": [],
    },
  })

  assert.deepEqual(metadata, {
    runtimeSignals: "provider-run-lifecycle,relay-target-freshness,session-authority,workspace-live-sync-state",
  })
})

test("omits runtime signal metadata when no signal evidence is present", () => {
  assert.deepEqual(runtimeSignalMetadataForValidationGateReport({ checks: {} }), {})
  assert.deepEqual(runtimeSignalMetadataForValidationGateAggregate({ coverage: {} }), {})
})
