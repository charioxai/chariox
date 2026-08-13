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

test("passes with explicit matrix report paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport())

    const report = await runDrillValidationGate({
      matrixReports: [reportPath],
      requireComplete: true,
    })

    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.matrices.inputs, [reportPath])
    assert.deepEqual(report.checks.matrices.reportPaths, [reportPath])
    assert.match(formatDrillValidationGateSummary(report), /matrices=passed roots=0 inputs=1 reports=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates matrix reports by required freshness", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    const completedAt = new Date(Date.now() - 500).toISOString()
    const startedAt = new Date(Date.parse(completedAt) - 1000).toISOString()
    await writeMatrixReport(reportPath, matrixReport({
      startedAt,
      completedAt,
      durationMs: 1000,
    }))

    const pass = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredMatrixMaxAgeMs: 3_600_000,
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.matrices.staleMatrixReports, [])
    assert.match(formatDrillValidationGateSummary(pass), /matrix_required_max_age_ms=3600000 stale_reports=0/)

    const fail = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredMatrixMaxAgeMs: 100,
    })
    assert.equal(fail.status, "failed")
    assert.equal(fail.checks.matrices.staleMatrixReports.length, 1)
    assert.equal(fail.checks.matrices.staleMatrixReports[0].source, reportPath)
    const text = formatDrillValidationGateSummary(fail)
    assert.match(text, /matrix_required_max_age_ms=100 stale_reports=1/)
    assert.match(text, /sources: test-matrix report=.*matrix\.json/)
    assert.deepEqual(
      fail.nextActions
        .filter(({ classification }) => classification === "matrix-staleness")
        .map(({ owner, classification, nextAction, count, sourceDetails }) => ({ owner, classification, nextAction, count, sourceDetails })),
      [{
        owner: "validation-harness",
        classification: "matrix-staleness",
        nextAction: "regenerate stale matrix reports, then rerun the validation gate",
        count: 1,
        sourceDetails: [{
          source: "test-matrix",
          matrix: "test-matrix",
          reportPath,
        }],
      }],
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates matrix reports by required matrix name coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const localReport = path.join(rootDir, "local.json")
    const remoteReport = path.join(rootDir, "remote.json")
    await writeMatrixReport(localReport, matrixReport({ matrix: "local-runtime" }))
    await writeMatrixReport(remoteReport, matrixReport({ matrix: "remote-runtime" }))

    const pass = await runDrillValidationGate({
      matrixReports: [localReport, remoteReport],
      requiredMatrices: ["local-runtime,remote-runtime"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.matrices.requiredMatrices, ["local-runtime", "remote-runtime"])
    assert.deepEqual(pass.checks.matrices.missingMatrices, [])
    assert.match(formatDrillValidationGateSummary(pass), /matrix_required_names=local-runtime,remote-runtime missing=none/)

    const fail = await runDrillValidationGate({
      matrixReports: [localReport],
      requiredMatrices: ["hosted-cloud", "local-runtime"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.matrices.missingMatrices, ["hosted-cloud"])
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run missing drill matrices: hosted-cloud",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates matrix reports by required classification coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      scenarios: [
        scenario("kernel-authority", "passed", { classification: "kernel-authority" }),
        scenario("relay-freshness", "passed", { classification: "relay-target-freshness" }),
      ],
    }))

    const pass = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredMatrixClassifications: ["kernel-authority,relay-target-freshness"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.matrices.requiredMatrixClassifications, ["kernel-authority", "relay-target-freshness"])
    assert.deepEqual(pass.checks.matrices.missingMatrixClassifications, [])
    assert.match(formatDrillValidationGateSummary(pass), /matrix_required_classifications=kernel-authority,relay-target-freshness missing=none/)

    const fail = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredMatrixClassifications: ["kernel-authority", "remote-extension-sync"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.matrices.missingMatrixClassifications, ["remote-extension-sync"])
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports covering failure classifications: remote-extension-sync",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects unknown required matrix classifications", async () => {
  await assert.rejects(
    () => runDrillValidationGate({
      requiredMatrixClassifications: ["kernel-authority", "remote-extension-synch"],
    }),
    /unknown required matrix classification: remote-extension-synch/,
  )
})

test("passes when matrix reports cover required deployment presets", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      metadata: { deploymentPresets: "hosted-cloud,local,self-hosted-relay" },
    }))

    const report = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredDeploymentPresets: ["self-hosted-relay,local", "hosted-cloud"],
    })

    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.matrices.requiredDeploymentPresets, ["hosted-cloud", "local", "self-hosted-relay"])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.match(formatDrillValidationGateSummary(report), /matrix_required_deployment_presets=hosted-cloud,local,self-hosted-relay missing=none/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when matrix reports miss required deployment presets", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      metadata: { deploymentPresets: "local,self-hosted-relay" },
    }))

    const report = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredDeploymentPresets: ["local", "hosted-cloud", "hetzner"],
    })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.matrices.status, "failed")
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, ["hetzner", "hosted-cloud"])
    assert.deepEqual(report.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports for missing deployment presets: hetzner, hosted-cloud",
    }])
    assert.match(formatDrillValidationGateSummary(report), /missing=hetzner,hosted-cloud/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates matrix reports by required provider coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      metadata: { providers: "codex,opencode" },
    }))

    const pass = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredProviders: ["codex,opencode"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.matrices.requiredProviders, ["codex", "opencode"])
    assert.deepEqual(pass.checks.matrices.missingProviders, [])
    assert.match(formatDrillValidationGateSummary(pass), /matrix_required_providers=codex,opencode missing=none/)

    const fail = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredProviders: ["claude", "codex"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.matrices.missingProviders, ["claude"])
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports for missing providers: claude",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates matrix reports by required scenario coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport({
      scenarios: [
        scenario("local-single-user", "passed"),
        scenario("remote-collab", "passed"),
      ],
    }))

    const pass = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredScenarios: ["local-single-user,remote-collab"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.matrices.requiredScenarios, ["local-single-user", "remote-collab"])
    assert.deepEqual(pass.checks.matrices.missingScenarios, [])
    assert.match(formatDrillValidationGateSummary(pass), /matrix_required_scenarios=local-single-user,remote-collab missing=none/)

    const fail = await runDrillValidationGate({
      matrixReports: [reportPath],
      requiredScenarios: ["hetzner-collab", "local-single-user"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.matrices.missingScenarios, ["hetzner-collab"])
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [{
      owner: "validation-harness",
      classification: "matrix-coverage",
      nextAction: "run matrix reports for missing scenarios: hetzner-collab",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects unknown required deployment presets", async () => {
  await assert.rejects(
    () => runDrillValidationGate({
      requiredDeploymentPresets: ["local", "hosted-clouds"],
    }),
    /unknown required deployment preset: hosted-clouds/,
  )
})

test("fails when no validation checks are configured", async () => {
  const report = await runDrillValidationGate()

  assert.equal(report.status, "failed")
  assert.equal(report.checks.configuration.error, "no validation checks configured")
  assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
    { owner: "validation-harness", classification: "validation-gate" },
  ])
  assert.match(formatDrillValidationGateSummary(report), /configuration=failed/)
})

test("fails when configured matrix roots contain no reports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const report = await runDrillValidationGate({ matrixRoots: [rootDir] })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.matrices.error, "no matrix reports found")
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "validation-harness", classification: "matrix-artifacts" },
    ])
    assert.equal(drillValidationGateExitCode(report), 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when require-complete sees dry-run matrix scenarios", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    await writeMatrixReport(path.join(rootDir, "matrix.json"), matrixReport({
      status: "dry-run",
      dryRun: true,
      scenarios: [scenario("remote", "dry-run")],
    }))

    const report = await runDrillValidationGate({
      matrixRoots: [rootDir],
      requireComplete: true,
    })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.matrices.aggregate.status, "dry-run")
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "validation-harness", classification: "incomplete-matrix" },
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

