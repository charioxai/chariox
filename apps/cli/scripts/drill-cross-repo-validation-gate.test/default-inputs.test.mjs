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

test("cross repo validation gate keeps default artifact roots opt-in", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    const skipped = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--json",
    ])).stdout)
    assert.equal(skipped.checks.artifacts.status, "skipped")

    const discovered = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--include-default-artifacts",
      "--json",
    ])).stdout)
    assert.equal(discovered.checks.artifacts.status, "passed")
    assert.equal(discovered.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate accepts explicit artifact evidence inputs", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    const artifactRoot = path.join(rootDir, "artifact-root")
    await writeDrillPlatformBundle(bundleDir)
    const artifactIndex = await writeValidationSuiteArtifact(artifactRoot)

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--artifact-index",
      artifactIndex,
      "--json",
    ])).stdout)

    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.inputs, [artifactIndex])
    assert.deepEqual(report.checks.artifacts.roots, [])
    assert.equal(report.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate keeps default failure roots opt-in", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeFailureManifest(path.join(cloudRoot, ".artifacts", "failed-run", "arroba-drill-failure.json"), {
      drill: "cloud-slice-runtime-matrix",
      message: "relay target stale",
    })

    const skipped = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--json",
    ])).stdout)
    assert.equal(skipped.checks.failures.status, "skipped")

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--no-default-roots",
        "--include-default-failures",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.failures.status, "failed")
        assert.deepEqual(report.checks.failures.roots, [
          path.join(cloudRoot, ".artifacts"),
          path.join(ossRoot, ".artifacts"),
        ].sort())
        assert.equal(report.checks.failures.aggregate.total, 1)
        assert.equal(report.checks.failures.aggregate.failures[0].drill, "cloud-slice-runtime-matrix")
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate accepts explicit failure manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    const failureManifest = await writeFailureManifest(path.join(rootDir, "preserved", "arroba-drill-failure.json"), {
      drill: "slice-runtime-matrix",
      message: "slice launch timed out",
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--no-default-roots",
        "--platform-bundle",
        bundleDir,
        "--failure-manifest",
        failureManifest,
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.failures.status, "failed")
        assert.deepEqual(report.checks.failures.inputs, [failureManifest])
        assert.deepEqual(report.checks.failures.roots, [])
        assert.equal(report.checks.failures.aggregate.total, 1)
        assert.equal(report.checks.failures.aggregate.failures[0].drill, "slice-runtime-matrix")
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate can disable default roots for focused evidence checks", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossMatrixRoot = path.join(rootDir, "oss-matrices")
    const cloudMatrixRoot = path.join(rootDir, "cloud-matrices")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeMatrixReport(path.join(ossMatrixRoot, "slice-runtime.json"), {
      matrix: "slice-runtime-matrix",
      metadata: {
        deploymentPresets: "local,self-hosted-relay",
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("slice-lifecycle", "slice-runtime", ["slice-runtime-state"]),
        scenario("provider-auth", "slice-auth", ["provider-run-lifecycle", "slice-auth-state"]),
        scenario("session-start", "kernel-authority", ["session-authority"]),
        scenario("agent-reuse", "worker-execution", ["agent-lifecycle"]),
        scenario("ui-projection", "ui-client-projection", ["client-projection-health", "runtime-projection-health"]),
        scenario("docker-browser-state", "docker-runtime", ["slice-runtime-state"]),
      ],
    })
    await writeMatrixReport(path.join(cloudMatrixRoot, "cloud-slice-runtime.json"), {
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
        providerCount: 3,
        providers: "claude,codex,opencode",
        defaultModel: "provider-default",
        providerModelOverrides: "",
      },
      scenarios: [
        scenario("hosted-slice-browser-e2e", "ui-client-projection", ["client-projection-health", "runtime-projection-health"], { providers: ["claude", "codex", "opencode"] }),
      ],
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--no-default-roots",
        "--matrix-root",
        ossMatrixRoot,
        "--platform-bundle",
        bundleDir,
        "--preset",
        "slice-runtime",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.matrices.missingDeploymentPresets, ["hosted-cloud"])
        return true
      },
    )

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--no-default-roots",
      "--matrix-root",
      ossMatrixRoot,
      "--matrix-root",
      cloudMatrixRoot,
      "--platform-bundle",
      bundleDir,
      "--preset",
      "slice-runtime",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

