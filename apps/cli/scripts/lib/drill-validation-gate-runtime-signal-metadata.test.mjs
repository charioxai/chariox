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
      platformBundle: {
        validationSuite: {
          coverageAreas: [{ id: "matrix-validation", testCount: 4 }],
        },
        runtimeSignals: [
          { id: "client-projection-health", owner: "ui-client" },
        ],
        failureTaxonomy: {
          drill: ["ui-client-projection"],
          scenario: ["kernel-authority"],
        },
      },
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
    classifications: "cloud-validation-suite,kernel-authority,matrix-coverage,provider-auth,relay-runtime,slice-runtime,ui-client-projection",
    coverageAreas: "distributed-observability,matrix-validation",
    owners: "kernel-authority,provider-account,runtime-network,ui-client,validation-harness,validation-platform",
    runtimeSignalOwners: "kernel-authority,provider-runtime,runtime-network,ui-client",
    runtimeSignals: "client-projection-health,provider-run-lifecycle,relay-target-freshness,session-authority",
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
      failureOwners: { "runtime-network": 1 },
      failureClassifications: { "relay-runtime": 1 },
      matrixRuntimeSignals: { "provider-run-lifecycle": 2 },
      matrixRuntimeSignalOwners: { "provider-runtime": 1 },
      matrixOwners: { "provider-account": 1 },
      matrixClassifications: { "provider-auth": 1 },
      requiredFailureClassifications: { "kernel-authority": 1 },
      missingFailureClassifications: { "remote-extension-sync": 1 },
      requiredMatrixClassifications: { "workspace-live-sync-conflict": 2 },
      missingMatrixClassifications: { "provider-auth": 1 },
      generatedEvidenceKinds: { "matrix-report": 1, "validation-suite-run": 1 },
      generatedMatrixLimitations: { "dry-run-classification-coverage": 1 },
      requiredGeneratedEvidenceKinds: { "matrix-report": 1, "validation-suite-run": 1 },
      missingGeneratedEvidenceKinds: { "matrix-report": 1 },
      requiredGeneratedMatrixLimitations: { "dry-run-classification-coverage": 1 },
      missingGeneratedMatrixLimitations: { "dry-run-classification-coverage": 1 },
    },
    matrixRuntimeSignalSources: {
      "provider-run-lifecycle": [],
    },
    requiredFailureClassifications: ["kernel-authority"],
    missingFailureClassifications: ["remote-extension-sync"],
    requiredMatrixClassifications: ["workspace-live-sync-conflict"],
    missingMatrixClassifications: ["provider-auth"],
    requiredGeneratedEvidenceKinds: ["matrix-report", "validation-suite-run"],
    missingGeneratedEvidenceKinds: ["matrix-report"],
    requiredGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
    missingGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
    nextActions: [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports",
    }],
  })

  assert.deepEqual(metadata, {
    classifications: "cloud-validation-suite,kernel-authority,matrix-coverage,provider-auth,relay-runtime,remote-extension-sync,workspace-live-sync-conflict",
    coverageAreas: "distributed-observability",
    generatedEvidenceKinds: "matrix-report,validation-suite-run",
    generatedMatrixLimitations: "dry-run-classification-coverage",
    missingGeneratedEvidenceKinds: "matrix-report",
    missingGeneratedMatrixLimitations: "dry-run-classification-coverage",
    owners: "kernel-authority,provider-account,provider-runtime,runtime-network,validation-harness,validation-platform",
    requiredGeneratedEvidenceKinds: "matrix-report,validation-suite-run",
    requiredGeneratedMatrixLimitations: "dry-run-classification-coverage",
    runtimeSignalOwners: "kernel-authority,provider-runtime,runtime-network",
    runtimeSignals: "provider-run-lifecycle,relay-target-freshness,session-authority",
  })
})

test("omits runtime signal metadata when no signal evidence is present", () => {
  assert.deepEqual(runtimeSignalMetadataForValidationGateReport({ checks: {} }), {})
  assert.deepEqual(runtimeSignalMetadataForValidationGateAggregate({ coverage: {} }), {})
})
