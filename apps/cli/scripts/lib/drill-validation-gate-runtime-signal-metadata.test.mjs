import assert from "node:assert/strict"
import test from "node:test"

import {
  diagnosticMetadataForValidationGateReport,
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

test("builds owner and classification metadata for validation gate reports", () => {
  const metadata = diagnosticMetadataForValidationGateReport({
    checks: {
      artifacts: { aggregate: { runtimeSignals: { "session-authority": 1 } } },
      failures: {
        aggregate: {
          owners: { "runtime-network": 1 },
          classifications: { "relay-runtime": 1 },
          runtimeSignals: { "relay-target-freshness": 1 },
        },
      },
      matrices: {
        aggregate: {
          owners: { "provider-account": 1 },
          classifications: { "provider-auth": 1, "slice-runtime": 2 },
          runtimeSignals: { "provider-run-lifecycle": 2 },
        },
      },
    },
    nextActions: [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports",
    }],
  })

  assert.deepEqual(metadata, {
    classifications: "matrix-coverage,provider-auth,relay-runtime,slice-runtime",
    owners: "provider-account,runtime-network,validation-harness",
    runtimeSignals: "provider-run-lifecycle,relay-target-freshness,session-authority",
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
