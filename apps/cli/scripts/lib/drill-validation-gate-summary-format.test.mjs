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
        aggregate: {
          schema: "arroba.drill.artifact_index.aggregate.v1",
          totals: { indexes: 1, artifacts: 3, sizeBytes: 42 },
          runtimeSignals: {
            "session-authority": 2,
            "workspace-live-sync-state": 1,
          },
        },
      },
      matrices: matrixCheck({
        status: "passed",
        reportPaths: ["/tmp/matrix.json"],
        requiredMatrixRuntimeSignals: ["session-authority", "workspace-live-sync-state"],
        missingMatrixRuntimeSignals: [],
        aggregate: {
          schema: "arroba.drill.matrix.aggregate.v1",
          status: "passed",
          totals: { failed: 0, skipped: 1, dryRun: 2 },
          runtimeSignalScenarios: {
            "session-authority": [{
              matrix: "workspace-live-sync-matrix",
              source: "/tmp/matrix.json",
              id: "permission",
              status: "passed",
            }],
            "workspace-live-sync-state": [{
              matrix: "workspace-live-sync-matrix",
              source: "/tmp/matrix.json",
              id: "managed",
              status: "passed",
            }],
          },
        },
      }),
      failures: {
        status: "failed",
        roots: [],
        inputs: ["/tmp/failure.json"],
        manifestPaths: ["/tmp/failure.json"],
        aggregate: {
          schema: "arroba.drill.failure.aggregate.v1",
          total: 1,
          runtimeSignals: {
            "lease-health": 1,
            "relay-target-freshness": 1,
          },
        },
      },
    },
    status: "failed",
  }))

  assert.match(text, /platform_validation_suite_tests=2 coverage=matrix-validation:2/)
  assert.match(text, /platform_failure_taxonomy=drill:1 scenario:2/)
  assert.match(text, /artifact_total=3 size_bytes=42/)
  assert.match(text, /artifact_runtime_signals=session-authority:2,workspace-live-sync-state:1/)
  assert.match(text, /matrix_status=passed failed=0 skipped=1 dry_run=2/)
  assert.match(text, /matrix_runtime_signal_sources:/)
  assert.match(text, /- session-authority: workspace-live-sync-matrix\/permission\(passed\) source=\/tmp\/matrix\.json/)
  assert.match(text, /- workspace-live-sync-state: workspace-live-sync-matrix\/managed\(passed\) source=\/tmp\/matrix\.json/)
  assert.match(text, /failure_total=1/)
  assert.match(text, /failure_runtime_signals=lease-health:1,relay-target-freshness:1/)
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
