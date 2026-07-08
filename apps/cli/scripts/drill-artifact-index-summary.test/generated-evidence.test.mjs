import {
  assert,
  execFile,
  mkdtemp,
  os,
  path,
  rm,
  scriptPath,
  test,
  verifyDrillArtifactIndex,
  writeFile,
  rewriteDrillArtifactIndexCreatedAt,
  rewriteDrillMatrixReportCompletedAt,
  writeIndexedReport,
} from '../drill-artifact-index-summary.test-support.mjs'

test("drill artifact index summary gates generated validation-suite failure roots", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-generated-validation-suite-failure-root",
      "/tmp/generated-suite/failed-run",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredGeneratedValidationSuiteFailureRootRequirements, ["/tmp/generated-suite/failed-run"])
    assert.deepEqual(aggregate.missingGeneratedValidationSuiteFailureRootRequirements, [])
    assert.equal(artifactIndex.metadata.requiredGeneratedValidationSuiteFailureRoots, "/tmp/generated-suite/failed-run")
    assert.equal(artifactIndex.metadata.missingGeneratedValidationSuiteFailureRoots, "/tmp/generated-suite/missing-run")

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-validation-suite-failure-root=/tmp/generated-suite/missing-run",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /generated_validation_suite_failure_roots_required=\/tmp\/generated-suite\/missing-run missing=\/tmp\/generated-suite\/missing-run/)
        assert.match(error.stdout, /next: rerun generated validation suites with --preserve-failure-root .*\/tmp\/generated-suite\/missing-run/)
        assert.match(error.stdout, /sources: \/tmp\/generated-suite\/missing-run/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-validation-suite-failure-root=/tmp/generated-suite/missing-run",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const missing = JSON.parse(error.stdout)
        assert.deepEqual(missing.nextActions.map(({ classification, sourceDetails }) => ({ classification, sourceDetails })), [{
          classification: "generated-evidence",
          sourceDetails: [{ source: "/tmp/generated-suite/missing-run" }],
        }])
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-validation-suite-failure-root",
        "/tmp/generated-suite/Bearer abcdefghijklmnop",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-validation-suite-failure-root includes secret-looking diagnostic text/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates generated validation-suite artifact indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-generated-validation-suite-artifact-index",
      "/tmp/generated-suite/arroba-drill-artifacts.json",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredGeneratedValidationSuiteArtifactIndexPaths, ["/tmp/generated-suite/arroba-drill-artifacts.json"])
    assert.deepEqual(aggregate.missingGeneratedValidationSuiteArtifactIndexPaths, [])
    assert.equal(artifactIndex.metadata.requiredGeneratedValidationSuiteArtifactIndexes, "/tmp/generated-suite/arroba-drill-artifacts.json")
    assert.equal(artifactIndex.metadata.missingGeneratedValidationSuiteArtifactIndexes, "/tmp/generated-suite/missing-artifacts.json")

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-validation-suite-artifact-index=/tmp/generated-suite/missing-artifacts.json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /generated_validation_suite_artifact_indexes_required=\/tmp\/generated-suite\/missing-artifacts\.json missing=\/tmp\/generated-suite\/missing-artifacts\.json/)
        assert.match(error.stdout, /next: rerun generated validation suites with artifact indexes .*\/tmp\/generated-suite\/missing-artifacts\.json/)
        assert.match(error.stdout, /sources: \/tmp\/generated-suite\/missing-artifacts\.json/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-validation-suite-artifact-index=/tmp/generated-suite/missing-artifacts.json",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const missing = JSON.parse(error.stdout)
        assert.deepEqual(missing.nextActions.map(({ classification, sourceDetails }) => ({ classification, sourceDetails })), [{
          classification: "generated-evidence",
          sourceDetails: [{ source: "/tmp/generated-suite/missing-artifacts.json" }],
        }])
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-validation-suite-artifact-index",
        "/tmp/generated-suite/Bearer abcdefghijklmnop.json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-validation-suite-artifact-index includes secret-looking diagnostic text/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates generated evidence kinds", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-generated-evidence-kind",
      "validation-suite-run",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredGeneratedEvidenceKindRequirements, ["validation-suite-run"])
    assert.deepEqual(aggregate.missingRequiredGeneratedEvidenceKinds, [])
    assert.equal(artifactIndex.metadata.requiredGeneratedEvidenceKindRequirements, "validation-suite-run")
    assert.equal(artifactIndex.metadata.missingRequiredGeneratedEvidenceKinds, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-evidence-kind=matrix-report",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /generated_evidence_kinds_required=matrix-report missing=matrix-report/)
        assert.match(error.stdout, /next: include drill artifact indexes that record generated evidence kinds: matrix-report/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-evidence-kind",
        "matrix-reprot",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-evidence-kind has unknown generated evidence kind: matrix-reprot/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates generated matrix limitations", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const matrixIndexPath = await writeIndexedReport(rootDir, "two", "arroba.drill.matrix.v1")
    const validationIndexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      matrixIndexPath,
      "--require-generated-matrix-limitation",
      "dry-run-classification-coverage",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredGeneratedMatrixLimitationRequirements, ["dry-run-classification-coverage"])
    assert.deepEqual(aggregate.missingRequiredGeneratedMatrixLimitations, [])
    assert.equal(artifactIndex.metadata.requiredGeneratedMatrixLimitations, "dry-run-classification-coverage")
    assert.equal(artifactIndex.metadata.missingGeneratedMatrixLimitations, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        validationIndexPath,
        "--require-generated-matrix-limitation=dry-run-classification-coverage",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /generated_matrix_limitations_required=dry-run-classification-coverage missing=dry-run-classification-coverage/)
        assert.match(error.stdout, /next: include drill artifact indexes that record generated matrix limitations: dry-run-classification-coverage/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        matrixIndexPath,
        "--require-generated-matrix-limitation",
        "dry-run-classification-covergae",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-matrix-limitation has unknown generated matrix limitation: dry-run-classification-covergae/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates generated matrix names and repos", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const matrixIndexPath = await writeIndexedReport(rootDir, "two", "arroba.drill.matrix.v1")
    const validationIndexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      matrixIndexPath,
      "--require-generated-matrix-name",
      "workspace-live-sync-matrix",
      "--require-generated-matrix-repo",
      "oss",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredGeneratedMatrixNameRequirements, ["workspace-live-sync-matrix"])
    assert.deepEqual(aggregate.missingRequiredGeneratedMatrixNames, [])
    assert.deepEqual(aggregate.requiredGeneratedMatrixRepoRequirements, ["oss"])
    assert.deepEqual(aggregate.missingRequiredGeneratedMatrixRepos, [])
    assert.equal(artifactIndex.metadata.requiredGeneratedMatrixNames, "workspace-live-sync-matrix")
    assert.equal(artifactIndex.metadata.missingGeneratedMatrixNames, undefined)
    assert.equal(artifactIndex.metadata.requiredGeneratedMatrixRepos, "oss")
    assert.equal(artifactIndex.metadata.missingGeneratedMatrixRepos, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        validationIndexPath,
        "--require-generated-matrix-name=workspace-live-sync-matrix",
        "--require-generated-matrix-repo=oss",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /generated_matrix_names_required=workspace-live-sync-matrix missing=workspace-live-sync-matrix/)
        assert.match(error.stdout, /generated_matrix_repos_required=oss missing=oss/)
        assert.match(error.stdout, /next: include drill artifact indexes that record generated matrix names: workspace-live-sync-matrix/)
        assert.match(error.stdout, /next: include drill artifact indexes that record generated matrix repos: oss/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        matrixIndexPath,
        "--require-generated-matrix-name=workspace-live-synch-matrix",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-matrix-name has unknown generated matrix name: workspace-live-synch-matrix/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        matrixIndexPath,
        "--require-generated-matrix-repo",
        "osz",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-matrix-repo has unknown generated matrix repo: osz/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates generated matrix artifact indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "two", "arroba.drill.matrix.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-generated-matrix-artifact-index",
      "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredGeneratedMatrixArtifactIndexPaths, ["/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"])
    assert.deepEqual(aggregate.missingGeneratedMatrixArtifactIndexPaths, [])
    assert.equal(artifactIndex.metadata.requiredGeneratedMatrixArtifactIndexes, "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json")
    assert.equal(artifactIndex.metadata.missingGeneratedMatrixArtifactIndexes, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-matrix-artifact-index=/tmp/generated-matrix/missing-artifacts.json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /generated_matrix_artifact_indexes_required=\/tmp\/generated-matrix\/missing-artifacts\.json missing=\/tmp\/generated-matrix\/missing-artifacts\.json/)
        assert.match(error.stdout, /next: rerun generated matrix drills with artifact indexes .*\/tmp\/generated-matrix\/missing-artifacts\.json/)
        assert.match(error.stdout, /sources: \/tmp\/generated-matrix\/missing-artifacts\.json/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-matrix-artifact-index=/tmp/generated-matrix/missing-artifacts.json",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const missing = JSON.parse(error.stdout)
        assert.deepEqual(missing.nextActions.map(({ classification, sourceDetails }) => ({ classification, sourceDetails })), [{
          classification: "generated-evidence",
          sourceDetails: [{ source: "/tmp/generated-matrix/missing-artifacts.json" }],
        }])
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-matrix-artifact-index=/tmp/generated-matrix/Bearer abcdefghijklmnop.json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-matrix-artifact-index includes secret-looking diagnostic text/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

