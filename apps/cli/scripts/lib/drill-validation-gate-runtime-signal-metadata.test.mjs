import assert from "node:assert/strict"
import test from "node:test"

import {
  diagnosticMetadataForValidationGateAggregate,
  diagnosticMetadataForValidationGateReport,
  runtimeSignalMetadataForValidationGateAggregate,
  runtimeSignalMetadataForValidationGateReport,
} from "./drill-validation-gate-runtime-signal-metadata.mjs"

test("builds runtime signal metadata for validation gate reports", () => {
  const metadata = runtimeSignalMetadataForValidationGateReport({
    checks: {
      artifacts: {
        aggregate: {
          classifications: { "cloud-validation-suite": 1 },
          coverageAreas: { "distributed-observability": 1 },
          owners: { "validation-platform": 1 },
          runtimeSignals: { "session-authority": 1 },
          runtimeSignalOwners: { "kernel-authority": 1 },
        },
      },
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
    runtimeSignalOwners: "kernel-authority,provider-runtime,runtime-network,runtime-state",
    runtimeSignals: "provider-run-lifecycle,relay-target-freshness,session-authority,workspace-live-sync-state",
  })
})

test("builds owner and classification metadata for validation gate reports", () => {
  const metadata = diagnosticMetadataForValidationGateReport({
    checks: {
      artifacts: {
        aggregate: {
          classifications: { "cloud-validation-suite": 1 },
          coverageAreas: { "distributed-observability": 1 },
          owners: { "validation-platform": 1 },
          runtimeSignals: { "session-authority": 1 },
          runtimeSignalOwners: { "kernel-authority": 1 },
        },
      },
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
    classifications: "cloud-validation-suite,matrix-coverage,provider-auth,relay-runtime,slice-runtime",
    coverageAreas: "distributed-observability",
    owners: "provider-account,runtime-network,validation-harness,validation-platform",
    runtimeSignalOwners: "kernel-authority,provider-runtime,runtime-network",
    runtimeSignals: "provider-run-lifecycle,relay-target-freshness,session-authority",
  })
})

test("builds runtime signal metadata for validation gate aggregates", () => {
  const metadata = runtimeSignalMetadataForValidationGateAggregate({
    coverage: {
      artifactRuntimeSignals: { "session-authority": 1 },
      artifactRuntimeSignalOwners: { "kernel-authority": 1 },
      artifactCoverageAreas: { "distributed-observability": 1 },
      artifactOwners: { "validation-platform": 1 },
      artifactClassifications: { "cloud-validation-suite": 1 },
      failureRuntimeSignals: { "relay-target-freshness": 1 },
      failureRuntimeSignalOwners: { "runtime-network": 1 },
      matrixRuntimeSignals: { "workspace-live-sync-state": 1 },
      matrixRuntimeSignalOwners: { "runtime-state": 1 },
    },
    matrixRuntimeSignalSources: {
      "provider-run-lifecycle": [],
    },
  })

  assert.deepEqual(metadata, {
    runtimeSignalOwners: "kernel-authority,provider-runtime,runtime-network,runtime-state",
    runtimeSignals: "provider-run-lifecycle,relay-target-freshness,session-authority,workspace-live-sync-state",
  })
})

test("builds owner and classification metadata for validation gate aggregates", () => {
  const metadata = diagnosticMetadataForValidationGateAggregate({
    coverage: {
      artifactRuntimeSignals: { "session-authority": 1 },
      artifactRuntimeSignalOwners: { "kernel-authority": 1 },
      artifactCoverageAreas: { "distributed-observability": 1 },
      artifactOwners: { "validation-platform": 1 },
      artifactClassifications: { "cloud-validation-suite": 1 },
      failureRuntimeSignals: { "relay-target-freshness": 1 },
      failureRuntimeSignalOwners: { "runtime-network": 1 },
      matrixRuntimeSignals: { "provider-run-lifecycle": 2 },
      matrixRuntimeSignalOwners: { "provider-runtime": 1 },
      requiredFailureClassifications: { "kernel-authority": 1 },
      missingFailureClassifications: { "remote-extension-sync": 1 },
      requiredMatrixClassifications: { "workspace-live-sync-conflict": 2 },
      missingMatrixClassifications: { "provider-auth": 1 },
    },
    matrixRuntimeSignalSources: {
      "provider-run-lifecycle": [],
    },
    requiredFailureClassifications: ["kernel-authority"],
    missingFailureClassifications: ["remote-extension-sync"],
    requiredMatrixClassifications: ["workspace-live-sync-conflict"],
    missingMatrixClassifications: ["provider-auth"],
    nextActions: [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports",
    }],
  })

  assert.deepEqual(metadata, {
    classifications: "cloud-validation-suite,kernel-authority,matrix-coverage,provider-auth,remote-extension-sync,workspace-live-sync-conflict",
    coverageAreas: "distributed-observability",
    owners: "kernel-authority,provider-runtime,runtime-network,validation-harness,validation-platform",
    runtimeSignalOwners: "kernel-authority,provider-runtime,runtime-network",
    runtimeSignals: "provider-run-lifecycle,relay-target-freshness,session-authority",
  })
})

test("omits runtime signal metadata when no signal evidence is present", () => {
  assert.deepEqual(runtimeSignalMetadataForValidationGateReport({ checks: {} }), {})
  assert.deepEqual(runtimeSignalMetadataForValidationGateAggregate({ coverage: {} }), {})
})
