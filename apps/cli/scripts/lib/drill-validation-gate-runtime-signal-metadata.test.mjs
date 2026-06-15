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
          exitCriterionStatuses: { "dry-run": 1 },
          incompleteExitCriterionStatuses: { "dry-run": 1 },
          providerAccountAliases: { "codex=work": 1 },
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
          exitCriterionStatuses: { "dry-run": 1 },
          incompleteExitCriterionStatuses: { "dry-run": 1 },
          providerAccountAliases: { "codex=work": 1 },
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
    generatedEvidence: {
      kinds: ["matrix-report"],
      validationSuites: {
        enabled: true,
        artifactIndexes: ["/tmp/generated-suite/arroba-drill-artifacts.json"],
        failureRoots: ["/tmp/generated-suite/failed-run"],
        outputRoots: ["/tmp/generated-suite"],
      },
      matrixReports: {
        enabled: true,
        artifactIndexes: ["/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"],
        roots: ["/tmp/generated-matrix"],
        dryRun: true,
        continueOnFailure: true,
        limitations: [{
          kind: "dry-run-classification-coverage",
          owner: "validation-harness",
          nextAction: "rerun generated matrix reports without --matrix-dry-run",
        }],
        commands: [{
          artifactIndexPath: "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
          args: ["--matrix-dry-run"],
          cwd: "/tmp/arroba",
          reportPath: "/tmp/generated-matrix/workspace-live-sync-matrix.json",
          scriptPath: "/tmp/arroba/apps/cli/scripts/live-workspace-live-sync-matrix-drill.mjs",
        }],
      },
    },
  })

  assert.deepEqual(metadata, {
    classifications: "cloud-validation-suite,kernel-authority,matrix-coverage,provider-auth,relay-runtime,slice-runtime,ui-client-projection",
    coverageAreas: "distributed-observability,matrix-validation",
    exitCriterionStatuses: "dry-run",
    generatedEvidenceKinds: "matrix-report,validation-suite-run",
    generatedMatrixArtifactIndexes: "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
    generatedMatrixLimitations: "dry-run-classification-coverage",
    generatedValidationSuiteFailureRoots: "/tmp/generated-suite/failed-run",
    incompleteExitCriterionStatuses: "dry-run",
    owners: "kernel-authority,provider-account,runtime-network,ui-client,validation-harness,validation-platform",
    providerAccountAliases: "codex=work",
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
      artifactExitCriterionStatuses: { "dry-run": 1 },
      artifactIncompleteExitCriterionStatuses: { "dry-run": 1 },
      artifactProviderAccountAliases: { "codex=work": 1 },
      requiredArtifactProviderAccountAliases: { "codex=work": 1 },
      missingArtifactProviderAccountAliases: { "opencode=zen": 1 },
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
      artifactExitCriterionStatuses: { "dry-run": 1 },
      artifactIncompleteExitCriterionStatuses: { "dry-run": 1 },
      artifactProviderAccountAliases: { "codex=work": 1 },
      requiredArtifactProviderAccountAliases: { "codex=work": 1 },
      missingArtifactProviderAccountAliases: { "opencode=zen": 1 },
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
      artifactGeneratedMatrixArtifactIndexes: { "/tmp/artifact-input/generated-matrix-artifacts.json": 1 },
      generatedMatrixLimitations: { "dry-run-classification-coverage": 1 },
      generatedValidationSuiteFailureRoots: { "/tmp/generated-suite/coverage-failed-run": 1 },
      artifactGeneratedValidationSuiteFailureRoots: { "/tmp/generated-suite/artifact-input-failed-run": 1 },
      requiredGeneratedEvidenceKinds: { "matrix-report": 1, "validation-suite-run": 1 },
      missingGeneratedEvidenceKinds: { "matrix-report": 1 },
      requiredGeneratedMatrixLimitations: { "dry-run-classification-coverage": 1 },
      missingGeneratedMatrixLimitations: { "dry-run-classification-coverage": 1 },
      requiredGeneratedValidationSuiteFailureRoots: { "/tmp/generated-suite/failed-run": 1 },
      missingGeneratedValidationSuiteFailureRoots: { "/tmp/generated-suite/missing-run": 1 },
      artifactCoverageInputSources: { "artifact metadata inputs": 1 },
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
    requiredGeneratedValidationSuiteFailureRoots: ["/tmp/generated-suite/failed-run"],
    missingGeneratedValidationSuiteFailureRoots: ["/tmp/generated-suite/missing-run"],
    requiredArtifactProviderAccountAliases: ["codex=work"],
    missingArtifactProviderAccountAliases: ["opencode=zen"],
    artifactCoverageInputs: [
      { source: "z-artifacts.json" },
      { source: "a-artifacts.json" },
    ],
    reports: [{
      generatedEvidence: {
        validationSuites: {
          enabled: true,
          commands: [{
            failureRoot: "/tmp/generated-suite/failed-run",
          }],
        },
        matrixReports: {
          artifactIndexes: ["/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"],
        },
      },
    }],
    nextActions: [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports",
    }],
  })

  assert.deepEqual(metadata, {
    artifactCoverageInputCount: "2",
    artifactCoverageInputSources: "a-artifacts.json,artifact metadata inputs,z-artifacts.json",
    classifications: "cloud-validation-suite,kernel-authority,matrix-coverage,provider-auth,relay-runtime,remote-extension-sync,workspace-live-sync-conflict",
    coverageAreas: "distributed-observability",
    exitCriterionStatuses: "dry-run",
    generatedEvidenceKinds: "matrix-report,validation-suite-run",
    generatedMatrixArtifactIndexes: "/tmp/artifact-input/generated-matrix-artifacts.json,/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
    generatedMatrixLimitations: "dry-run-classification-coverage",
    generatedValidationSuiteFailureRoots: "/tmp/generated-suite/artifact-input-failed-run,/tmp/generated-suite/coverage-failed-run,/tmp/generated-suite/failed-run",
    incompleteExitCriterionStatuses: "dry-run",
    missingGeneratedEvidenceKinds: "matrix-report",
    missingGeneratedMatrixLimitations: "dry-run-classification-coverage",
    missingGeneratedValidationSuiteFailureRoots: "/tmp/generated-suite/missing-run",
    missingProviderAccountAliases: "opencode=zen",
    owners: "kernel-authority,provider-account,provider-runtime,runtime-network,validation-harness,validation-platform",
    providerAccountAliases: "codex=work",
    requiredProviderAccountAliases: "codex=work",
    requiredGeneratedEvidenceKinds: "matrix-report,validation-suite-run",
    requiredGeneratedMatrixLimitations: "dry-run-classification-coverage",
    requiredGeneratedValidationSuiteFailureRoots: "/tmp/generated-suite/failed-run",
    runtimeSignalOwners: "kernel-authority,provider-runtime,runtime-network",
    runtimeSignals: "provider-run-lifecycle,relay-target-freshness,session-authority",
  })
})

test("omits runtime signal metadata when no signal evidence is present", () => {
  assert.deepEqual(runtimeSignalMetadataForValidationGateReport({ checks: {} }), {})
  assert.deepEqual(runtimeSignalMetadataForValidationGateAggregate({ coverage: {} }), {})
})
