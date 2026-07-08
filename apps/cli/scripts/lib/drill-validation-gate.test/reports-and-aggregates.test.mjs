import {
  assert,
  describeDrillValidationGatePresets,
  drillValidationGateExitCode,
  findDrillValidationGateAggregatePaths,
  findDrillValidationGateReportPaths,
  formatDrillValidationGateAggregateSummary,
  formatDrillValidationGateSummary,
  mkdir,
  mkdtemp,
  os,
  path,
  readDrillValidationGateAggregate,
  readDrillValidationGateReport,
  rm,
  runDrillValidationGate,
  summarizeDrillValidationGateReports,
  test,
  validateDrillValidationGateAggregate,
  validateDrillValidationGateReport,
  writeDrillArtifactIndex,
  writeDrillPlatformBundle,
  writeFile,
  emptyArtifactCoverageSummary,
  matrixReport,
  platformValidationPresetSummaries,
  rewriteArtifactIndexCreatedAt,
  scenario,
  workspaceLiveSyncRequiredScenarios,
  writeFailureManifest,
  writeMatrixReport,
} from '../drill-validation-gate.test-support.mjs'

test("reads and discovers validation gate report artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await writeDrillPlatformBundle(bundleDir)
    const report = await runDrillValidationGate({ platformBundleDir: bundleDir })
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8")
    await writeFile(path.join(rootDir, "reports", "unrelated.json"), "{\"schema\":\"other\"}\n", "utf8")

    assert.deepEqual(await findDrillValidationGateReportPaths([rootDir]), [reportPath])
    assert.deepEqual(await readDrillValidationGateReport(reportPath), report)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("summarizes validation gate reports", async () => {
  const passed = await runDrillValidationGate({
    failureRoots: ["/tmp/no-such-arroba-failure-root"],
  })
  const failed = await runDrillValidationGate()
  const aggregate = summarizeDrillValidationGateReports([passed, failed], {
    sources: ["passed.json", "failed.json"],
  })

  assert.equal(aggregate.schema, "arroba.drill.validation_gate.aggregate.v1")
  assert.equal(aggregate.status, "failed")
  assert.deepEqual(aggregate.totals, { reports: 2, passed: 1, failed: 1 })
  assert.deepEqual(aggregate.reports.map((report) => report.source), ["passed.json", "failed.json"])
  assert.deepEqual(aggregate.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
    { owner: "validation-harness", classification: "validation-gate" },
  ])
  assert.doesNotThrow(() => validateDrillValidationGateAggregate(aggregate))
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /status=failed reports=2 passed=1 failed=1/)
})

test("summarizes validation gate matrix coverage across reports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      metadata: {
        deploymentPresets: "local,self-hosted-relay",
        providers: "codex,opencode",
      },
    }))
    const passed = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredPlatformCoverageAreas: ["runtime-fixtures"],
      requiredFailureClassifications: ["kernel-authority"],
      requiredMatrices: ["test-matrix"],
      requiredMatrixClassifications: ["kernel-authority"],
      requiredDeploymentPresets: ["local"],
      requiredProviders: ["codex"],
      requiredScenarios: ["local"],
    })
    const failed = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredPlatformCoverageAreas: ["hosted-cloud-drills"],
      requiredFailureClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
      requiredMatrices: ["hosted-matrix", "test-matrix"],
      requiredMatrixClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
      requiredDeploymentPresets: ["hosted-cloud", "local"],
      requiredProviders: ["claude", "codex"],
      requiredScenarios: ["remote"],
    })
    const aggregate = summarizeDrillValidationGateReports([passed, failed], {
      sources: ["passed.json", "failed.json"],
    })

    assert.equal(aggregate.status, "failed")
    assert.deepEqual(aggregate.coverage, {
      presets: {},
      requiredPlatformCoverageAreas: { "hosted-cloud-drills": 1, "runtime-fixtures": 1 },
      missingPlatformCoverageAreas: { "hosted-cloud-drills": 1, "runtime-fixtures": 1 },
      requiredArtifactCoverageAreas: {},
      missingArtifactCoverageAreas: {},
      requiredRuntimeSignals: {},
      missingRuntimeSignals: {},
      requiredRuntimeSignalOwners: {},
      missingRuntimeSignalOwners: {},
      requiredFailureClassifications: { "kernel-authority": 1, "remote-extension-sync": 1, "workspace-live-sync-conflict": 1 },
      missingFailureClassifications: { "kernel-authority": 1, "remote-extension-sync": 1, "workspace-live-sync-conflict": 1 },
      requiredArtifactSchemas: {},
      missingArtifactSchemas: {},
      requiredArtifactKinds: {},
      missingArtifactKinds: {},
      requiredArtifactGeneratedEvidenceKinds: {},
      missingArtifactGeneratedEvidenceKinds: {},
      requiredArtifactGeneratedEvidenceRepos: {},
      missingArtifactGeneratedEvidenceRepos: {},
      requiredArtifactGeneratedMatrixArtifactIndexes: {},
      missingArtifactGeneratedMatrixArtifactIndexes: {},
      requiredArtifactGeneratedMatrixLimitations: {},
      missingArtifactGeneratedMatrixLimitations: {},
      requiredArtifactGeneratedMatrixNames: {},
      missingArtifactGeneratedMatrixNames: {},
      requiredArtifactGeneratedMatrixRepos: {},
      missingArtifactGeneratedMatrixRepos: {},
      requiredArtifactGeneratedValidationSuiteArtifactIndexes: {},
      missingArtifactGeneratedValidationSuiteArtifactIndexes: {},
      requiredArtifactGeneratedValidationSuiteFailureRoots: {},
      missingArtifactGeneratedValidationSuiteFailureRoots: {},
      requiredArtifactEvidenceRepos: {},
      missingArtifactEvidenceRepos: {},
      requiredArtifactProviderAccountAliases: {},
      missingArtifactProviderAccountAliases: {},
      requiredArtifactValidationPresets: {},
      missingArtifactValidationPresets: {},
      requiredArtifactRuntimeAuthorityInvariants: {},
      missingArtifactRuntimeAuthorityInvariants: {},
      requiredArtifactRuntimeSignals: {},
      missingArtifactRuntimeSignals: {},
      requiredArtifactRuntimeSignalOwners: {},
      missingArtifactRuntimeSignalOwners: {},
      requiredArtifactOwners: {},
      missingArtifactOwners: {},
      requiredArtifactClassifications: {},
      missingArtifactClassifications: {},
      requiredArtifactFailureClassifications: {},
      missingArtifactFailureClassifications: {},
      requiredArtifactPlannedOwners: {},
      missingArtifactPlannedOwners: {},
      requiredArtifactPlannedClassifications: {},
      missingArtifactPlannedClassifications: {},
      requiredArtifactExitCriterionStatuses: {},
      missingArtifactExitCriterionStatuses: {},
      requiredArtifactIncompleteExitCriterionStatuses: {},
      missingArtifactIncompleteExitCriterionStatuses: {},
      artifactSchemas: {},
      artifactCoverageAreas: {},
      artifactRuntimeAuthorityInvariants: {},
      artifactRuntimeSignals: {},
      artifactRuntimeSignalOwners: {},
      artifactOwners: {},
      artifactClassifications: {},
      artifactFailureClassifications: {},
      artifactPlannedOwners: {},
      artifactPlannedClassifications: {},
      artifactExitCriterionStatuses: {},
      artifactIncompleteExitCriterionStatuses: {},
      artifactKinds: {},
      artifactGeneratedEvidenceKinds: {},
      artifactGeneratedEvidenceRepos: {},
      artifactGeneratedMatrixArtifactIndexes: {},
      artifactGeneratedMatrixLimitations: {},
      artifactGeneratedMatrixNames: {},
      artifactGeneratedMatrixRepos: {},
      artifactGeneratedValidationSuiteArtifactIndexes: {},
      artifactGeneratedValidationSuiteFailureRoots: {},
      artifactEvidenceRepos: {},
      artifactProviderAccountAliases: {},
      artifactValidationPresets: {},
      artifactCoverageInputSources: {},
      failureRuntimeSignals: {},
      failureRuntimeSignalOwners: {},
      failureOwners: {},
      failureClassifications: {},
      failureStaleManifests: {},
      matrixRuntimeSignals: {},
      matrixRuntimeSignalOwners: {},
      matrixOwners: {},
      matrixClassifications: {},
      matrixStaleReports: {},
      requiredMatrices: { "hosted-matrix": 1, "test-matrix": 2 },
      missingMatrices: { "hosted-matrix": 1 },
      requiredMatrixClassifications: { "kernel-authority": 1, "remote-extension-sync": 1, "workspace-live-sync-conflict": 1 },
      missingMatrixClassifications: { "kernel-authority": 1, "remote-extension-sync": 1, "workspace-live-sync-conflict": 1 },
      requiredMatrixRuntimeSignals: {},
      missingMatrixRuntimeSignals: {},
      requiredDeploymentPresets: { "hosted-cloud": 1, local: 2 },
      missingDeploymentPresets: { "hosted-cloud": 1 },
      requiredProviders: { claude: 1, codex: 2 },
      missingProviders: { claude: 1 },
      requiredScenarios: { local: 1, remote: 1 },
      missingScenarios: { remote: 1 },
      generatedEvidenceKinds: {},
      generatedMatrixArtifactIndexes: {},
      generatedMatrixLimitations: {},
      generatedValidationSuiteArtifactIndexes: {},
      generatedValidationSuiteFailureRoots: {},
      requiredGeneratedEvidenceKinds: {},
      missingGeneratedEvidenceKinds: {},
      requiredGeneratedMatrixArtifactIndexes: {},
      missingGeneratedMatrixArtifactIndexes: {},
      requiredGeneratedMatrixLimitations: {},
      missingGeneratedMatrixLimitations: {},
      requiredGeneratedValidationSuiteArtifactIndexes: {},
      missingGeneratedValidationSuiteArtifactIndexes: {},
      requiredGeneratedValidationSuiteFailureRoots: {},
      missingGeneratedValidationSuiteFailureRoots: {},
    })
    assert.deepEqual(aggregate.reports.map((report) => report.platformCoverage), [
      {
        requiredCoverageAreas: ["runtime-fixtures"],
        missingCoverageAreas: ["runtime-fixtures"],
        requiredRuntimeSignals: [],
        missingRuntimeSignals: [],
        requiredRuntimeSignalOwners: [],
        missingRuntimeSignalOwners: [],
        requiredFailureClassifications: ["kernel-authority"],
        missingFailureClassifications: ["kernel-authority"],
      },
      {
        requiredCoverageAreas: ["hosted-cloud-drills"],
        missingCoverageAreas: ["hosted-cloud-drills"],
        requiredRuntimeSignals: [],
        missingRuntimeSignals: [],
        requiredRuntimeSignalOwners: [],
        missingRuntimeSignalOwners: [],
        requiredFailureClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
        missingFailureClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
      },
    ])
    assert.deepEqual(aggregate.reports.map((report) => report.artifactCoverage), [
      emptyArtifactCoverageSummary(),
      emptyArtifactCoverageSummary(),
    ])
    assert.deepEqual(aggregate.reports.map((report) => report.failureCoverage), [
      { runtimeSignals: {}, runtimeSignalOwners: {}, owners: {}, classifications: {}, staleFailureManifests: [] },
      { runtimeSignals: {}, runtimeSignalOwners: {}, owners: {}, classifications: {}, staleFailureManifests: [] },
    ])
    assert.deepEqual(aggregate.reports.map((report) => report.matrixCoverage), [
      {
        runtimeSignals: {},
        runtimeSignalOwners: {},
        owners: {},
        classifications: {},
        staleMatrixReports: [],
        requiredMatrices: ["test-matrix"],
        missingMatrices: [],
        requiredMatrixClassifications: ["kernel-authority"],
        missingMatrixClassifications: ["kernel-authority"],
        requiredMatrixRuntimeSignals: [],
        missingMatrixRuntimeSignals: [],
        requiredDeploymentPresets: ["local"],
        missingDeploymentPresets: [],
        requiredProviders: ["codex"],
        missingProviders: [],
        requiredScenarios: ["local"],
        missingScenarios: [],
      },
      {
        runtimeSignals: {},
        runtimeSignalOwners: {},
        owners: {},
        classifications: {},
        staleMatrixReports: [],
        requiredMatrices: ["hosted-matrix", "test-matrix"],
        missingMatrices: ["hosted-matrix"],
        requiredMatrixClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
        missingMatrixClassifications: ["remote-extension-sync", "workspace-live-sync-conflict"],
        requiredMatrixRuntimeSignals: [],
        missingMatrixRuntimeSignals: [],
        requiredDeploymentPresets: ["hosted-cloud", "local"],
        missingDeploymentPresets: ["hosted-cloud"],
        requiredProviders: ["claude", "codex"],
        missingProviders: ["claude"],
        requiredScenarios: ["remote"],
        missingScenarios: ["remote"],
      },
    ])
    const text = formatDrillValidationGateAggregateSummary(aggregate)
    assert.match(text, /coverage:/)
    assert.match(text, /missing_platform_coverage_areas: hosted-cloud-drills=1 runtime-fixtures=1/)
    assert.match(text, /required_failure_classifications: kernel-authority=1 remote-extension-sync=1 workspace-live-sync-conflict=1/)
    assert.match(text, /missing_failure_classifications: kernel-authority=1 remote-extension-sync=1 workspace-live-sync-conflict=1/)
    assert.match(text, /missing_matrices: hosted-matrix=1/)
    assert.match(text, /missing_matrix_classifications: kernel-authority=1 remote-extension-sync=1 workspace-live-sync-conflict=1/)
    assert.match(text, /required_deployment_presets: hosted-cloud=1 local=2/)
    assert.match(text, /missing_providers: claude=1/)
    assert.match(text, /missing_scenarios: remote=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("reads and discovers validation gate aggregate artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const aggregatePath = path.join(rootDir, "reports", "aggregate.json")
    const aggregate = summarizeDrillValidationGateReports([await runDrillValidationGate({
      failureRoots: ["/tmp/no-such-arroba-failure-root"],
    })])
    await mkdir(path.dirname(aggregatePath), { recursive: true })
    await writeFile(aggregatePath, `${JSON.stringify(aggregate, null, 2)}\n`, "utf8")
    await writeFile(path.join(rootDir, "reports", "gate.json"), `${JSON.stringify(await runDrillValidationGate(), null, 2)}\n`, "utf8")

    assert.deepEqual(await findDrillValidationGateAggregatePaths([rootDir]), [aggregatePath])
    assert.deepEqual(await readDrillValidationGateAggregate(aggregatePath), aggregate)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects inconsistent validation gate aggregates", async () => {
  const aggregate = summarizeDrillValidationGateReports([await runDrillValidationGate()])

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      totals: {
        ...aggregate.totals,
        failed: 0,
      },
    }),
    /totals do not match reports/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      coverage: {
        ...aggregate.coverage,
        requiredProviders: { codex: 2 },
      },
    }),
    /coverage does not match reports/,
  )
  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      status: "passed",
      requiredPresets: ["workspace-live-sync"],
      missingPresets: [],
    }),
    /missingPresets does not match reports/,
  )
})

