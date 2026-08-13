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

test("drill artifact index summary accepts explicit index paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "chariox.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--json",
    ])
    const aggregate = JSON.parse(stdout)

    assert.equal(aggregate.totals.indexes, 1)
    assert.equal(aggregate.totals.artifacts, 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary rejects empty inputs", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /no drill artifact indexes found/)
      return true
    },
  )
})

test("drill artifact index summary rejects tampered artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "chariox.drill.validation_gate.v1")
    await writeFile(path.join(rootDir, "one", "reports", "report.json"), "{\"schema\":\"tampered\"}\n", "utf8")

    await assert.rejects(
      execFile(process.execPath, [scriptPath, "--artifact-index", indexPath, "--json"]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /sha256 mismatch/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

