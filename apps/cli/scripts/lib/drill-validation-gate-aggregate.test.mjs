import assert from "node:assert/strict"
import test from "node:test"

import {
  drillValidationGateAggregateExitCode,
  formatDrillValidationGateAggregateSummary,
  summarizeValidationGateReportAggregate,
  validateDrillValidationGateAggregate,
} from "./drill-validation-gate-aggregate.mjs"

test("summarizes validation gate reports with aggregate requirements", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    sources: ["workspace-live-sync.json"],
    normalizedRequiredPresets: ["workspace-live-sync"],
    normalizedAggregateRequirements: {
      requiredPlatformCoverageAreas: ["runtime-fixtures"],
      requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
      requiredFailureClassifications: ["kernel-authority"],
      requiredMatrices: ["workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["workspace-live-sync-conflict"],
      requiredMatrixRuntimeSignals: ["workspace-live-sync-state"],
      requiredDeploymentPresets: ["local"],
      requiredProviders: ["codex"],
      requiredScenarios: ["managed"],
    },
    validateReport: () => {},
  })

  assert.equal(aggregate.status, "passed")
  assert.equal(drillValidationGateAggregateExitCode(aggregate), 0)
  assert.deepEqual(aggregate.totals, { reports: 1, passed: 1, failed: 0 })
  assert.deepEqual(aggregate.coverage.presets, { "workspace-live-sync": 1 })
  assert.deepEqual(aggregate.coverage.artifactRuntimeSignals, {
    "session-authority": 2,
    "workspace-live-sync-state": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactRuntimeSignalOwners, {
    "kernel-authority": 1,
    "runtime-state": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactSchemas, {
    "arroba.drill.validation_suite_run.v1": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactOwners, {
    "validation-platform": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactClassifications, {
    "cloud-validation-suite": 1,
  })
  assert.deepEqual(aggregate.coverage.matrixRuntimeSignals, {
    "workspace-live-sync-state": 1,
  })
  assert.deepEqual(aggregate.coverage.matrixRuntimeSignalOwners, {
    "runtime-state": 1,
  })
  assert.deepEqual(aggregate.matrixRuntimeSignalSources, {
    "workspace-live-sync-state": [{
      reportSource: "workspace-live-sync.json",
      matrix: "workspace-live-sync-matrix",
      source: "/tmp/workspace-live-sync-matrix.json",
      id: "managed",
      status: "passed",
    }],
  })
  assert.deepEqual(aggregate.missingPresets, [])
  assert.deepEqual(aggregate.missingProviders, [])
  assert.deepEqual(aggregate.missingArtifactSchemas, [])
  assert.deepEqual(aggregate.reports[0].source, "workspace-live-sync.json")
  assert.deepEqual(aggregate.reports[0].artifactCoverage.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
  assert.deepEqual(aggregate.reports[0].artifactCoverage.runtimeSignals, {
    "session-authority": 2,
    "workspace-live-sync-state": 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.runtimeSignalOwners, {
    "kernel-authority": 1,
    "runtime-state": 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.owners, {
    "validation-platform": 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.classifications, {
    "cloud-validation-suite": 1,
  })
  assert.deepEqual(aggregate.reports[0].matrixCoverage.runtimeSignals, {
    "workspace-live-sync-state": 1,
  })
  assert.deepEqual(aggregate.reports[0].matrixCoverage.runtimeSignalOwners, {
    "runtime-state": 1,
  })
  assert.doesNotThrow(() => validateDrillValidationGateAggregate(aggregate))
  const text = formatDrillValidationGateAggregateSummary(aggregate)
  assert.match(text, /required_providers=codex missing=none/)
  assert.match(text, /required_artifact_schemas=arroba\.drill\.validation_suite_run\.v1 missing=none/)
  assert.match(text, /- artifact_schemas: arroba.drill.validation_suite_run.v1=1/)
  assert.match(text, /- artifact_runtime_signals: session-authority=2 workspace-live-sync-state=1/)
  assert.match(text, /- artifact_runtime_signal_owners: kernel-authority=1 runtime-state=1/)
  assert.match(text, /- artifact_owners: validation-platform=1/)
  assert.match(text, /- artifact_classifications: cloud-validation-suite=1/)
  assert.match(text, /- matrix_runtime_signals: workspace-live-sync-state=1/)
  assert.match(text, /- matrix_runtime_signal_owners: runtime-state=1/)
  assert.match(text, /matrix_runtime_signal_sources:/)
  assert.match(text, /- workspace-live-sync-state: workspace-live-sync-matrix\/managed\(passed\) source=\/tmp\/workspace-live-sync-matrix\.json report=workspace-live-sync\.json/)
})

test("fails aggregate requirements missing from otherwise passing reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    normalizedRequiredPresets: ["remote-home-extension"],
    normalizedAggregateRequirements: {
      requiredPlatformCoverageAreas: ["hosted-cloud-drills"],
      requiredArtifactSchemas: ["arroba.drill.matrix.v1"],
      requiredFailureClassifications: ["remote-extension-sync"],
      requiredMatrices: ["remote-home-extension-matrix"],
      requiredMatrixClassifications: ["remote-extension-sync"],
      requiredMatrixRuntimeSignals: ["home-extension-manifest-sync"],
      requiredDeploymentPresets: ["hosted-cloud"],
      requiredProviders: ["claude"],
      requiredScenarios: ["hetzner-collab"],
    },
    validateReport: () => {},
  })

  assert.equal(aggregate.status, "failed")
  assert.equal(drillValidationGateAggregateExitCode(aggregate), 1)
  assert.deepEqual(aggregate.missingPresets, ["remote-home-extension"])
  assert.deepEqual(aggregate.missingArtifactSchemas, ["arroba.drill.matrix.v1"])
  assert.deepEqual(aggregate.missingMatrixRuntimeSignals, ["home-extension-manifest-sync"])
  assert.deepEqual(aggregate.missingProviders, ["claude"])
  assert.deepEqual(aggregate.missingScenarios, ["hetzner-collab"])
  assert.deepEqual(
    aggregate.nextActions.map(({ classification, nextAction }) => ({ classification, nextAction })),
    [
      {
        classification: "artifact-coverage",
        nextAction: "provide validation gate reports with artifact schemas: arroba.drill.matrix.v1",
      },
      {
        classification: "matrix-coverage",
        nextAction: "provide validation gate reports requiring deployment presets: hosted-cloud",
      },
      {
        classification: "matrix-coverage",
        nextAction: "provide validation gate reports requiring matrices: remote-home-extension-matrix",
      },
      {
        classification: "matrix-coverage",
        nextAction: "provide validation gate reports requiring matrix classifications: remote-extension-sync",
      },
      {
        classification: "matrix-coverage",
        nextAction: "provide validation gate reports requiring matrix runtime signals: home-extension-manifest-sync",
      },
      {
        classification: "matrix-coverage",
        nextAction: "provide validation gate reports requiring providers: claude",
      },
      {
        classification: "matrix-coverage",
        nextAction: "provide validation gate reports requiring scenarios: hetzner-collab",
      },
      {
        classification: "platform-bundle",
        nextAction: "provide validation gate reports requiring failure classifications: remote-extension-sync",
      },
      {
        classification: "platform-bundle",
        nextAction: "provide validation gate reports requiring platform coverage areas: hosted-cloud-drills",
      },
      {
        classification: "validation-gate",
        nextAction: "provide validation gate reports for presets: remote-home-extension",
      },
    ],
  )
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /required_presets=remote-home-extension missing=remote-home-extension/)
})

test("reports executable validation suite remediation for missing suite-run aggregate evidence", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    normalizedAggregateRequirements: {
      requiredArtifactSchemas: [
        "arroba.drill.validation_suite_run.v1",
        "arroba.drill.matrix.v1",
      ],
    },
    validateReport: () => {},
  })

  assert.equal(aggregate.status, "failed")
  assert.deepEqual(aggregate.missingArtifactSchemas, [
    "arroba.drill.matrix.v1",
  ])
  assert.deepEqual(
    aggregate.nextActions
      .filter((action) => action.classification === "artifact-coverage")
      .map(({ nextAction }) => nextAction),
    [
      "provide validation gate reports with artifact schemas: arroba.drill.matrix.v1",
    ],
  )

  const missingSuiteRun = summarizeValidationGateReportAggregate([reportFixture({
    checks: {
      ...reportFixture().checks,
      artifacts: {
        ...reportFixture().checks.artifacts,
        aggregate: {
          schemas: {},
          runtimeSignals: {},
          runtimeSignalOwners: {},
        },
      },
    },
  })], {
    normalizedAggregateRequirements: {
      requiredArtifactSchemas: [
        "arroba.drill.validation_suite_run.v1",
        "arroba.drill.matrix.v1",
      ],
    },
    validateReport: () => {},
  })

  assert.deepEqual(missingSuiteRun.missingArtifactSchemas, [
    "arroba.drill.validation_suite_run.v1",
    "arroba.drill.matrix.v1",
  ])
  assert.deepEqual(
    missingSuiteRun.nextActions
      .filter((action) => action.classification === "artifact-coverage")
      .map(({ nextAction }) => nextAction),
    [
      "provide validation gate reports with artifact schemas: arroba.drill.matrix.v1",
      "run an executable validation suite with --run-json --output PATH --output-artifact-index PATH, then rerun the validation gate aggregate",
    ],
  )
})

test("aggregates failure runtime signal coverage from failed reports", () => {
  const failedReport = reportFixture()
  failedReport.status = "failed"
  failedReport.checks.failures = {
    status: "failed",
    aggregate: {
      runtimeSignals: {
        "lease-health": 1,
        "provider-run-lifecycle": 2,
      },
    },
  }
  const aggregate = summarizeValidationGateReportAggregate([failedReport], {
    validateReport: () => {},
  })

  assert.equal(aggregate.status, "failed")
  assert.deepEqual(aggregate.coverage.failureRuntimeSignals, {
    "lease-health": 1,
    "provider-run-lifecycle": 2,
  })
  assert.deepEqual(aggregate.coverage.failureRuntimeSignalOwners, {
    "kernel-authority": 1,
    "provider-runtime": 2,
  })
  assert.deepEqual(aggregate.reports[0].failureCoverage.runtimeSignals, {
    "lease-health": 1,
    "provider-run-lifecycle": 2,
  })
  assert.deepEqual(aggregate.reports[0].failureCoverage.runtimeSignalOwners, {
    "kernel-authority": 1,
    "provider-runtime": 2,
  })
  assert.match(
    formatDrillValidationGateAggregateSummary(aggregate),
    /- failure_runtime_signals: lease-health=1 provider-run-lifecycle=2/,
  )
  assert.match(
    formatDrillValidationGateAggregateSummary(aggregate),
    /- failure_runtime_signal_owners: kernel-authority=1 provider-runtime=2/,
  )
})

test("rejects inconsistent aggregate status and coverage", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    normalizedRequiredPresets: ["workspace-live-sync"],
    validateReport: () => {},
  })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      status: "failed",
    }),
    /status does not match totals and requirements/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        presets: {},
      },
    }),
    /missingPresets does not match reports/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      matrixRuntimeSignalSources: {
        "workspace-live-sync-state": [{
          reportSource: "other.json",
          matrix: "workspace-live-sync-matrix",
          source: "/tmp/workspace-live-sync-matrix.json",
          id: "managed",
          status: "passed",
        }],
      },
    }),
    /matrixRuntimeSignalSources does not match reports/,
  )
})

function reportFixture(overrides = {}) {
  return {
    schema: "arroba.drill.validation_gate.v1",
    status: "passed",
    presets: ["workspace-live-sync"],
    checks: {
      configuration: { status: "passed" },
      platformBundle: {
        status: "passed",
        requiredCoverageAreas: ["runtime-fixtures"],
        missingCoverageAreas: [],
        requiredFailureClassifications: ["kernel-authority"],
        missingFailureClassifications: [],
      },
      artifacts: {
        status: "passed",
        requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
        missingArtifactSchemas: [],
        aggregate: {
          schemas: {
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
          owners: {
            "validation-platform": 1,
          },
          classifications: {
            "cloud-validation-suite": 1,
          },
        },
      },
      matrices: {
        status: "passed",
        requiredMatrices: ["workspace-live-sync-matrix"],
        missingMatrices: [],
        requiredMatrixClassifications: ["workspace-live-sync-conflict"],
        missingMatrixClassifications: [],
        requiredMatrixRuntimeSignals: ["workspace-live-sync-state"],
        missingMatrixRuntimeSignals: [],
        aggregate: {
          runtimeSignals: {
            "workspace-live-sync-state": 1,
          },
          runtimeSignalScenarios: {
            "workspace-live-sync-state": [{
              matrix: "workspace-live-sync-matrix",
              source: "/tmp/workspace-live-sync-matrix.json",
              id: "managed",
              status: "passed",
            }],
          },
        },
        requiredDeploymentPresets: ["local"],
        missingDeploymentPresets: [],
        requiredProviders: ["codex"],
        missingProviders: [],
        requiredScenarios: ["managed"],
        missingScenarios: [],
      },
      failures: { status: "skipped" },
    },
    nextActions: [],
    ...overrides,
  }
}
