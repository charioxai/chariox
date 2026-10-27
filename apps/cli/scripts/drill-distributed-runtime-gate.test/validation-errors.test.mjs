import {
  assert,
  DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS,
  DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES_BY_REPO,
  DISTRIBUTED_RUNTIME_REQUIRED_FAILURE_CLASSIFICATIONS,
  DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS,
  drillRuntimeAuthorityManifest,
  drillRuntimeSignalOwnersFor,
  drillRuntimeSignalsManifest,
  execFile,
  generatedMatrixNamesForEvidenceRepo,
  matrixReport,
  mkdtemp,
  os,
  path,
  readFile,
  rm,
  scenario,
  scriptPath,
  summaryScriptPath,
  test,
  verifyDrillArtifactIndex,
  writeCloudFailureTaxonomyRegistry,
  writeCloudGeneratedMatrixRegistry,
  writeCloudRuntimeAuthorityRegistry,
  writeCloudRuntimeSignalsRegistry,
  writeDistributedRuntimeMatrices,
  writeDrillArtifactIndex,
  writeFailureManifest,
  writeFakeDistributedRuntimeMatrixScripts,
  writeFakeMatrixScript,
  writeFakeValidationSuiteScript,
  writeValidationSuiteArtifact,
  writeValidationSuiteManifestArtifact,
} from '../drill-distributed-runtime-gate.test-support.mjs'

test("distributed runtime gate rejects secret-looking generated output roots", async () => {
  for (const flag of ["--validation-suite-output-root", "--matrix-output-root"]) {
    await assert.rejects(
      execFile(process.execPath, [scriptPath, flag, "/tmp/Bearer abcdefghijklmnop", "--json"]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, new RegExp(`${flag} includes secret-looking generated evidence path`))
        assert.doesNotMatch(error.stderr, /Bearer abcdefghijklmnop/)
        assert.doesNotMatch(error.stdout, /Bearer abcdefghijklmnop/)
        return true
      },
    )
  }
})
