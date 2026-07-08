import {
  assert,
  drillFailureTaxonomyManifest,
  drillRuntimeAuthorityManifest,
  drillRuntimeSignalsManifest,
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

test("cross repo validation gate checks generated matrix registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeCloudGeneratedMatrixRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--require-generated-matrix-registry-parity",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate rejects generated matrix registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeCloudGeneratedMatrixRegistry(cloudRoot, {
      matrices: [
        { name: "cloud-slice-runtime-matrix", repo: "cloud" },
        { name: "slice-runtime-matrix", repo: "oss" },
      ],
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--no-default-roots",
        "--require-generated-matrix-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /generated matrix registry parity failed/)
        assert.match(error.stderr, /workspace-live-sync-matrix/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate checks runtime signal registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeCloudRuntimeSignalsRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--require-runtime-signal-registry-parity",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate checks runtime authority registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeCloudRuntimeAuthorityRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--require-runtime-authority-registry-parity",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate checks failure taxonomy registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeCloudFailureTaxonomyRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--require-failure-taxonomy-registry-parity",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate rejects failure taxonomy registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillFailureTaxonomyManifest()
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: [
        ...manifest.classifications.filter((classification) => classification.kind === "kernel-authority"),
        {
          kind: "future-cloud-only-classification",
          owner: "kernel-authority",
          nextAction: "inspect future diagnostics",
        },
      ],
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--no-default-roots",
        "--require-failure-taxonomy-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /failure taxonomy registry parity failed/)
        assert.match(error.stderr, /future-cloud-only-classification/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate rejects runtime signal registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillRuntimeSignalsManifest()
    await writeCloudRuntimeSignalsRegistry(cloudRoot, {
      signals: manifest.signals.map((signal) => signal.id === "workspace-live-sync-state"
        ? { ...signal, owner: "kernel-authority" }
        : signal),
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--no-default-roots",
        "--require-runtime-signal-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /runtime signal registry parity failed/)
        assert.match(error.stderr, /workspace-live-sync-state/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate rejects runtime authority registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillRuntimeAuthorityManifest()
    await writeCloudRuntimeAuthorityRegistry(cloudRoot, {
      invariants: manifest.invariants.map((invariant) => invariant.id === "home-session-authority"
        ? { ...invariant, owner: "runtime-network" }
        : invariant),
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--no-default-roots",
        "--require-runtime-authority-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /runtime authority registry parity failed/)
        assert.match(error.stderr, /home-session-authority/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})
