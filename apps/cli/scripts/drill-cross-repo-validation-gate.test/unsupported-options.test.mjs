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

test("cross repo validation gate rejects aggregate-only generated evidence requirements", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-failure-max-age-ms", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-failure-max-age-ms requires a value/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-evidence-kind", "matrix-report", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-evidence-kind is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-matrix-artifact-index", "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-matrix-artifact-index is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-matrix-limitation", "dry-run-classification-coverage", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-matrix-limitation is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-validation-suite-artifact-index", "/tmp/generated-suite/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-validation-suite-artifact-index is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-validation-suite-failure-root", "/tmp/generated-suite/failed-run", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-validation-suite-failure-root is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
})

