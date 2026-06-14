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
      requiredArtifactKinds: ["validation-suite-run"],
      requiredArtifactEvidenceRepos: ["oss"],
      requiredFailureClassifications: ["kernel-authority"],
      requiredMatrices: ["workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["workspace-live-sync-conflict"],
      requiredMatrixRuntimeSignals: ["workspace-live-sync-state"],
      requiredDeploymentPresets: ["local"],
      requiredProviders: ["codex"],
      requiredScenarios: ["managed"],
      requiredGeneratedEvidenceKinds: [],
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
  assert.deepEqual(aggregate.coverage.artifactKinds, {
    "validation-suite-run": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactEvidenceRepos, {
    oss: 1,
  })
  assert.deepEqual(aggregate.coverage.matrixRuntimeSignals, {
    "workspace-live-sync-state": 1,
  })
  assert.deepEqual(aggregate.coverage.matrixRuntimeSignalOwners, {
    "runtime-state": 1,
  })
  assert.deepEqual(aggregate.coverage.matrixOwners, {
    "runtime-state": 1,
  })
  assert.deepEqual(aggregate.coverage.matrixClassifications, {
    "workspace-live-sync-conflict": 1,
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
  assert.deepEqual(aggregate.missingArtifactKinds, [])
  assert.deepEqual(aggregate.missingArtifactEvidenceRepos, [])
  assert.deepEqual(aggregate.reports[0].source, "workspace-live-sync.json")
  assert.deepEqual(aggregate.reports[0].artifactCoverage.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
  assert.deepEqual(aggregate.reports[0].artifactCoverage.requiredArtifactKinds, ["validation-suite-run"])
  assert.deepEqual(aggregate.reports[0].artifactCoverage.requiredArtifactEvidenceRepos, ["oss"])
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
  assert.deepEqual(aggregate.reports[0].artifactCoverage.artifactKinds, {
    "validation-suite-run": 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.evidenceRepos, {
    oss: 1,
  })
  assert.deepEqual(aggregate.reports[0].matrixCoverage.runtimeSignals, {
    "workspace-live-sync-state": 1,
  })
  assert.deepEqual(aggregate.reports[0].matrixCoverage.runtimeSignalOwners, {
    "runtime-state": 1,
  })
  assert.deepEqual(aggregate.reports[0].matrixCoverage.owners, {
    "runtime-state": 1,
  })
  assert.deepEqual(aggregate.reports[0].matrixCoverage.classifications, {
    "workspace-live-sync-conflict": 1,
  })
  assert.doesNotThrow(() => validateDrillValidationGateAggregate(aggregate))
  const text = formatDrillValidationGateAggregateSummary(aggregate)
  assert.match(text, /required_providers=codex missing=none/)
  assert.match(text, /required_artifact_schemas=arroba\.drill\.validation_suite_run\.v1 missing=none/)
  assert.match(text, /required_artifact_kinds=validation-suite-run missing=none/)
  assert.match(text, /required_artifact_evidence_repos=oss missing=none/)
  assert.match(text, /- artifact_schemas: arroba.drill.validation_suite_run.v1=1/)
  assert.match(text, /- artifact_runtime_signals: session-authority=2 workspace-live-sync-state=1/)
  assert.match(text, /- artifact_runtime_signal_owners: kernel-authority=1 runtime-state=1/)
  assert.match(text, /- artifact_owners: validation-platform=1/)
  assert.match(text, /- artifact_classifications: cloud-validation-suite=1/)
  assert.match(text, /- artifact_kinds: validation-suite-run=1/)
  assert.match(text, /- artifact_evidence_repos: oss=1/)
  assert.match(text, /- matrix_runtime_signals: workspace-live-sync-state=1/)
  assert.match(text, /- matrix_runtime_signal_owners: runtime-state=1/)
  assert.match(text, /- matrix_owners: runtime-state=1/)
  assert.match(text, /- matrix_classifications: workspace-live-sync-conflict=1/)
  assert.match(text, /matrix_runtime_signal_sources:/)
  assert.match(text, /- workspace-live-sync-state: workspace-live-sync-matrix\/managed\(passed\) source=\/tmp\/workspace-live-sync-matrix\.json report=workspace-live-sync\.json/)
})

test("rejects unknown artifact evidence repo labels in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredArtifactEvidenceRepos: ["cluod"],
    }),
    /requiredArtifactEvidenceRepos\[0\] has unknown evidence repo "cluod"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        artifactEvidenceRepos: { cluod: 1 },
      },
    }),
    /coverage\.artifactEvidenceRepos has unknown evidence repo "cluod"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        artifactCoverage: {
          ...aggregate.reports[0].artifactCoverage,
          evidenceRepos: { cluod: 1 },
        },
      }],
    }),
    /reports\[0\]\.artifactCoverage\.evidenceRepos has unknown evidence repo "cluod"/,
  )
})

test("rejects unknown generated evidence kind labels in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture({
    generatedEvidence: {
      kinds: ["matrix-report"],
      validationSuites: {
        enabled: false,
        artifactIndexes: [],
        outputRoots: [],
      },
      matrixReports: {
        enabled: false,
        roots: [],
        commands: [],
        dryRun: false,
        continueOnFailure: false,
      },
    },
  })], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredGeneratedEvidenceKinds: ["matrix-reprot"],
    }),
    /requiredGeneratedEvidenceKinds\[0\] has unknown generated evidence kind "matrix-reprot"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        generatedEvidenceKinds: { "matrix-reprot": 1 },
      },
    }),
    /coverage\.generatedEvidenceKinds has unknown generated evidence kind "matrix-reprot"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        generatedEvidence: {
          ...aggregate.reports[0].generatedEvidence,
          kinds: ["matrix-reprot"],
        },
      }],
    }),
    /reports\[0\]\.generatedEvidence\.kinds\[0\] has unknown generated evidence kind "matrix-reprot"/,
  )
})

test("rejects unknown artifact kind labels in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredArtifactKinds: ["validation-sutie"],
    }),
    /requiredArtifactKinds\[0\] has unknown artifact kind "validation-sutie"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        artifactKinds: { "validation-sutie": 1 },
      },
    }),
    /coverage\.artifactKinds has unknown artifact kind "validation-sutie"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        artifactCoverage: {
          ...aggregate.reports[0].artifactCoverage,
          artifactKinds: { "validation-sutie": 1 },
        },
      }],
    }),
    /reports\[0\]\.artifactCoverage\.artifactKinds has unknown artifact kind "validation-sutie"/,
  )
})

test("rejects unknown provider labels in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredProviders: ["cdoex"],
    }),
    /requiredProviders\[0\] has unknown provider "cdoex"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        requiredProviders: { cdoex: 1 },
      },
    }),
    /coverage\.requiredProviders has unknown provider "cdoex"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        matrixCoverage: {
          ...aggregate.reports[0].matrixCoverage,
          requiredProviders: ["cdoex"],
        },
      }],
    }),
    /reports\[0\]\.matrixCoverage\.requiredProviders\[0\] has unknown provider "cdoex"/,
  )
})

test("rejects unknown deployment preset labels in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredDeploymentPresets: ["same-host-remtoe"],
    }),
    /requiredDeploymentPresets\[0\] has unknown deployment preset "same-host-remtoe"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        requiredDeploymentPresets: { "same-host-remtoe": 1 },
      },
    }),
    /coverage\.requiredDeploymentPresets has unknown deployment preset "same-host-remtoe"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        matrixCoverage: {
          ...aggregate.reports[0].matrixCoverage,
          requiredDeploymentPresets: ["same-host-remtoe"],
        },
      }],
    }),
    /reports\[0\]\.matrixCoverage\.requiredDeploymentPresets\[0\] has unknown deployment preset "same-host-remtoe"/,
  )
})

test("rejects unknown runtime signal owners and failure classifications in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredFailureClassifications: ["workspace-live-synch-conflict"],
    }),
    /requiredFailureClassifications\[0\] has unknown failure classification "workspace-live-synch-conflict"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        requiredFailureClassifications: { "workspace-live-synch-conflict": 1 },
      },
    }),
    /coverage\.requiredFailureClassifications has unknown failure classification "workspace-live-synch-conflict"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        platformCoverage: {
          ...aggregate.reports[0].platformCoverage,
          requiredFailureClassifications: ["workspace-live-synch-conflict"],
        },
      }],
    }),
    /reports\[0\]\.platformCoverage\.requiredFailureClassifications\[0\] has unknown failure classification "workspace-live-synch-conflict"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        artifactCoverage: {
          ...aggregate.reports[0].artifactCoverage,
          requiredArtifactRuntimeSignalOwners: ["kernel-authorit"],
        },
      }],
    }),
    /reports\[0\]\.artifactCoverage\.requiredArtifactRuntimeSignalOwners\[0\] has unknown runtime signal owner "kernel-authorit"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        matrixCoverage: {
          ...aggregate.reports[0].matrixCoverage,
          requiredMatrixClassifications: ["workspace-live-synch-conflict"],
        },
      }],
    }),
    /reports\[0\]\.matrixCoverage\.requiredMatrixClassifications\[0\] has unknown failure classification "workspace-live-synch-conflict"/,
  )
})

test("fails aggregate requirements missing from otherwise passing reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    normalizedRequiredPresets: ["remote-home-extension"],
    normalizedAggregateRequirements: {
      requiredPlatformCoverageAreas: ["hosted-cloud-drills"],
      requiredArtifactSchemas: ["arroba.drill.matrix.v1"],
      requiredArtifactKinds: ["matrix-report"],
      requiredArtifactEvidenceRepos: ["cloud"],
      requiredFailureClassifications: ["remote-extension-sync"],
      requiredMatrices: ["remote-home-extension-matrix"],
      requiredMatrixClassifications: ["remote-extension-sync"],
      requiredMatrixRuntimeSignals: ["home-extension-manifest-sync"],
      requiredDeploymentPresets: ["hosted-cloud"],
      requiredProviders: ["claude"],
      requiredScenarios: ["hetzner-collab"],
      requiredGeneratedEvidenceKinds: ["matrix-report"],
    },
    validateReport: () => {},
  })

  assert.equal(aggregate.status, "failed")
  assert.equal(drillValidationGateAggregateExitCode(aggregate), 1)
  assert.deepEqual(aggregate.missingPresets, ["remote-home-extension"])
  assert.deepEqual(aggregate.missingArtifactSchemas, ["arroba.drill.matrix.v1"])
  assert.deepEqual(aggregate.missingArtifactKinds, ["matrix-report"])
  assert.deepEqual(aggregate.missingArtifactEvidenceRepos, ["cloud"])
  assert.deepEqual(aggregate.missingMatrixRuntimeSignals, ["home-extension-manifest-sync"])
  assert.deepEqual(aggregate.missingProviders, ["claude"])
  assert.deepEqual(aggregate.missingScenarios, ["hetzner-collab"])
  assert.deepEqual(aggregate.missingGeneratedEvidenceKinds, ["matrix-report"])
  assert.deepEqual(
    aggregate.nextActions.map(({ classification, nextAction }) => ({ classification, nextAction })),
    [
      {
        classification: "artifact-coverage",
        nextAction: "provide validation gate reports with artifact evidence repos: cloud",
      },
      {
        classification: "artifact-coverage",
        nextAction: "provide validation gate reports with artifact kinds: matrix-report",
      },
      {
        classification: "artifact-coverage",
        nextAction: "provide validation gate reports with artifact schemas: arroba.drill.matrix.v1",
      },
      {
        classification: "generated-evidence",
        nextAction: "provide validation gate reports with generated evidence kinds: matrix-report",
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
      owners: {
        "kernel-authority": 1,
        "provider-runtime": 2,
      },
      classifications: {
        "kernel-authority": 1,
        "provider-error": 2,
      },
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
  assert.deepEqual(aggregate.coverage.failureOwners, {
    "kernel-authority": 1,
    "provider-runtime": 2,
  })
  assert.deepEqual(aggregate.coverage.failureClassifications, {
    "kernel-authority": 1,
    "provider-error": 2,
  })
  assert.deepEqual(aggregate.reports[0].failureCoverage.runtimeSignals, {
    "lease-health": 1,
    "provider-run-lifecycle": 2,
  })
  assert.deepEqual(aggregate.reports[0].failureCoverage.runtimeSignalOwners, {
    "kernel-authority": 1,
    "provider-runtime": 2,
  })
  assert.deepEqual(aggregate.reports[0].failureCoverage.owners, {
    "kernel-authority": 1,
    "provider-runtime": 2,
  })
  assert.deepEqual(aggregate.reports[0].failureCoverage.classifications, {
    "kernel-authority": 1,
    "provider-error": 2,
  })
  assert.match(
    formatDrillValidationGateAggregateSummary(aggregate),
    /- failure_runtime_signals: lease-health=1 provider-run-lifecycle=2/,
  )
  assert.match(
    formatDrillValidationGateAggregateSummary(aggregate),
    /- failure_runtime_signal_owners: kernel-authority=1 provider-runtime=2/,
  )
  assert.match(
    formatDrillValidationGateAggregateSummary(aggregate),
    /- failure_owners: kernel-authority=1 provider-runtime=2/,
  )
  assert.match(
    formatDrillValidationGateAggregateSummary(aggregate),
    /- failure_classifications: kernel-authority=1 provider-error=2/,
  )
})

test("aggregates generated evidence provenance from gate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture({
    generatedEvidence: generatedEvidenceFixture(),
  })], {
    sources: ["distributed-runtime-gate.json"],
    normalizedAggregateRequirements: {
      requiredGeneratedEvidenceKinds: ["matrix-report", "validation-suite-run"],
    },
    validateReport: () => {},
  })

  assert.deepEqual(aggregate.coverage.generatedEvidenceKinds, {
    "matrix-report": 1,
    "validation-suite-run": 1,
  })
  assert.deepEqual(aggregate.requiredGeneratedEvidenceKinds, ["matrix-report", "validation-suite-run"])
  assert.deepEqual(aggregate.missingGeneratedEvidenceKinds, [])
  assert.deepEqual(aggregate.coverage.requiredGeneratedEvidenceKinds, {
    "matrix-report": 1,
    "validation-suite-run": 1,
  })
  assert.deepEqual(aggregate.coverage.missingGeneratedEvidenceKinds, {})
  assert.deepEqual(aggregate.reports[0].generatedEvidence, {
    kinds: ["validation-suite-run", "matrix-report"],
    validationSuites: {
      enabled: true,
      artifactIndexes: [
        "/tmp/suites/cloud/arroba-drill-artifacts.json",
        "/tmp/suites/oss/arroba-drill-artifacts.json",
      ],
      outputRoots: ["/tmp/suites/cloud", "/tmp/suites/oss"],
    },
    matrixReports: {
      enabled: true,
      roots: ["/tmp/matrices/cloud", "/tmp/matrices/oss"],
      dryRun: false,
      continueOnFailure: true,
      commands: [{
        artifactIndexPath: "/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json",
        args: ["--include-hetzner"],
        cwd: "/repo/arroba",
        reportPath: "/tmp/matrices/oss/native-provider-tui-matrix.json",
        scriptPath: "/repo/arroba/apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs",
      }],
    },
  })
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /- generated_evidence_kinds: matrix-report=1 validation-suite-run=1/)
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /required_generated_evidence_kinds=matrix-report,validation-suite-run missing=none/)
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
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredMatrixRuntimeSignals: ["workspace-live-synch-state"],
    }),
    /requiredMatrixRuntimeSignals\[0\] has unknown runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        matrixRuntimeSignals: { "workspace-live-synch-state": 1 },
        matrixRuntimeSignalOwners: { "runtime-state": 1 },
      },
    }),
    /coverage\.matrixRuntimeSignals has unknown runtime signal "workspace-live-synch-state"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        failureRuntimeSignals: { "relay-target-freshness": 1 },
        failureRuntimeSignalOwners: { "kernel-authority": 1 },
      },
    }),
    /coverage\.failureRuntimeSignalOwners must match runtimeSignals/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        matrixCoverage: {
          ...aggregate.reports[0].matrixCoverage,
          runtimeSignalScenarios: {
            "workspace-live-synch-state": [{
              matrix: "workspace-live-sync-matrix",
              source: "/tmp/workspace-live-sync-matrix.json",
              id: "managed",
              status: "passed",
            }],
          },
        },
      }],
    }),
    /reports\[0\]\.matrixCoverage\.runtimeSignalScenarios has unknown runtime signal "workspace-live-synch-state"/,
  )
})

function generatedEvidenceFixture() {
  return {
    validationSuites: {
      enabled: true,
      artifactIndexes: [
        "/tmp/suites/cloud/arroba-drill-artifacts.json",
        "/tmp/suites/oss/arroba-drill-artifacts.json",
      ],
      outputRoots: ["/tmp/suites/cloud", "/tmp/suites/oss"],
    },
    matrixReports: {
      enabled: true,
      roots: ["/tmp/matrices/cloud", "/tmp/matrices/oss"],
      dryRun: false,
      continueOnFailure: true,
      commands: [{
        artifactIndexPath: "/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json",
        args: ["--include-hetzner"],
        cwd: "/repo/arroba",
        reportPath: "/tmp/matrices/oss/native-provider-tui-matrix.json",
        scriptPath: "/repo/arroba/apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs",
      }],
    },
  }
}

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
        requiredArtifactKinds: ["validation-suite-run"],
        missingArtifactKinds: [],
        requiredArtifactEvidenceRepos: ["oss"],
        missingArtifactEvidenceRepos: [],
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
          artifactKinds: {
            "validation-suite-run": 1,
          },
          evidenceRepos: {
            oss: 1,
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
          owners: {
            "runtime-state": 1,
          },
          classifications: {
            "workspace-live-sync-conflict": 1,
          },
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
