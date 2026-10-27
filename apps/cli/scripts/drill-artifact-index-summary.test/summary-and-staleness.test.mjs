import {
  assert,
  execFile,
  mkdtemp,
  os,
  path,
  readFile,
  rm,
  scriptPath,
  test,
  verifyDrillArtifactIndex,
  writeFile,
  rewriteDrillArtifactIndexCreatedAt,
  rewriteDrillMatrixReportCompletedAt,
  writeIndexedReport,
} from '../drill-artifact-index-summary.test-support.mjs'

test("drill artifact index summary aggregates discovered indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "chariox-drill-artifacts.json")
  try {
    const firstIndexPath = await writeIndexedReport(rootDir, "one", "chariox.drill.validation_gate.v1")
    const secondIndexPath = await writeIndexedReport(rootDir, "two", "chariox.drill.matrix.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-root",
      rootDir,
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const stdoutAggregate = JSON.parse(stdout)
    const fileAggregate = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileAggregate, stdoutAggregate)
    assert.equal(stdoutAggregate.schema, "chariox.drill.artifact_index.aggregate.v1")
    assert.equal(stdoutAggregate.totals.indexes, 2)
    assert.equal(stdoutAggregate.totals.artifacts, 2)
    assert(stdoutAggregate.totals.sizeBytes > 0)
    assert.deepEqual(stdoutAggregate.runtimeSignals, {
      "lease-health": 1,
      "session-authority": 2,
      "workspace-live-sync-state": 1,
    })
    assert.deepEqual(stdoutAggregate.runtimeSignalOwners, {
      "kernel-authority": 2,
      "runtime-state": 1,
    })
    assert.deepEqual(stdoutAggregate.runtimeAuthorityInvariants, {
      "client-render-request": 1,
      "home-session-authority": 1,
    })
    assert.deepEqual(stdoutAggregate.validationPresets, {
      "distributed-runtime": 1,
      "workspace-live-sync": 1,
    })
    assert.deepEqual(stdoutAggregate.requiredFailureClassifications, {
      "kernel-authority": 1,
      "workspace-live-sync-conflict": 1,
    })
    assert.deepEqual(stdoutAggregate.artifactKinds, {
      "artifact-index": 1,
      "matrix-report": 1,
      "validation-gate": 1,
    })
    assert.deepEqual(stdoutAggregate.evidenceRepos, {
      cloud: 1,
      oss: 2,
    })
    assert.deepEqual(stdoutAggregate.providerAccountAliases, {
      "codex=work": 1,
      "opencode=zen": 1,
    })
    assert.deepEqual(stdoutAggregate.artifactCoverageInputSources, {
      "artifact metadata inputs": 1,
    })
    assert.deepEqual(stdoutAggregate.exitCriterionStatuses, {
      "dry-run": 1,
    })
    assert.deepEqual(stdoutAggregate.incompleteExitCriterionStatuses, {
      "dry-run": 1,
    })
    assert.deepEqual(stdoutAggregate.plannedOwners, {
      "validation-harness": 1,
    })
    assert.deepEqual(stdoutAggregate.plannedClassifications, {
      "matrix-coverage": 1,
    })
    assert.deepEqual(stdoutAggregate.generatedEvidenceKinds, {
      "matrix-report": 1,
      "validation-suite-run": 1,
    })
    assert.deepEqual(stdoutAggregate.generatedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(stdoutAggregate.generatedMatrixArtifactIndexes, {
      "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.generatedMatrixNames, {
      "workspace-live-sync-matrix": 1,
    })
    assert.deepEqual(stdoutAggregate.generatedMatrixRepos, {
      oss: 1,
    })
    assert.deepEqual(stdoutAggregate.generatedValidationSuiteArtifactIndexes, {
      "/tmp/generated-suite/chariox-drill-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.generatedValidationSuiteFailureRoots, {
      "/tmp/generated-suite/failed-run": 1,
    })
    assert.deepEqual(stdoutAggregate.requiredGeneratedEvidenceKinds, {
      "matrix-report": 2,
      "validation-suite-run": 1,
    })
    assert.deepEqual(stdoutAggregate.missingGeneratedEvidenceKinds, {
      "matrix-report": 1,
    })
    assert.deepEqual(stdoutAggregate.requiredGeneratedMatrixArtifactIndexes, {
      "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.missingGeneratedMatrixArtifactIndexes, {
      "/tmp/generated-matrix/missing-matrix-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.requiredGeneratedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(stdoutAggregate.missingGeneratedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(stdoutAggregate.requiredGeneratedValidationSuiteArtifactIndexes, {
      "/tmp/generated-suite/chariox-drill-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.missingGeneratedValidationSuiteArtifactIndexes, {
      "/tmp/generated-suite/missing-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.requiredGeneratedValidationSuiteFailureRoots, {
      "/tmp/generated-suite/failed-run": 1,
    })
    assert.deepEqual(stdoutAggregate.missingGeneratedValidationSuiteFailureRoots, {
      "/tmp/generated-suite/missing-run": 1,
    })
    assert.deepEqual(stdoutAggregate.indexes.map((index) => index.source), [
      firstIndexPath,
      secondIndexPath,
    ])
    assert.equal(artifactIndex.metadata.drill, "artifact-index-summary")
    assert.equal(artifactIndex.metadata.indexes, 2)
    assert.equal(artifactIndex.metadata.runtimeSignals, "lease-health,session-authority,workspace-live-sync-state")
    assert.equal(artifactIndex.metadata.runtimeSignalOwners, "kernel-authority,runtime-state")
    assert.equal(artifactIndex.metadata.validationPresets, "distributed-runtime,workspace-live-sync")
    assert.equal(artifactIndex.metadata.owners, "runtime-network,validation-harness")
    assert.equal(artifactIndex.metadata.classifications, "matrix-coverage,validation-gate")
    assert.equal(artifactIndex.metadata.plannedOwners, "validation-harness")
    assert.equal(artifactIndex.metadata.plannedClassifications, "matrix-coverage")
    assert.equal(artifactIndex.metadata.exitCriterionStatuses, "dry-run")
    assert.equal(artifactIndex.metadata.incompleteExitCriterionStatuses, "dry-run")
    assert.equal(artifactIndex.metadata.artifactKinds, "artifact-index,artifact-index-aggregate,matrix-report,validation-gate")
    assert.equal(artifactIndex.metadata.generatedEvidenceKinds, "matrix-report,validation-suite-run")
    assert.equal(artifactIndex.metadata.generatedMatrixArtifactIndexes, "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json")
    assert.equal(artifactIndex.metadata.generatedMatrixLimitations, "dry-run-classification-coverage")
    assert.equal(artifactIndex.metadata.generatedMatrixNames, "workspace-live-sync-matrix")
    assert.equal(artifactIndex.metadata.generatedMatrixRepos, "oss")
    assert.equal(artifactIndex.metadata.generatedValidationSuiteArtifactIndexes, "/tmp/generated-suite/chariox-drill-artifacts.json")
    assert.equal(artifactIndex.metadata.generatedValidationSuiteFailureRoots, "/tmp/generated-suite/failed-run")
    assert.equal(artifactIndex.metadata.requiredGeneratedEvidenceKinds, "matrix-report,validation-suite-run")
    assert.equal(artifactIndex.metadata.missingGeneratedEvidenceKinds, "matrix-report")
    assert.equal(artifactIndex.metadata.requiredGeneratedMatrixArtifactIndexes, "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json")
    assert.equal(artifactIndex.metadata.missingGeneratedMatrixArtifactIndexes, "/tmp/generated-matrix/missing-matrix-artifacts.json")
    assert.equal(artifactIndex.metadata.requiredGeneratedMatrixLimitations, "dry-run-classification-coverage")
    assert.equal(artifactIndex.metadata.missingGeneratedMatrixLimitations, "dry-run-classification-coverage")
    assert.equal(artifactIndex.metadata.requiredGeneratedValidationSuiteArtifactIndexes, "/tmp/generated-suite/chariox-drill-artifacts.json")
    assert.equal(artifactIndex.metadata.missingGeneratedValidationSuiteArtifactIndexes, "/tmp/generated-suite/missing-artifacts.json")
    assert.equal(artifactIndex.metadata.requiredGeneratedValidationSuiteFailureRoots, "/tmp/generated-suite/failed-run")
    assert.equal(artifactIndex.metadata.missingGeneratedValidationSuiteFailureRoots, "/tmp/generated-suite/missing-run")
    assert.equal(artifactIndex.metadata.evidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.providerAccountAliases, "codex=work,opencode=zen")
    assert.equal(artifactIndex.metadata.artifactCoverageInputCount, "1")
    assert.equal(artifactIndex.metadata.artifactCoverageInputSources, "artifact metadata inputs")
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "aggregate.json",
      schema: "chariox.drill.artifact_index.aggregate.v1",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/chariox-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

test("drill artifact index summary prints artifact coverage input count", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  try {
    await writeIndexedReport(rootDir, "one", "chariox.drill.validation_gate.v1")
    await writeIndexedReport(rootDir, "two", "chariox.drill.matrix.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-root",
      rootDir,
    ])

    assert.match(stdout, /artifact_coverage_input_sources: artifact metadata inputs=1/)
    assert.match(stdout, /artifact_coverage_input_count=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates stale indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "chariox.drill.validation_gate.v1")
    await rewriteDrillArtifactIndexCreatedAt(indexPath, new Date(Date.now() - 500).toISOString())

    const fresh = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-artifact-max-age-ms",
      "3600000",
      "--json",
    ])).stdout)
    assert.equal(fresh.requiredArtifactMaxAgeMs, 3_600_000)
    assert.deepEqual(fresh.staleArtifactIndexes, [])

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-artifact-max-age-ms=100",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /artifact_required_max_age_ms=100 stale_indexes=1/)
        assert.match(error.stdout, /next: regenerate stale drill artifact indexes/)
        assert.match(error.stdout, /sources: artifact-index report=.*chariox-drill-artifacts\.json/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-artifact-max-age-ms=100",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const aggregate = JSON.parse(error.stdout)
        assert.deepEqual(aggregate.nextActions.map(({ classification, count, sourceDetails }) => ({
          classification,
          count,
          sourceDetails,
        })), [{
          classification: "artifact-staleness",
          count: 1,
          sourceDetails: [{
            source: "artifact-index",
            reportPath: indexPath,
          }],
        }])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates stale matrix reports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  try {
    const indexPath = await writeIndexedReport(rootDir, "two", "chariox.drill.matrix.v1")
    await rewriteDrillMatrixReportCompletedAt(indexPath, new Date(Date.now() - 500).toISOString())

    const fresh = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-matrix-max-age-ms",
      "3600000",
      "--json",
    ])).stdout)
    assert.equal(fresh.requiredMatrixMaxAgeMs, 3_600_000)
    assert.deepEqual(fresh.staleMatrixReports, [])

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-matrix-max-age-ms=100",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /matrix_required_max_age_ms=100 stale_reports=1/)
        assert.match(error.stdout, /next: regenerate stale drill matrix reports/)
        assert.match(error.stdout, /sources: artifact-index-summary-matrix report=.*reports\/report\.json/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-matrix-max-age-ms=100",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const aggregate = JSON.parse(error.stdout)
        assert.deepEqual(aggregate.nextActions.map(({ classification, count, sourceDetails }) => ({
          classification,
          count,
          sourceDetails,
        })), [{
          classification: "matrix-staleness",
          count: 1,
          sourceDetails: [{
            source: "artifact-index-summary-matrix",
            matrix: "artifact-index-summary-matrix",
            reportPath: path.join(rootDir, "two", "reports", "report.json"),
          }],
        }])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})
