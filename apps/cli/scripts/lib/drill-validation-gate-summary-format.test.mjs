import assert from "node:assert/strict"
import test from "node:test"

import { formatDrillValidationGateSummary } from "./drill-validation-gate-summary-format.mjs"
import { DRILL_VALIDATION_GATE_SCHEMA } from "./drill-validation-gate-report.mjs"

test("formats a minimal passing validation gate summary", () => {
  const text = formatDrillValidationGateSummary(report())

  assert.match(text, /drill validation gate:/)
  assert.match(text, /status=passed/)
  assert.match(text, /configuration=passed/)
  assert.match(text, /platform_bundle=skipped/)
  assert.match(text, /artifacts=skipped roots=0 inputs=0 indexes=0/)
  assert.match(text, /matrices=skipped roots=0 inputs=0 reports=0 require_complete=false/)
  assert.match(text, /failures=skipped roots=0 inputs=0 manifests=0/)
  assert.match(text, /next: validation artifacts passed configured gates/)
})

test("formats required platform and matrix coverage diagnostics", () => {
  const text = formatDrillValidationGateSummary(report({
    status: "failed",
    checks: {
      platformBundle: {
        status: "failed",
        dir: null,
        requiredCoverageAreas: ["matrix-validation"],
        missingCoverageAreas: ["matrix-validation"],
        requiredFailureClassifications: ["kernel-authority"],
        missingFailureClassifications: ["kernel-authority"],
        error: "no platform bundle provided",
      },
      matrices: matrixCheck({
        status: "failed",
        requireComplete: true,
        requiredMatrices: ["workspace-live-sync-matrix"],
        missingMatrices: ["workspace-live-sync-matrix"],
        requiredMatrixClassifications: ["workspace-live-sync-conflict"],
        missingMatrixClassifications: ["workspace-live-sync-conflict"],
        requiredMatrixRuntimeSignals: ["workspace-live-sync-state"],
        missingMatrixRuntimeSignals: ["workspace-live-sync-state"],
        requiredDeploymentPresets: ["hosted-cloud"],
        missingDeploymentPresets: ["hosted-cloud"],
        requiredProviders: ["claude"],
        missingProviders: ["claude"],
        requiredScenarios: ["tracked"],
        missingScenarios: ["tracked"],
        error: "no matrix reports found",
      }),
    },
    nextActions: [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run missing drill matrices: workspace-live-sync-matrix",
      count: 1,
    }],
  }))

  assert.match(text, /platform_required_coverage_areas=matrix-validation missing=matrix-validation/)
  assert.match(text, /platform_required_failure_classifications=kernel-authority missing=kernel-authority/)
  assert.match(text, /matrix_required_names=workspace-live-sync-matrix missing=workspace-live-sync-matrix/)
  assert.match(text, /matrix_required_classifications=workspace-live-sync-conflict missing=workspace-live-sync-conflict/)
  assert.match(text, /matrix_required_runtime_signals=workspace-live-sync-state missing=workspace-live-sync-state/)
  assert.match(text, /matrix_runtime_signal_sources:\n- workspace-live-sync-state: missing/)
  assert.match(text, /matrix_required_deployment_presets=hosted-cloud missing=hosted-cloud/)
  assert.match(text, /matrix_required_providers=claude missing=claude/)
  assert.match(text, /matrix_required_scenarios=tracked missing=tracked/)
  assert.match(text, /matrix_error=no matrix reports found/)
  assert.match(text, /next actions:/)
  assert.match(text, /owner=validation-harness classification=matrix-coverage count=1/)
  assert.match(text, /next: inspect failed gate checks and rerun the relevant drills/)
})

test("formats aggregate summaries for platform, artifact, matrix, and failure evidence", () => {
  const text = formatDrillValidationGateSummary(report({
    checks: {
      platformBundle: {
        status: "passed",
        dir: "/tmp/platform",
        requiredCoverageAreas: [],
        missingCoverageAreas: [],
        requiredFailureClassifications: [],
        missingFailureClassifications: [],
        artifacts: [{
          path: "validation-suite.json",
          schema: "arroba.drill.validation_suite.v1",
          sha256: "a".repeat(64),
          sizeBytes: 100,
        }],
        validationSuite: {
          testCount: 2,
          coverageAreas: [{ id: "matrix-validation", testCount: 2 }],
        },
        failureTaxonomy: {
          drill: ["kernel-authority"],
          scenario: ["kernel-authority", "workspace-live-sync-conflict"],
        },
      },
      artifacts: {
        status: "passed",
        roots: [],
        inputs: ["/tmp/artifacts.json"],
        indexPaths: ["/tmp/artifacts.json"],
        aggregate: artifactAggregateFixture(),
      },
      matrices: matrixCheck({
        status: "passed",
        reportPaths: ["/tmp/matrix.json"],
        requiredMatrixRuntimeSignals: ["session-authority", "workspace-live-sync-state"],
        missingMatrixRuntimeSignals: [],
        aggregate: matrixAggregateFixture(),
      }),
      failures: {
        status: "failed",
        roots: [],
        inputs: ["/tmp/failure.json"],
        manifestPaths: ["/tmp/failure.json"],
        aggregate: failureAggregateFixture(),
      },
    },
    status: "failed",
  }))

  assert.match(text, /platform_validation_suite_tests=2 coverage=matrix-validation:2/)
  assert.match(text, /platform_failure_taxonomy=drill:1 scenario:2/)
  assert.match(text, /artifact_total=3 size_bytes=42/)
  assert.match(text, /artifact_schemas=arroba\.drill\.matrix\.v1:2,arroba\.drill\.validation_suite_run\.v1:1/)
  assert.match(text, /artifact_runtime_signals=session-authority:2,workspace-live-sync-state:1/)
  assert.match(text, /artifact_runtime_signal_owners=kernel-authority:1,runtime-state:1/)
  assert.match(text, /artifact_owners=validation-platform:1/)
  assert.match(text, /artifact_classifications=cloud-validation-suite:1/)
  assert.match(text, /artifact_kinds=artifact-index:1,matrix-report:1,validation-suite-run:1/)
  assert.match(text, /artifact_generated_evidence_kinds=matrix-report:1,validation-suite-run:1/)
  assert.match(text, /artifact_generated_matrix_limitations=dry-run-classification-coverage:1/)
  assert.match(text, /artifact_required_generated_evidence_kinds=matrix-report:2,validation-suite-run:1/)
  assert.match(text, /artifact_missing_generated_evidence_kinds=matrix-report:1/)
  assert.match(text, /artifact_required_generated_matrix_limitations=dry-run-classification-coverage:1/)
  assert.match(text, /artifact_missing_generated_matrix_limitations=dry-run-classification-coverage:1/)
  assert.match(text, /artifact_evidence_repos=cloud:1,oss:1/)
  assert.match(text, /artifact_coverage_input_sources=artifact metadata inputs:1/)
  assert.match(text, /matrix_status=passed failed=0 skipped=1 dry_run=2/)
  assert.match(text, /matrix_runtime_signals=session-authority:1,workspace-live-sync-state:1/)
  assert.match(text, /matrix_runtime_signal_owners=kernel-authority:1,runtime-state:1/)
  assert.match(text, /matrix_exit_criteria=dry-run:1,satisfied:1/)
  assert.match(text, /matrix_incomplete_exit_criteria:/)
  assert.match(text, /workspace-live-sync-matrix\/managed\/managed:exit-02\(dry-run\) reason=scenario command was selected but not executed source=\/tmp\/matrix\.json: remote worker acknowledged projection/)
  assert.match(text, /matrix_runtime_signal_sources:/)
  assert.match(text, /- session-authority: workspace-live-sync-matrix\/permission\(dry-run\) source=\/tmp\/matrix\.json/)
  assert.match(text, /- workspace-live-sync-state: workspace-live-sync-matrix\/managed\(dry-run\) source=\/tmp\/matrix\.json/)
  assert.match(text, /failure_total=1/)
  assert.match(text, /failure_runtime_signals=lease-health:1,relay-target-freshness:1/)
  assert.match(text, /failure_runtime_signal_owners=kernel-authority:1,runtime-network:1/)
})

test("formats generated evidence provenance when present", () => {
  const text = formatDrillValidationGateSummary(report({
    generatedEvidence: {
      validationSuites: {
        enabled: true,
        outputRoots: ["/tmp/suites/cloud", "/tmp/suites/oss"],
        artifactIndexes: [
          "/tmp/suites/cloud/arroba-drill-artifacts.json",
          "/tmp/suites/oss/arroba-drill-artifacts.json",
        ],
        commands: [
          {
            artifactIndexPath: "/tmp/suites/oss/arroba-drill-artifacts.json",
            args: ["--run-json", "--preserve-failure-root", "/tmp/suites/oss/failed-run"],
            cwd: "/repo/arroba",
            failureRoot: "/tmp/suites/oss/failed-run",
            reportPath: "/tmp/suites/oss/drill-validation-suite-run.json",
            scriptPath: "/repo/arroba/apps/cli/scripts/drill-validation-suite.mjs",
          },
          {
            artifactIndexPath: "/tmp/suites/cloud/arroba-drill-artifacts.json",
            args: ["--run-json", "--preserve-failure-root", "/tmp/suites/cloud/failed-run"],
            cwd: "/repo/arroba-cloud",
            failureRoot: "/tmp/suites/cloud/failed-run",
            reportPath: "/tmp/suites/cloud/cloud-validation-suite-run.json",
            scriptPath: "/repo/arroba-cloud/scripts/cloud-validation-suite.mjs",
          },
        ],
      },
      matrixReports: {
        enabled: true,
        roots: ["/tmp/matrices/cloud", "/tmp/matrices/oss"],
        commands: [
          {
            args: ["--include-hetzner"],
            artifactIndexPath: "/tmp/matrices/oss/a-artifacts.json",
            cwd: "/repo/arroba",
            reportPath: "/tmp/matrices/oss/a.json",
            scriptPath: "/repo/arroba/a.mjs",
          },
          {
            args: ["--include-hosted-cloud"],
            artifactIndexPath: "/tmp/matrices/cloud/b-artifacts.json",
            cwd: "/repo/arroba-cloud",
            reportPath: "/tmp/matrices/cloud/b.json",
            scriptPath: "/repo/arroba-cloud/b.mjs",
          },
        ],
        dryRun: false,
        continueOnFailure: true,
        limitations: [{
          kind: "dry-run-classification-coverage",
          owner: "validation-harness",
          nextAction: "rerun distributed runtime matrix reports without --matrix-dry-run before release",
        }],
      },
    },
  }))

  assert.match(text, /generated_validation_suites=enabled output_roots=\/tmp\/suites\/cloud,\/tmp\/suites\/oss artifact_indexes=\/tmp\/suites\/cloud\/arroba-drill-artifacts\.json,\/tmp\/suites\/oss\/arroba-drill-artifacts\.json commands=2 failure_roots=\/tmp\/suites\/oss\/failed-run,\/tmp\/suites\/cloud\/failed-run/)
  assert.match(text, /generated_matrices=enabled roots=\/tmp\/matrices\/cloud,\/tmp\/matrices\/oss commands=2 dry_run=false continue_on_failure=true/)
  assert.match(text, /generated_matrix_limitations:/)
  assert.match(text, /- kind=dry-run-classification-coverage owner=validation-harness: rerun distributed runtime matrix reports without --matrix-dry-run before release/)
})

test("validates reports before formatting", () => {
  assert.throws(
    () => formatDrillValidationGateSummary({ schema: "wrong" }),
    /unsupported schema/,
  )
})

function report(overrides = {}) {
  const checks = {
    configuration: { status: "passed" },
    platformBundle: {
      status: "skipped",
      dir: null,
      requiredCoverageAreas: [],
      missingCoverageAreas: [],
      requiredFailureClassifications: [],
      missingFailureClassifications: [],
    },
    artifacts: { status: "skipped", roots: [], inputs: [], indexPaths: [] },
    matrices: matrixCheck(),
    failures: { status: "skipped", roots: [], inputs: [], manifestPaths: [] },
    ...(overrides.checks ?? {}),
  }
  return {
    schema: DRILL_VALIDATION_GATE_SCHEMA,
    status: Object.values(checks).some((check) => check.status === "failed") ? "failed" : "passed",
    presets: [],
    checks,
    nextActions: [],
    ...overrides,
    checks,
  }
}

function matrixCheck(overrides = {}) {
  return {
    status: "skipped",
    roots: [],
    inputs: [],
    reportPaths: [],
    requireComplete: false,
    requiredMatrices: [],
    missingMatrices: [],
    requiredMatrixClassifications: [],
    missingMatrixClassifications: [],
    requiredMatrixRuntimeSignals: [],
    missingMatrixRuntimeSignals: [],
    requiredDeploymentPresets: [],
    missingDeploymentPresets: [],
    requiredProviders: [],
    missingProviders: [],
    requiredScenarios: [],
    missingScenarios: [],
    ...overrides,
  }
}

function artifactAggregateFixture() {
  return {
    schema: "arroba.drill.artifact_index.aggregate.v1",
    totals: { indexes: 1, artifacts: 3, sizeBytes: 42 },
    schemas: {
      "arroba.drill.matrix.v1": 2,
      "arroba.drill.validation_suite_run.v1": 1,
    },
    runtimeSignals: {
      "session-authority": 2,
      "workspace-live-sync-state": 1,
    },
    runtimeSignalOwners: {
      "kernel-authority": 1,
      "runtime-state": 1,
    },
    coverageAreas: {
      "distributed-observability": 1,
    },
    owners: {
      "validation-platform": 1,
    },
    classifications: {
      "cloud-validation-suite": 1,
    },
    exitCriterionStatuses: {},
    incompleteExitCriterionStatuses: {},
    artifactKinds: {
      "artifact-index": 1,
      "matrix-report": 1,
      "validation-suite-run": 1,
    },
    generatedEvidenceKinds: {
      "matrix-report": 1,
      "validation-suite-run": 1,
    },
    generatedMatrixLimitations: {
      "dry-run-classification-coverage": 1,
    },
    requiredGeneratedEvidenceKinds: {
      "matrix-report": 2,
      "validation-suite-run": 1,
    },
    missingGeneratedEvidenceKinds: {
      "matrix-report": 1,
    },
    requiredGeneratedMatrixLimitations: {
      "dry-run-classification-coverage": 1,
    },
    missingGeneratedMatrixLimitations: {
      "dry-run-classification-coverage": 1,
    },
    evidenceRepos: {
      cloud: 1,
      oss: 1,
    },
    artifactCoverageInputSources: {
      "artifact metadata inputs": 1,
    },
    indexes: [{
      source: "/tmp/artifacts.json",
      rootDir: "/tmp/artifacts",
      artifacts: 3,
      sizeBytes: 42,
      schemas: {
        "arroba.drill.validation_suite_run.v1": 1,
        "arroba.drill.matrix.v1": 2,
      },
      runtimeSignals: {
        "session-authority": 2,
        "workspace-live-sync-state": 1,
      },
      runtimeSignalOwners: {
        "kernel-authority": 1,
        "runtime-state": 1,
      },
      coverageAreas: {
        "distributed-observability": 1,
      },
      owners: {
        "validation-platform": 1,
      },
      classifications: {
        "cloud-validation-suite": 1,
      },
      exitCriterionStatuses: {},
      incompleteExitCriterionStatuses: {},
      artifactKinds: {
        "artifact-index": 1,
        "matrix-report": 1,
        "validation-suite-run": 1,
      },
      generatedEvidenceKinds: {
        "matrix-report": 1,
        "validation-suite-run": 1,
      },
      generatedMatrixLimitations: {
        "dry-run-classification-coverage": 1,
      },
      requiredGeneratedEvidenceKinds: {
        "matrix-report": 2,
        "validation-suite-run": 1,
      },
      missingGeneratedEvidenceKinds: {
        "matrix-report": 1,
      },
      requiredGeneratedMatrixLimitations: {
        "dry-run-classification-coverage": 1,
      },
      missingGeneratedMatrixLimitations: {
        "dry-run-classification-coverage": 1,
      },
      evidenceRepos: {
        cloud: 1,
        oss: 1,
      },
      artifactCoverageInputSources: {
        "artifact metadata inputs": 1,
      },
    }],
  }
}

function matrixAggregateFixture() {
  return {
    schema: "arroba.drill.matrix.aggregate.v1",
    status: "passed",
    totals: { reports: 1, scenarios: 3, passed: 0, failed: 0, skipped: 1, dryRun: 2, durationMs: 30 },
    failedScenarios: [],
    skippedScenarios: [],
    incompleteScenarios: [
      matrixScenarioFixture("managed", "dry-run"),
      matrixScenarioFixture("permission", "dry-run"),
      matrixScenarioFixture("restart", "skipped"),
    ],
    owners: {},
    classifications: {},
    matrixNames: { "workspace-live-sync-matrix": 1 },
    deploymentPresets: {},
    providers: {},
    scenarioIds: { managed: 1, permission: 1, restart: 1 },
    exitCriteria: { "dry-run": 1, satisfied: 1 },
    incompleteExitCriteria: [{
      matrix: "workspace-live-sync-matrix",
      source: "/tmp/matrix.json",
      scenarioId: "managed",
      id: "managed:exit-02",
      criterion: "remote worker acknowledged projection",
      status: "dry-run",
      reason: "scenario command was selected but not executed",
    }],
    runtimeSignals: {
      "session-authority": 1,
      "workspace-live-sync-state": 1,
    },
    runtimeSignalOwners: {
      "kernel-authority": 1,
      "runtime-state": 1,
    },
    runtimeSignalScenarios: {
      "session-authority": [matrixScenarioFixture("permission", "dry-run")],
      "workspace-live-sync-state": [matrixScenarioFixture("managed", "dry-run")],
    },
    nextActions: [],
    reports: [{
      matrix: "workspace-live-sync-matrix",
      source: "/tmp/matrix.json",
      status: "passed",
      classifications: {},
      deploymentPresets: [],
      providers: [],
      scenarioIds: ["managed", "permission", "restart"],
      exitCriteria: { "dry-run": 1, satisfied: 1 },
      runtimeSignals: {
        "session-authority": 1,
        "workspace-live-sync-state": 1,
      },
      runtimeSignalScenarios: {
        "session-authority": [matrixScenarioFixture("permission", "dry-run")],
        "workspace-live-sync-state": [matrixScenarioFixture("managed", "dry-run")],
      },
      scenarioCount: 3,
      counts: { passed: 0, failed: 0, skipped: 1, dryRun: 2 },
      durationMs: 30,
    }],
  }
}

function matrixScenarioFixture(id, status) {
  return {
    matrix: "workspace-live-sync-matrix",
    source: "/tmp/matrix.json",
    id,
    status,
  }
}

function failureAggregateFixture() {
  const nextAction = "inspect relay and kernel logs in the preserved artifact root, then rerun the drill"
  return {
    schema: "arroba.drill.failure.aggregate.v1",
    total: 1,
    owners: { "runtime-network": 1 },
    classifications: { "relay-runtime": 1 },
    runtimeSignals: {
      "lease-health": 1,
      "relay-target-freshness": 1,
    },
    runtimeSignalOwners: {
      "kernel-authority": 1,
      "runtime-network": 1,
    },
    nextActions: [{
      owner: "runtime-network",
      classification: "relay-runtime",
      nextAction,
      count: 1,
    }],
    failures: [{
      drill: "relay-drill",
      source: "/tmp/failure.json",
      rootDir: "/tmp/failure",
      owner: "runtime-network",
      classification: "relay-runtime",
      runtimeSignals: ["lease-health", "relay-target-freshness"],
      nextAction,
    }],
  }
}
