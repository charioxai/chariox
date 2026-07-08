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

test("summarizes validation gate reports with aggregate requirements", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], {
    sources: ["workspace-live-sync.json"],
    normalizedRequiredPresets: ["workspace-live-sync"],
    normalizedAggregateRequirements: {
      requiredPlatformCoverageAreas: ["runtime-fixtures"],
      requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
      requiredArtifactKinds: ["validation-suite-run"],
      requiredArtifactGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
      requiredArtifactEvidenceRepos: ["oss"],
      requiredArtifactProviderAccountAliases: ["codex=work"],
      requiredArtifactValidationPresets: ["distributed-runtime"],
      requiredArtifactRuntimeAuthorityInvariants: ["home-session-authority"],
      requiredFailureClassifications: ["kernel-authority"],
      requiredMatrices: ["workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["workspace-live-sync-conflict"],
      requiredMatrixRuntimeSignals: ["workspace-live-sync-state"],
      requiredDeploymentPresets: ["local"],
      requiredProviders: ["codex"],
      requiredScenarios: ["managed"],
      requiredGeneratedEvidenceKinds: [],
      requiredGeneratedMatrixLimitations: [],
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
  assert.deepEqual(aggregate.coverage.artifactRuntimeAuthorityInvariants, {
    "home-session-authority": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactGeneratedMatrixLimitations, {
    "dry-run-classification-coverage": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactGeneratedValidationSuiteFailureRoots, {
    "/tmp/generated-suite/failed-run": 1,
  })
  assert.deepEqual(aggregate.requiredArtifactGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
  assert.deepEqual(aggregate.missingArtifactGeneratedMatrixLimitations, [])
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
  assert.deepEqual(aggregate.coverage.artifactExitCriterionStatuses, {
    "dry-run": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactIncompleteExitCriterionStatuses, {
    "dry-run": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactKinds, {
    "validation-suite-run": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactEvidenceRepos, {
    oss: 1,
  })
  assert.deepEqual(aggregate.coverage.artifactProviderAccountAliases, {
    "codex=work": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactValidationPresets, {
    "distributed-runtime": 1,
  })
  assert.deepEqual(aggregate.coverage.artifactCoverageInputSources, {
    "artifact metadata inputs": 1,
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
  assert.deepEqual(aggregate.missingArtifactProviderAccountAliases, [])
  assert.deepEqual(aggregate.missingArtifactValidationPresets, [])
  assert.deepEqual(aggregate.requiredArtifactRuntimeAuthorityInvariants, ["home-session-authority"])
  assert.deepEqual(aggregate.missingArtifactRuntimeAuthorityInvariants, [])
  assert.deepEqual(aggregate.reports[0].source, "workspace-live-sync.json")
  assert.deepEqual(aggregate.reports[0].artifactCoverage.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
  assert.deepEqual(aggregate.reports[0].artifactCoverage.requiredArtifactKinds, ["validation-suite-run"])
  assert.deepEqual(aggregate.reports[0].artifactCoverage.requiredArtifactEvidenceRepos, ["oss"])
  assert.deepEqual(aggregate.reports[0].artifactCoverage.requiredArtifactProviderAccountAliases, ["codex=work"])
  assert.deepEqual(aggregate.reports[0].artifactCoverage.requiredArtifactValidationPresets, ["distributed-runtime"])
  assert.deepEqual(aggregate.reports[0].artifactCoverage.requiredArtifactRuntimeAuthorityInvariants, ["home-session-authority"])
  assert.deepEqual(aggregate.reports[0].artifactCoverage.runtimeAuthorityInvariants, {
    "home-session-authority": 1,
  })
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
  assert.deepEqual(aggregate.reports[0].artifactCoverage.exitCriterionStatuses, {
    "dry-run": 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.incompleteExitCriterionStatuses, {
    "dry-run": 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.artifactKinds, {
    "validation-suite-run": 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.generatedValidationSuiteFailureRoots, {
    "/tmp/generated-suite/failed-run": 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.generatedValidationSuiteArtifactIndexes, {
    "/tmp/generated-suite/arroba-drill-artifacts.json": 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.evidenceRepos, {
    oss: 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.providerAccountAliases, {
    "codex=work": 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.validationPresets, {
    "distributed-runtime": 1,
  })
  assert.deepEqual(aggregate.reports[0].artifactCoverage.artifactCoverageInputSources, {
    "artifact metadata inputs": 1,
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
  assert.match(text, /- artifact_coverage_input_sources: artifact metadata inputs=1/)
  assert.match(text, /required_artifact_schemas=arroba\.drill\.validation_suite_run\.v1 missing=none/)
  assert.match(text, /required_artifact_kinds=validation-suite-run missing=none/)
  assert.match(text, /required_artifact_evidence_repos=oss missing=none/)
  assert.match(text, /required_artifact_validation_presets=distributed-runtime missing=none/)
  assert.match(text, /- artifact_schemas: arroba.drill.validation_suite_run.v1=1/)
  assert.match(text, /- artifact_runtime_signals: session-authority=2 workspace-live-sync-state=1/)
  assert.match(text, /- artifact_runtime_signal_owners: kernel-authority=1 runtime-state=1/)
  assert.match(text, /- artifact_owners: validation-platform=1/)
  assert.match(text, /- artifact_classifications: cloud-validation-suite=1/)
  assert.match(text, /- artifact_exit_criterion_statuses: dry-run=1/)
  assert.match(text, /- artifact_incomplete_exit_criterion_statuses: dry-run=1/)
  assert.match(text, /- artifact_kinds: validation-suite-run=1/)
  assert.match(text, /- artifact_generated_validation_suite_failure_roots: \/tmp\/generated-suite\/failed-run=1/)
  assert.match(text, /- artifact_evidence_repos: oss=1/)
  assert.match(text, /- artifact_validation_presets: distributed-runtime=1/)
  assert.match(text, /- matrix_runtime_signals: workspace-live-sync-state=1/)
  assert.match(text, /- matrix_runtime_signal_owners: runtime-state=1/)
  assert.match(text, /- matrix_owners: runtime-state=1/)
  assert.match(text, /- matrix_classifications: workspace-live-sync-conflict=1/)
  assert.match(text, /matrix_runtime_signal_sources:/)
  assert.match(text, /- workspace-live-sync-state: workspace-live-sync-matrix\/managed\(passed\) source=\/tmp\/workspace-live-sync-matrix\.json report=workspace-live-sync\.json/)
})

test("summarizes stale matrix reports from gate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture({
    status: "failed",
    checks: {
      ...reportFixture().checks,
      matrices: {
        ...reportFixture().checks.matrices,
        status: "failed",
        requiredMatrixMaxAgeMs: 100,
        staleMatrixReports: [{
          source: "/tmp/workspace-live-sync-matrix.json",
          matrix: "workspace-live-sync-matrix",
          completedAt: "2026-01-01T00:00:00.000Z",
          ageMs: 1000,
          maxAgeMs: 100,
        }],
      },
    },
    nextActions: [{
      owner: "validation-harness",
      classification: "matrix-staleness",
      nextAction: "regenerate stale matrix reports, then rerun the validation gate",
      count: 1,
      sourceDetails: [{
        source: "workspace-live-sync-matrix/managed",
        matrix: "workspace-live-sync-matrix",
        scenarioId: "managed",
        reportPath: "/tmp/workspace-live-sync-matrix.json",
      }],
    }],
  })], {
    sources: ["distributed-runtime-gate.json"],
    validateReport: () => {},
  })
  const text = formatDrillValidationGateAggregateSummary(aggregate)

  assert.equal(aggregate.status, "failed")
  assert.deepEqual(aggregate.coverage.matrixStaleReports, {
    "/tmp/workspace-live-sync-matrix.json": 1,
  })
  assert.deepEqual(aggregate.reports[0].matrixCoverage.staleMatrixReports, [{
    source: "/tmp/workspace-live-sync-matrix.json",
    matrix: "workspace-live-sync-matrix",
    completedAt: "2026-01-01T00:00:00.000Z",
    ageMs: 1000,
    maxAgeMs: 100,
  }])
  assert.match(text, /- matrix_stale_reports: \/tmp\/workspace-live-sync-matrix\.json=1/)
  assert.match(text, /classification=matrix-staleness count=1/)
  assert.deepEqual(aggregate.nextActions[0].sourceDetails, [{
    source: "workspace-live-sync-matrix/managed",
    matrix: "workspace-live-sync-matrix",
    scenarioId: "managed",
    reportPath: "/tmp/workspace-live-sync-matrix.json",
  }])
  assert.match(text, /sources: workspace-live-sync-matrix\/managed report=\/tmp\/workspace-live-sync-matrix\.json/)
})

test("summarizes stale failure manifests from gate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture({
    status: "failed",
    checks: {
      ...reportFixture().checks,
      failures: {
        ...reportFixture().checks.failures,
        status: "failed",
        requiredFailureMaxAgeMs: 100,
        staleFailureManifests: [{
          source: "/tmp/arroba-drill-failure.json",
          drill: "workspace-live-sync",
          failedAt: "2026-01-01T00:00:00.000Z",
          ageMs: 1000,
          maxAgeMs: 100,
        }],
      },
    },
    nextActions: [{
      owner: "validation-harness",
      classification: "failure-artifacts",
      nextAction: "regenerate stale preserved failure bundles or rerun the failing drills before routing them",
      count: 1,
    }],
  })], {
    sources: ["distributed-runtime-gate.json"],
    validateReport: () => {},
  })
  const text = formatDrillValidationGateAggregateSummary(aggregate)

  assert.equal(aggregate.status, "failed")
  assert.deepEqual(aggregate.coverage.failureStaleManifests, {
    "/tmp/arroba-drill-failure.json": 1,
  })
  assert.deepEqual(aggregate.reports[0].failureCoverage.staleFailureManifests, [{
    source: "/tmp/arroba-drill-failure.json",
    drill: "workspace-live-sync",
    failedAt: "2026-01-01T00:00:00.000Z",
    ageMs: 1000,
    maxAgeMs: 100,
  }])
  assert.match(text, /- failure_stale_manifests: \/tmp\/arroba-drill-failure\.json=1/)
  assert.match(text, /classification=failure-artifacts count=1/)
})

test("rejects unknown artifact evidence repo labels in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredArtifactGeneratedEvidenceRepos: ["cluod"],
    }),
    /requiredArtifactGeneratedEvidenceRepos\[0\] has unknown evidence repo "cluod"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        artifactGeneratedEvidenceRepos: { cluod: 1 },
      },
    }),
    /coverage\.artifactGeneratedEvidenceRepos has unknown evidence repo "cluod"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        artifactCoverage: {
          ...aggregate.reports[0].artifactCoverage,
          generatedEvidenceRepos: { cluod: 1 },
        },
      }],
    }),
    /reports\[0\]\.artifactCoverage\.generatedEvidenceRepos has unknown evidence repo "cluod"/,
  )
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

test("rejects unknown artifact provider account alias labels in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredArtifactProviderAccountAliases: ["cdoex=work"],
    }),
    /requiredArtifactProviderAccountAliases\[0\] has unknown provider account alias provider "cdoex"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        artifactProviderAccountAliases: { "codex=sk-secretsecretsecretsecret": 1 },
      },
    }),
    /provider account alias must be a non-secret label/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        artifactCoverage: {
          ...aggregate.reports[0].artifactCoverage,
          providerAccountAliases: { "cdoex=work": 1 },
        },
      }],
    }),
    /reports\[0\]\.artifactCoverage\.providerAccountAliases has unknown provider account alias provider "cdoex"/,
  )
})

test("rejects unknown artifact validation preset labels in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredArtifactValidationPresets: ["distributed-runtmie"],
    }),
    /requiredArtifactValidationPresets\[0\] has unknown artifact validation preset "distributed-runtmie"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        artifactValidationPresets: { "distributed-runtmie": 1 },
      },
    }),
    /coverage\.artifactValidationPresets has unknown artifact validation preset "distributed-runtmie"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        artifactCoverage: {
          ...aggregate.reports[0].artifactCoverage,
          validationPresets: { "distributed-runtmie": 1 },
        },
      }],
    }),
    /reports\[0\]\.artifactCoverage\.validationPresets has unknown artifact validation preset "distributed-runtmie"/,
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

test("rejects unknown generated matrix limitation labels in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture({
    generatedEvidence: {
      kinds: ["matrix-report"],
      validationSuites: {
        enabled: false,
        artifactIndexes: [],
        outputRoots: [],
      },
      matrixReports: {
        enabled: true,
        roots: ["/tmp/generated-matrix"],
        commands: [{
          artifactIndexFlag: "--artifact-index",
          artifactIndexPath: "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
          args: ["--dry-run"],
          cwd: "/repo/arroba",
          matrix: "workspace-live-sync-matrix",
          nodeArgs: ["/repo/arroba/apps/cli/scripts/live-workspace-live-sync-matrix-drill.mjs", "--dry-run", "--report", "/tmp/generated-matrix/workspace-live-sync-matrix.json", "--artifact-index", "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"],
          repo: "oss",
          reportPath: "/tmp/generated-matrix/workspace-live-sync-matrix.json",
          scriptPath: "/repo/arroba/apps/cli/scripts/live-workspace-live-sync-matrix-drill.mjs",
        }],
        dryRun: true,
        continueOnFailure: false,
        limitations: [{
          kind: "dry-run-classification-coverage",
          owner: "validation-harness",
          nextAction: "rerun generated matrix reports without --dry-run before release",
        }],
      },
    },
  })], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredGeneratedMatrixLimitations: ["dry-run-classification-covergae"],
    }),
    /requiredGeneratedMatrixLimitations\[0\] has unknown generated matrix limitation "dry-run-classification-covergae"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        generatedMatrixLimitations: { "dry-run-classification-covergae": 1 },
      },
    }),
    /coverage\.generatedMatrixLimitations has unknown generated matrix limitation "dry-run-classification-covergae"/,
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
            limitations: [{
              kind: "dry-run-classification-covergae",
              owner: "validation-harness",
              nextAction: "rerun generated matrix reports without --dry-run before release",
            }],
          },
        },
      }],
    }),
    /reports\[0\]\.generatedEvidence\.matrixReports\.limitations\[0\] has unknown generated matrix limitation "dry-run-classification-covergae"/,
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

test("rejects unknown artifact exit criterion status labels in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        artifactExitCriterionStatuses: { satisifed: 1 },
      },
    }),
    /coverage\.artifactExitCriterionStatuses has unknown exit criterion status "satisifed"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        artifactCoverage: {
          ...aggregate.reports[0].artifactCoverage,
          incompleteExitCriterionStatuses: { pending: 1 },
        },
      }],
    }),
    /reports\[0\]\.artifactCoverage\.incompleteExitCriterionStatuses has unknown exit criterion status "pending"/,
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

test("rejects unknown validation gate preset labels in aggregate reports", () => {
  const aggregate = summarizeValidationGateReportAggregate([reportFixture()], { validateReport: () => {} })

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      requiredPresets: ["workspace-live-synch"],
    }),
    /requiredPresets\[0\] has unknown validation gate preset "workspace-live-synch"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      missingPresets: ["workspace-live-synch"],
    }),
    /missingPresets\[0\] has unknown validation gate preset "workspace-live-synch"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        presets: { "workspace-live-synch": 1 },
      },
    }),
    /coverage\.presets has unknown validation gate preset "workspace-live-synch"/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      reports: [{
        ...aggregate.reports[0],
        presets: ["workspace-live-synch"],
      }],
    }),
    /reports\[0\]\.presets\[0\] has unknown validation gate preset "workspace-live-synch"/,
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
