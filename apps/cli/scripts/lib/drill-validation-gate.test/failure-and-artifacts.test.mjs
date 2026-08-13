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

test("fails when preserved failure manifests are found", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    await writeFailureManifest(path.join(rootDir, "failed", "chariox-drill-failure.json"))

    const report = await runDrillValidationGate({ failureRoots: [rootDir] })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.failures.aggregate.total, 1)
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "provider-account", classification: "provider-auth" },
    ])
    assert.match(formatDrillValidationGateSummary(report), /failure_total=1/)
    assert.match(formatDrillValidationGateSummary(report), /next actions:/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails with explicit failure manifest paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const manifestPath = path.join(rootDir, "chariox-drill-failure.json")
    await writeFailureManifest(manifestPath)

    const report = await runDrillValidationGate({ failureInputs: [manifestPath] })

    assert.equal(report.status, "failed")
    assert.deepEqual(report.checks.failures.inputs, [manifestPath])
    assert.deepEqual(report.checks.failures.manifestPaths, [manifestPath])
    assert.match(formatDrillValidationGateSummary(report), /failures=failed roots=0 inputs=1 manifests=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("reports stale preserved failure manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const manifestPath = path.join(rootDir, "chariox-drill-failure.json")
    await writeFailureManifest(manifestPath, {
      drill: "stale-failure",
      failedAt: new Date(Date.now() - 500).toISOString(),
    })

    const report = await runDrillValidationGate({
      failureInputs: [manifestPath],
      requiredFailureMaxAgeMs: 100,
    })
    const summary = formatDrillValidationGateSummary(report)

    assert.equal(report.status, "failed")
    assert.equal(report.checks.failures.requiredFailureMaxAgeMs, 100)
    assert.equal(report.checks.failures.staleFailureManifests.length, 1)
    assert.equal(report.checks.failures.staleFailureManifests[0].source, manifestPath)
    assert.match(summary, /failure_required_max_age_ms=100 stale_manifests=1/)
    assert.match(summary, /stale_failure_manifest=.*chariox-drill-failure\.json drill=stale-failure/)
    assert.match(summary, /sources: stale-failure report=.*chariox-drill-failure\.json/)
    assert.deepEqual(
      report.nextActions
        .filter(({ owner, classification }) => owner === "validation-harness" && classification === "failure-artifacts")
        .map(({ nextAction, count, sourceDetails }) => ({ nextAction, count, sourceDetails })),
      [{
        nextAction: "regenerate stale preserved failure bundles or rerun the failing drills before routing them",
        count: 1,
        sourceDetails: [{
          source: "stale-failure",
          reportPath: manifestPath,
        }],
      }],
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("passes with explicit artifact index paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, "{\"schema\":\"chariox.drill.validation_gate.v1\"}\n", "utf8")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
    })
    const indexPath = path.join(rootDir, "chariox-drill-artifacts.json")

    const report = await runDrillValidationGate({ artifactIndexes: [indexPath] })

    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.inputs, [indexPath])
    assert.deepEqual(report.checks.artifacts.indexPaths, [indexPath])
    assert.equal(report.checks.artifacts.aggregate.totals.artifacts, 1)
    assert.match(formatDrillValidationGateSummary(report), /artifacts=passed roots=0 inputs=1 indexes=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates explicit artifact index paths by required freshness", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, "{\"schema\":\"chariox.drill.validation_gate.v1\"}\n", "utf8")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
    })
    const indexPath = path.join(rootDir, "chariox-drill-artifacts.json")
    await rewriteArtifactIndexCreatedAt(indexPath, new Date(Date.now() - 500).toISOString())

    const fresh = await runDrillValidationGate({
      artifactIndexes: [indexPath],
      requiredArtifactMaxAgeMs: 3_600_000,
    })
    assert.equal(fresh.status, "passed")
    assert.equal(fresh.checks.artifacts.requiredArtifactMaxAgeMs, 3_600_000)
    assert.deepEqual(fresh.checks.artifacts.staleArtifactIndexes, [])
    assert.match(formatDrillValidationGateSummary(fresh), /artifact_required_max_age_ms=3600000 stale_indexes=0/)

    const stale = await runDrillValidationGate({
      artifactIndexes: [indexPath],
      requiredArtifactMaxAgeMs: 100,
    })
    assert.equal(stale.status, "failed")
    assert.equal(stale.checks.artifacts.staleArtifactIndexes.length, 1)
    assert.equal(stale.checks.artifacts.staleArtifactIndexes[0].source, indexPath)
    assert.match(stale.checks.artifacts.error, /stale artifact indexes:/)
    const text = formatDrillValidationGateSummary(stale)
    assert.match(text, /artifact_required_max_age_ms=100 stale_indexes=1/)
    assert.match(text, /stale_artifact_index=.*chariox-drill-artifacts\.json/)
    assert.match(text, /sources: artifact-index report=.*chariox-drill-artifacts\.json/)
    assert.deepEqual(
      stale.nextActions
        .filter(({ classification }) => classification === "artifact-staleness")
        .map(({ owner, classification, nextAction, count, sourceDetails }) => ({ owner, classification, nextAction, count, sourceDetails })),
      [{
        owner: "validation-harness",
        classification: "artifact-staleness",
        nextAction: "regenerate stale drill artifact indexes, then rerun the validation gate",
        count: 1,
        sourceDetails: [{
          source: "artifact-index",
          reportPath: indexPath,
        }],
      }],
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates explicit artifact index paths by required schema", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, "{\"schema\":\"chariox.drill.validation_gate.v1\"}\n", "utf8")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
    })
    const indexPath = path.join(rootDir, "chariox-drill-artifacts.json")

    const pass = await runDrillValidationGate({
      artifactIndexes: [indexPath],
      requiredArtifactSchemas: ["chariox.drill.validation_gate.v1"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.artifacts.missingArtifactSchemas, [])

    const fail = await runDrillValidationGate({
      artifactIndexes: [indexPath],
      requiredArtifactSchemas: ["chariox.drill.validation_suite_run.v1"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.artifacts.requiredArtifactSchemas, ["chariox.drill.validation_suite_run.v1"])
    assert.deepEqual(fail.checks.artifacts.missingArtifactSchemas, ["chariox.drill.validation_suite_run.v1"])
    assert.match(formatDrillValidationGateSummary(fail), /artifact_required_schemas=chariox\.drill\.validation_suite_run\.v1 missing=chariox\.drill\.validation_suite_run\.v1/)
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [
      {
        owner: "validation-harness",
        classification: "artifact-coverage",
        nextAction: "run an executable validation suite with --run-json --output PATH --output-artifact-index PATH, then rerun the validation gate",
      },
      {
        owner: "validation-harness",
        classification: "artifact-index",
        nextAction: "fix missing, unreadable, or tampered artifact indexes before using collected drill evidence",
      },
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates explicit artifact index paths by required diagnostic metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, "{\"schema\":\"chariox.drill.validation_gate.v1\"}\n", "utf8")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
      metadata: {
        classifications: "validation-gate",
        owners: "validation-platform",
        providerAccountAliases: "codex=work",
        runtimeSignals: "session-authority,workspace-live-sync-state",
        runtimeSignalOwners: "kernel-authority,runtime-state",
      },
    })
    const indexPath = path.join(rootDir, "chariox-drill-artifacts.json")

    const pass = await runDrillValidationGate({
      artifactIndexes: [indexPath],
      requiredArtifactRuntimeSignals: ["session-authority"],
      requiredArtifactRuntimeSignalOwners: ["kernel-authority,runtime-state"],
      requiredArtifactProviderAccountAliases: ["codex=work"],
      requiredArtifactOwners: ["validation-platform"],
      requiredArtifactClassifications: ["validation-gate"],
    })
    assert.equal(pass.status, "passed")
    assert.match(formatDrillValidationGateSummary(pass), /artifact_required_runtime_signals=session-authority missing=none/)
    assert.match(formatDrillValidationGateSummary(pass), /artifact_required_runtime_signal_owners=kernel-authority,runtime-state missing=none/)
    assert.match(formatDrillValidationGateSummary(pass), /artifact_required_provider_account_aliases=codex=work missing=none/)
    assert.match(formatDrillValidationGateSummary(pass), /artifact_required_owners=validation-platform missing=none/)
    assert.match(formatDrillValidationGateSummary(pass), /artifact_required_classifications=validation-gate missing=none/)

    const fail = await runDrillValidationGate({
      artifactIndexes: [indexPath],
      requiredArtifactRuntimeSignals: ["lease-health"],
      requiredArtifactRuntimeSignalOwners: ["worker-kernel"],
      requiredArtifactProviderAccountAliases: ["opencode=zen"],
      requiredArtifactOwners: ["runtime-network"],
      requiredArtifactClassifications: ["cloud-validation-suite"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.artifacts.missingArtifactRuntimeSignals, ["lease-health"])
    assert.deepEqual(fail.checks.artifacts.missingArtifactRuntimeSignalOwners, ["worker-kernel"])
    assert.deepEqual(fail.checks.artifacts.missingArtifactProviderAccountAliases, ["opencode=zen"])
    assert.deepEqual(fail.checks.artifacts.missingArtifactOwners, ["runtime-network"])
    assert.deepEqual(fail.checks.artifacts.missingArtifactClassifications, ["cloud-validation-suite"])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when configured artifact roots contain no indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const report = await runDrillValidationGate({ artifactRoots: [rootDir] })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.artifacts.error, "no artifact indexes found")
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "validation-harness", classification: "artifact-index" },
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when artifact indexes point at tampered artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, "{\"schema\":\"chariox.drill.validation_gate.v1\"}\n", "utf8")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
    })
    await writeFile(reportPath, "{\"schema\":\"tampered\"}\n", "utf8")

    const report = await runDrillValidationGate({
      artifactIndexes: [path.join(rootDir, "chariox-drill-artifacts.json")],
    })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.artifacts.status, "failed")
    assert.match(report.checks.artifacts.error, /sha256 mismatch/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects validation gate reports with mismatched top-level status", async () => {
  const report = await runDrillValidationGate()

  assert.throws(
    () => validateDrillValidationGateReport({ ...report, status: "passed" }),
    /status does not match check statuses/,
  )
})

test("rejects malformed platform bundle artifact evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    const report = await runDrillValidationGate({ platformBundleDir: bundleDir })
    const malformed = {
      ...report,
      checks: {
        ...report.checks,
        platformBundle: {
          ...report.checks.platformBundle,
          artifacts: [{
            ...report.checks.platformBundle.artifacts[0],
            sha256: "not-a-sha",
          }],
        },
      },
    }

    assert.throws(
      () => formatDrillValidationGateSummary(malformed),
      /artifacts\[0\] has invalid sha256/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects inconsistent platform bundle validation suite evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    const report = await runDrillValidationGate({ platformBundleDir: bundleDir })
    const malformed = {
      ...report,
      checks: {
        ...report.checks,
        platformBundle: {
          ...report.checks.platformBundle,
          validationSuite: {
            ...report.checks.platformBundle.validationSuite,
            coverageAreas: report.checks.platformBundle.validationSuite.coverageAreas.slice(1),
          },
        },
      },
    }

    assert.throws(
      () => formatDrillValidationGateSummary(malformed),
      /coverageAreas do not match testCount/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("resolves explicit failure root inputs to manifest paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-validation-gate-"))
  try {
    const failureRoot = path.join(rootDir, "failed")
    const manifestPath = path.join(failureRoot, "chariox-drill-failure.json")
    await writeFailureManifest(manifestPath)

    const report = await runDrillValidationGate({ failureInputs: [failureRoot] })

    assert.equal(report.status, "failed")
    assert.deepEqual(report.checks.failures.inputs, [failureRoot])
    assert.deepEqual(report.checks.failures.manifestPaths, [manifestPath])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

