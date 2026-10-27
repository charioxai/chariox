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
  writeDrillArtifactIndex,
  writeDrillPlatformBundle,
  writeFile,
  scenario,
  writeCloudFailureTaxonomyRegistry,
  writeCloudGeneratedMatrixRegistry,
  writeCloudRuntimeAuthorityRegistry,
  writeCloudRuntimeSignalsRegistry,
  writeFailureManifest,
  writeMatrixReport,
  writeValidationSuiteArtifact,
} from '../drill-cross-repo-validation-gate.test-support.mjs'

test("cross repo validation gate requires artifact generated matrix limitation metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "chariox")
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const artifactIndexPath = await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      metadata: { generatedMatrixLimitations: "dry-run-classification-coverage" },
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--artifact-index",
        artifactIndexPath,
        "--require-artifact-generated-matrix-limitation",
        "dry-run-classification-covergae",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /unknown required artifact generated matrix limitation: dry-run-classification-covergae/)
        return true
      },
    )

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--no-default-roots",
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--artifact-index",
      artifactIndexPath,
      "--require-artifact-generated-matrix-limitation",
      "dry-run-classification-coverage",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.artifacts.requiredArtifactGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
    assert.deepEqual(report.checks.artifacts.missingArtifactGeneratedMatrixLimitations, [])
    assert.equal(report.checks.artifacts.aggregate.generatedMatrixLimitations["dry-run-classification-coverage"], 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate requires artifact generated matrix artifact-index metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "chariox")
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const generatedMatrixArtifactIndex = path.join(rootDir, "generated-matrix", "workspace-live-sync-matrix-artifacts.json")
    const artifactIndexPath = await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      metadata: { generatedMatrixArtifactIndexes: generatedMatrixArtifactIndex },
    })

    const missingGeneratedMatrixArtifactIndex = path.join(rootDir, "generated-matrix", "missing-matrix-artifacts.json")
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--no-default-roots",
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--artifact-index",
        artifactIndexPath,
        "--require-artifact-generated-matrix-artifact-index",
        missingGeneratedMatrixArtifactIndex,
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const failed = JSON.parse(error.stdout)
        assert.equal(failed.status, "failed")
        assert.deepEqual(failed.checks.artifacts.missingArtifactGeneratedMatrixArtifactIndexes, [
          missingGeneratedMatrixArtifactIndex,
        ])
        return true
      },
    )

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--no-default-roots",
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--artifact-index",
      artifactIndexPath,
      "--require-artifact-generated-matrix-artifact-index",
      generatedMatrixArtifactIndex,
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.artifacts.requiredArtifactGeneratedMatrixArtifactIndexes, [generatedMatrixArtifactIndex])
    assert.deepEqual(report.checks.artifacts.missingArtifactGeneratedMatrixArtifactIndexes, [])
    assert.equal(report.checks.artifacts.aggregate.generatedMatrixArtifactIndexes[generatedMatrixArtifactIndex], 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate requires artifact generated validation-suite failure-root metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "chariox")
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const generatedFailureRoot = path.join(rootDir, "generated-suite", "failed-run")
    const artifactIndexPath = await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      metadata: { generatedValidationSuiteFailureRoots: generatedFailureRoot },
    })

    const missingGeneratedFailureRoot = path.join(rootDir, "generated-suite", "missing-run")
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--no-default-roots",
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--artifact-index",
        artifactIndexPath,
        "--require-artifact-generated-validation-suite-failure-root",
        missingGeneratedFailureRoot,
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const failed = JSON.parse(error.stdout)
        assert.equal(failed.status, "failed")
        assert.deepEqual(failed.checks.artifacts.missingArtifactGeneratedValidationSuiteFailureRoots, [
          missingGeneratedFailureRoot,
        ])
        return true
      },
    )

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--no-default-roots",
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--artifact-index",
      artifactIndexPath,
      "--require-artifact-generated-validation-suite-failure-root",
      generatedFailureRoot,
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.artifacts.requiredArtifactGeneratedValidationSuiteFailureRoots, [generatedFailureRoot])
    assert.deepEqual(report.checks.artifacts.missingArtifactGeneratedValidationSuiteFailureRoots, [])
    assert.equal(report.checks.artifacts.aggregate.generatedValidationSuiteFailureRoots[generatedFailureRoot], 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate requires artifact generated validation-suite artifact-index metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "chariox")
    const cloudRoot = path.join(rootDir, "chariox-cloud")
    const generatedArtifactIndex = path.join(rootDir, "generated-suite", "chariox-drill-artifacts.json")
    const artifactIndexPath = await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      metadata: { generatedValidationSuiteArtifactIndexes: generatedArtifactIndex },
    })

    const missingGeneratedArtifactIndex = path.join(rootDir, "generated-suite", "missing-artifacts.json")
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--no-default-roots",
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--artifact-index",
        artifactIndexPath,
        "--require-artifact-generated-validation-suite-artifact-index",
        missingGeneratedArtifactIndex,
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const failed = JSON.parse(error.stdout)
        assert.equal(failed.status, "failed")
        assert.deepEqual(failed.checks.artifacts.missingArtifactGeneratedValidationSuiteArtifactIndexes, [
          missingGeneratedArtifactIndex,
        ])
        return true
      },
    )

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--no-default-roots",
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--artifact-index",
      artifactIndexPath,
      "--require-artifact-generated-validation-suite-artifact-index",
      generatedArtifactIndex,
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.artifacts.requiredArtifactGeneratedValidationSuiteArtifactIndexes, [generatedArtifactIndex])
    assert.deepEqual(report.checks.artifacts.missingArtifactGeneratedValidationSuiteArtifactIndexes, [])
    assert.equal(report.checks.artifacts.aggregate.generatedValidationSuiteArtifactIndexes[generatedArtifactIndex], 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

