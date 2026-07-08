import {
  assert,
  drillValidationGateAggregateExitCode,
  formatDrillValidationGateAggregateSummary,
  generatedEvidenceFixture,
  reportFixture,
  summarizeValidationGateReportAggregate,
  test,
  validateDrillValidationGateAggregate,
} from '../drill-validation-gate-aggregate.test-support.mjs'

test("fails aggregate requirements missing from otherwise passing reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    normalizedRequiredPresets: ["remote-home-extension"],
    normalizedAggregateRequirements: {
      requiredPlatformCoverageAreas: ["hosted-cloud-drills"],
      requiredArtifactSchemas: ["arroba.drill.matrix.v1"],
      requiredArtifactKinds: ["matrix-report"],
      requiredArtifactEvidenceRepos: ["cloud"],
      requiredArtifactRuntimeAuthorityInvariants: ["worker-execution-authority"],
      requiredFailureClassifications: ["remote-extension-sync"],
      requiredMatrices: ["remote-home-extension-matrix"],
      requiredMatrixClassifications: ["remote-extension-sync"],
      requiredMatrixRuntimeSignals: ["home-extension-manifest-sync"],
      requiredDeploymentPresets: ["hosted-cloud"],
      requiredProviders: ["claude"],
      requiredScenarios: ["hetzner-collab"],
      requiredGeneratedEvidenceKinds: ["matrix-report"],
      requiredGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
    },
    validateReport: () => {},
  })

  assert.equal(aggregate.status, "failed")
  assert.equal(drillValidationGateAggregateExitCode(aggregate), 1)
  assert.deepEqual(aggregate.missingPresets, ["remote-home-extension"])
  assert.deepEqual(aggregate.missingArtifactSchemas, ["arroba.drill.matrix.v1"])
  assert.deepEqual(aggregate.missingArtifactKinds, ["matrix-report"])
  assert.deepEqual(aggregate.missingArtifactEvidenceRepos, ["cloud"])
  assert.deepEqual(aggregate.missingArtifactRuntimeAuthorityInvariants, ["worker-execution-authority"])
  assert.deepEqual(aggregate.missingMatrixRuntimeSignals, ["home-extension-manifest-sync"])
  assert.deepEqual(aggregate.missingProviders, ["claude"])
  assert.deepEqual(aggregate.missingScenarios, ["hetzner-collab"])
  assert.deepEqual(aggregate.missingGeneratedEvidenceKinds, ["matrix-report"])
  assert.deepEqual(aggregate.missingGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
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
        nextAction: "provide validation gate reports with artifact runtime authority invariants: worker-execution-authority",
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
        classification: "generated-evidence",
        nextAction: "provide validation gate reports with generated matrix limitations: dry-run-classification-coverage",
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
      requiredGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
      requiredGeneratedValidationSuiteArtifactIndexes: [
        "/tmp/suites/cloud/arroba-drill-artifacts.json",
        "/tmp/suites/oss/arroba-drill-artifacts.json",
      ],
      requiredGeneratedValidationSuiteFailureRoots: ["/tmp/suites/cloud/failed-run", "/tmp/suites/oss/failed-run"],
    },
    validateReport: () => {},
  })

  assert.deepEqual(aggregate.coverage.generatedEvidenceKinds, {
    "matrix-report": 1,
    "validation-suite-run": 1,
  })
  assert.deepEqual(aggregate.coverage.generatedMatrixLimitations, {
    "dry-run-classification-coverage": 1,
  })
  assert.deepEqual(aggregate.coverage.generatedValidationSuiteArtifactIndexes, {
    "/tmp/suites/cloud/arroba-drill-artifacts.json": 1,
    "/tmp/suites/oss/arroba-drill-artifacts.json": 1,
  })
  assert.deepEqual(aggregate.coverage.generatedValidationSuiteFailureRoots, {
    "/tmp/suites/cloud/failed-run": 1,
    "/tmp/suites/oss/failed-run": 1,
  })
  assert.deepEqual(aggregate.requiredGeneratedEvidenceKinds, ["matrix-report", "validation-suite-run"])
  assert.deepEqual(aggregate.missingGeneratedEvidenceKinds, [])
  assert.deepEqual(aggregate.requiredGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
  assert.deepEqual(aggregate.missingGeneratedMatrixLimitations, [])
  assert.deepEqual(aggregate.requiredGeneratedValidationSuiteArtifactIndexes, [
    "/tmp/suites/cloud/arroba-drill-artifacts.json",
    "/tmp/suites/oss/arroba-drill-artifacts.json",
  ])
  assert.deepEqual(aggregate.missingGeneratedValidationSuiteArtifactIndexes, [])
  assert.deepEqual(aggregate.requiredGeneratedValidationSuiteFailureRoots, ["/tmp/suites/cloud/failed-run", "/tmp/suites/oss/failed-run"])
  assert.deepEqual(aggregate.missingGeneratedValidationSuiteFailureRoots, [])
  assert.deepEqual(aggregate.coverage.requiredGeneratedEvidenceKinds, {
    "matrix-report": 1,
    "validation-suite-run": 1,
  })
  assert.deepEqual(aggregate.coverage.missingGeneratedEvidenceKinds, {})
  assert.deepEqual(aggregate.coverage.requiredGeneratedMatrixLimitations, {
    "dry-run-classification-coverage": 1,
  })
  assert.deepEqual(aggregate.coverage.missingGeneratedMatrixLimitations, {})
  assert.deepEqual(aggregate.coverage.requiredGeneratedValidationSuiteArtifactIndexes, {
    "/tmp/suites/cloud/arroba-drill-artifacts.json": 1,
    "/tmp/suites/oss/arroba-drill-artifacts.json": 1,
  })
  assert.deepEqual(aggregate.coverage.missingGeneratedValidationSuiteArtifactIndexes, {})
  assert.deepEqual(aggregate.coverage.requiredGeneratedValidationSuiteFailureRoots, {
    "/tmp/suites/cloud/failed-run": 1,
    "/tmp/suites/oss/failed-run": 1,
  })
  assert.deepEqual(aggregate.coverage.missingGeneratedValidationSuiteFailureRoots, {})
  assert.deepEqual(aggregate.reports[0].generatedEvidence, {
    kinds: ["validation-suite-run", "matrix-report"],
    validationSuites: {
      enabled: true,
      artifactIndexes: [
        "/tmp/suites/cloud/arroba-drill-artifacts.json",
        "/tmp/suites/oss/arroba-drill-artifacts.json",
      ],
      failureRoots: [
        "/tmp/suites/cloud/failed-run",
        "/tmp/suites/oss/failed-run",
      ],
      commands: [
        {
          artifactIndexPath: "/tmp/suites/oss/arroba-drill-artifacts.json",
          args: ["--run-json", "--preserve-failure-root", "/tmp/suites/oss/failed-run"],
          cwd: "/repo/arroba",
          failureRoot: "/tmp/suites/oss/failed-run",
          nodeArgs: ["/repo/arroba/apps/cli/scripts/drill-validation-suite.mjs", "--run-json", "--output", "/tmp/suites/oss/drill-validation-suite-run.json", "--output-artifact-index", "/tmp/suites/oss/arroba-drill-artifacts.json", "--preserve-failure-root", "/tmp/suites/oss/failed-run"],
          reportPath: "/tmp/suites/oss/drill-validation-suite-run.json",
          scriptPath: "/repo/arroba/apps/cli/scripts/drill-validation-suite.mjs",
        },
        {
          artifactIndexPath: "/tmp/suites/cloud/arroba-drill-artifacts.json",
          args: ["--run-json", "--preserve-failure-root", "/tmp/suites/cloud/failed-run"],
          cwd: "/repo/arroba-cloud",
          failureRoot: "/tmp/suites/cloud/failed-run",
          nodeArgs: ["/repo/arroba-cloud/scripts/cloud-validation-suite.mjs", "--run-json", "--output", "/tmp/suites/cloud/cloud-validation-suite-run.json", "--output-artifact-index", "/tmp/suites/cloud/arroba-drill-artifacts.json", "--preserve-failure-root", "/tmp/suites/cloud/failed-run"],
          reportPath: "/tmp/suites/cloud/cloud-validation-suite-run.json",
          scriptPath: "/repo/arroba-cloud/scripts/cloud-validation-suite.mjs",
        },
      ],
      outputRoots: ["/tmp/suites/cloud", "/tmp/suites/oss"],
    },
    matrixReports: {
      enabled: true,
      artifactIndexes: ["/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json"],
      roots: ["/tmp/matrices/cloud", "/tmp/matrices/oss"],
      dryRun: false,
      continueOnFailure: true,
      limitations: [{
        kind: "dry-run-classification-coverage",
        owner: "validation-harness",
        nextAction: "rerun distributed runtime matrix reports without --matrix-dry-run before release",
      }],
      commands: [{
        artifactIndexPath: "/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json",
        args: ["--include-hetzner"],
        artifactIndexFlag: "--artifact-index",
        cwd: "/repo/arroba",
        matrix: "native-provider-tui-matrix",
        nodeArgs: ["/repo/arroba/apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs", "--include-hetzner", "--report", "/tmp/matrices/oss/native-provider-tui-matrix.json", "--artifact-index", "/tmp/matrices/oss/native-provider-tui-matrix-artifacts.json"],
        repo: "oss",
        reportPath: "/tmp/matrices/oss/native-provider-tui-matrix.json",
        scriptPath: "/repo/arroba/apps/cli/scripts/live-native-provider-tui-matrix-drill.mjs",
      }],
    },
  })
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /- generated_evidence_kinds: matrix-report=1 validation-suite-run=1/)
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /- generated_matrix_limitations: dry-run-classification-coverage=1/)
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /- generated_validation_suite_artifact_indexes: \/tmp\/suites\/cloud\/arroba-drill-artifacts\.json=1 \/tmp\/suites\/oss\/arroba-drill-artifacts\.json=1/)
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /required_generated_evidence_kinds=matrix-report,validation-suite-run missing=none/)
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /required_generated_matrix_limitations=dry-run-classification-coverage missing=none/)
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /required_generated_validation_suite_artifact_indexes=\/tmp\/suites\/cloud\/arroba-drill-artifacts\.json,\/tmp\/suites\/oss\/arroba-drill-artifacts\.json missing=none/)
})

test("rejects secret-looking generated evidence paths in validation gate aggregates", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture({
    generatedEvidence: generatedEvidenceFixture(),
  })], {
    sources: ["distributed-runtime-gate.json"],
    normalizedAggregateRequirements: {
      requiredGeneratedValidationSuiteFailureRoots: ["/tmp/suites/cloud/failed-run"],
    },
    validateReport: () => {},
  })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredGeneratedValidationSuiteArtifactIndexes: ["/tmp/Bearer abcdefghijklmnop.json"],
    }),
    /requiredGeneratedValidationSuiteArtifactIndexes\[0\] includes secret-looking generated evidence path/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredGeneratedValidationSuiteFailureRoots: ["/tmp/Bearer abcdefghijklmnop"],
    }),
    /requiredGeneratedValidationSuiteFailureRoots\[0\] includes secret-looking generated evidence path/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        generatedValidationSuiteArtifactIndexes: { "/tmp/Bearer abcdefghijklmnop.json": 1 },
      },
    }),
    /coverage\.generatedValidationSuiteArtifactIndexes\.\/tmp\/Bearer abcdefghijklmnop\.json includes secret-looking generated evidence path/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        generatedValidationSuiteFailureRoots: { "/tmp/Bearer abcdefghijklmnop": 1 },
      },
    }),
    /coverage\.generatedValidationSuiteFailureRoots\.\/tmp\/Bearer abcdefghijklmnop includes secret-looking generated evidence path/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        generatedEvidence: {
          ...aggregate.reports[0].generatedEvidence,
          validationSuites: {
            ...aggregate.reports[0].generatedEvidence.validationSuites,
            outputRoots: ["/tmp/Bearer abcdefghijklmnop"],
          },
        },
      }],
    }),
    /reports\[0\]\.generatedEvidence\.validationSuites\.outputRoots\[0\] includes secret-looking generated evidence path/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        generatedEvidence: {
          ...aggregate.reports[0].generatedEvidence,
          matrixReports: {
            ...aggregate.reports[0].generatedEvidence.matrixReports,
            commands: [{
              ...aggregate.reports[0].generatedEvidence.matrixReports.commands[0],
              args: ["--artifact-index", "/tmp/Bearer abcdefghijklmnop.json"],
            }],
          },
        },
      }],
    }),
    /reports\[0\]\.generatedEvidence\.matrixReports\.commands\[0\]\.args\[1\] includes secret-looking generated evidence path/,
  )
  {
    const { nodeArgs: _nodeArgs, ...commandWithoutNodeArgs } = aggregate.reports[0].generatedEvidence.matrixReports.commands[0]
    assert.throws(
      () => validateDrillValidationGateAggregate({
        ...aggregate,
        reports: [{
          ...aggregate.reports[0],
          generatedEvidence: {
            ...aggregate.reports[0].generatedEvidence,
            matrixReports: {
              ...aggregate.reports[0].generatedEvidence.matrixReports,
              commands: [commandWithoutNodeArgs],
            },
          },
        }],
      }),
      /reports\[0\]\.generatedEvidence\.matrixReports\.commands\[0\]\.nodeArgs is not an array/,
    )
  }
  {
    const { artifactIndexFlag: _artifactIndexFlag, ...commandWithoutFlag } = aggregate.reports[0].generatedEvidence.matrixReports.commands[0]
    assert.throws(
      () => validateDrillValidationGateAggregate({
        ...aggregate,
        reports: [{
          ...aggregate.reports[0],
          generatedEvidence: {
            ...aggregate.reports[0].generatedEvidence,
            matrixReports: {
              ...aggregate.reports[0].generatedEvidence.matrixReports,
              commands: [commandWithoutFlag],
            },
          },
        }],
      }),
      /reports\[0\]\.generatedEvidence\.matrixReports\.commands\[0\] has invalid artifactIndexFlag/,
    )
  }
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        generatedEvidence: {
          ...aggregate.reports[0].generatedEvidence,
          matrixReports: {
            ...aggregate.reports[0].generatedEvidence.matrixReports,
            commands: [{
              ...aggregate.reports[0].generatedEvidence.matrixReports.commands[0],
              repo: "",
            }],
          },
        },
      }],
    }),
    /reports\[0\]\.generatedEvidence\.matrixReports\.commands\[0\] has invalid repo/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        generatedEvidence: {
          ...aggregate.reports[0].generatedEvidence,
          matrixReports: {
            ...aggregate.reports[0].generatedEvidence.matrixReports,
            commands: [{
              ...aggregate.reports[0].generatedEvidence.matrixReports.commands[0],
              matrix: "workspace-live-synch-matrix",
            }],
          },
        },
      }],
    }),
    /reports\[0\]\.generatedEvidence\.matrixReports\.commands\[0\]\.matrix has unknown generated matrix name "workspace-live-synch-matrix"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        generatedEvidence: {
          ...aggregate.reports[0].generatedEvidence,
          matrixReports: {
            ...aggregate.reports[0].generatedEvidence.matrixReports,
            commands: [{
              ...aggregate.reports[0].generatedEvidence.matrixReports.commands[0],
              repo: "osz",
            }],
          },
        },
      }],
    }),
    /reports\[0\]\.generatedEvidence\.matrixReports\.commands\[0\]\.repo has unknown evidence repo "osz"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        generatedEvidence: {
          ...aggregate.reports[0].generatedEvidence,
          matrixReports: {
            ...aggregate.reports[0].generatedEvidence.matrixReports,
            commands: [{
              ...aggregate.reports[0].generatedEvidence.matrixReports.commands[0],
              matrix: "cloud-slice-runtime-matrix",
              repo: "oss",
            }],
          },
        },
      }],
    }),
    /reports\[0\]\.generatedEvidence\.matrixReports\.commands\[0\]\.repo does not match generated matrix "cloud-slice-runtime-matrix"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        artifactCoverage: {
          ...aggregate.reports[0].artifactCoverage,
          generatedValidationSuiteArtifactIndexes: { "/tmp/Bearer abcdefghijklmnop.json": 1 },
        },
      }],
    }),
    /reports\[0\]\.artifactCoverage\.generatedValidationSuiteArtifactIndexes\.\/tmp\/Bearer abcdefghijklmnop\.json includes secret-looking generated evidence path/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        artifactCoverage: {
          ...aggregate.reports[0].artifactCoverage,
          generatedValidationSuiteFailureRoots: { "/tmp/Bearer abcdefghijklmnop": 1 },
        },
      }],
    }),
    /reports\[0\]\.artifactCoverage\.generatedValidationSuiteFailureRoots\.\/tmp\/Bearer abcdefghijklmnop includes secret-looking generated evidence path/,
  )
})

test("formats supplemental artifact coverage inputs without inflating report totals", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    sources: ["distributed-runtime-gate.json"],
    supplementalArtifactReports: [reportFixture()],
    supplementalArtifactSources: ["distributed-runtime-gate-artifacts.json"],
    normalizedAggregateRequirements: {
      requiredArtifactGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
    },
    validateReport: () => {},
  })
  const text = formatDrillValidationGateAggregateSummary(aggregate)

  assert.deepEqual(aggregate.totals, { reports: 1, passed: 1, failed: 0 })
  assert.equal(aggregate.artifactCoverageInputs.length, 1)
  assert.deepEqual(aggregate.artifactCoverageInputs.map((input) => input.source), ["distributed-runtime-gate-artifacts.json"])
  assert.deepEqual(aggregate.artifactCoverageInputs.map((input) => input.status), ["passed"])
  assert.deepEqual(aggregate.coverage.artifactCoverageInputSources, {
    "artifact metadata inputs": 2,
  })
  assert.deepEqual(aggregate.coverage.artifactGeneratedValidationSuiteFailureRoots, {
    "/tmp/generated-suite/failed-run": 2,
  })
  assert.match(text, /status=passed reports=1 passed=1 failed=0/)
  assert.match(text, /artifact_coverage_inputs=1 failed=0 sources=distributed-runtime-gate-artifacts\.json/)
  assert.match(text, /- artifact_coverage_input_sources: artifact metadata inputs=2/)
  assert.match(text, /- artifact_generated_validation_suite_failure_roots: \/tmp\/generated-suite\/failed-run=2/)
  assert.match(text, /required_artifact_generated_matrix_limitations=dry-run-classification-coverage missing=none/)
})

test("failed supplemental artifact coverage inputs fail the aggregate without inflating report totals", () => {
  const failedArtifactReport = {
    ...reportFixture(),
    status: "failed",
    checks: {
      ...reportFixture().checks,
      artifacts: {
        ...reportFixture().checks.artifacts,
        status: "failed",
        requiredArtifactMaxAgeMs: 100,
        staleArtifactIndexes: [{
          source: "/tmp/arroba-drill-artifacts.json",
          createdAt: "2026-01-01T00:00:00.000Z",
          ageMs: 1000,
          maxAgeMs: 100,
        }],
      },
    },
    nextActions: [{
      owner: "validation-harness",
      classification: "artifact-staleness",
      nextAction: "regenerate stale drill artifact indexes, then rerun the validation gate",
      count: 1,
    }],
  }
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    supplementalArtifactReports: [failedArtifactReport],
    supplementalArtifactSources: ["distributed-runtime-gate-artifacts.json"],
    validateReport: () => {},
  })
  const text = formatDrillValidationGateAggregateSummary(aggregate)

  assert.deepEqual(aggregate.totals, { reports: 1, passed: 1, failed: 0 })
  assert.equal(aggregate.status, "failed")
  assert.deepEqual(aggregate.artifactCoverageInputs.map((input) => input.status), ["failed"])
  assert.match(text, /artifact_coverage_inputs=1 failed=1 sources=distributed-runtime-gate-artifacts\.json/)
  assert.deepEqual(aggregate.nextActions.map(({ classification }) => classification), ["artifact-staleness"])
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
