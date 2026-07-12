import {
  assert,
  DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS,
  DISTRIBUTED_RUNTIME_GENERATED_MATRIX_NAMES_BY_REPO,
  DISTRIBUTED_RUNTIME_REQUIRED_FAILURE_CLASSIFICATIONS,
  DRILL_RUNTIME_AUTHORITY_INVARIANT_IDS,
  drillChaosContractManifest,
  drillFailureTaxonomyManifest,
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
  writeCloudChaosContractRegistry,
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

test("distributed runtime gate requires executed Cloud validation suite artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteManifestArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        assert.deepEqual(report.checks.artifacts.missingArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        assert.equal(report.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite.v1"], 1)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate requires Cloud validation suite distributed observability coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      coverageAreas: ["suite-contract"],
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      coverageAreas: ["suite-contract"],
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.artifacts.status, "failed")
        assert.deepEqual(report.checks.artifacts.requiredArtifactCoverageAreas, ["distributed-observability"])
        assert.deepEqual(report.checks.artifacts.missingArtifactCoverageAreas, ["distributed-observability"])
        assert.match(report.checks.artifacts.error, /missing required artifact coverage areas: distributed-observability/)
        assert.equal(report.checks.artifacts.aggregate.coverageAreas["suite-contract"], 2)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can include default failure manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), { evidenceRepo: "oss" })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    await writeFailureManifest(path.join(cloudRoot, ".artifacts", "failed-run", "arroba-drill-failure.json"), {
      drill: "cloud-slice-runtime-matrix",
      message: "slice auth stale projection",
    })

    const skipped = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--json",
    ])).stdout)
    assert.equal(skipped.status, "passed")
    assert.equal(skipped.checks.failures.status, "skipped")

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
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

test("distributed runtime gate accepts explicit failure manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), { evidenceRepo: "oss" })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    const failureManifest = await writeFailureManifest(path.join(rootDir, "preserved", "arroba-drill-failure.json"), {
      drill: "remote-agent-runtime-matrix",
      message: "worker lease expired",
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
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
        assert.equal(report.checks.failures.aggregate.total, 1)
        assert.equal(report.checks.failures.aggregate.failures[0].drill, "remote-agent-runtime-matrix")
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate reports missing hosted Cloud evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: false })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), { evidenceRepo: "oss" })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.matrices.missingDeploymentPresets, ["hosted-cloud"])
        assert.deepEqual(report.checks.matrices.missingScenarios, [
          "hetzner-collaborator-reconnect-authority",
          "hosted-browser-relay-kernel-reconnect",
          "hosted-cloud-relay-second-kernel-reconnect",
          "hosted-collab-remote-agent",
          "hosted-single-user-remote-agent",
          "local-browser-relay-kernel-reconnect",
          "ui-projection",
        ])
        assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
          { owner: "validation-harness", classification: "matrix-coverage" },
          { owner: "validation-harness", classification: "matrix-coverage" },
          { owner: "validation-harness", classification: "matrix-coverage" },
          { owner: "validation-harness", classification: "matrix-coverage" },
        ])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

test("distributed runtime gate can require generated matrix registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    await writeCloudGeneratedMatrixRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--require-generated-matrix-registry-parity",
      "--json",
    ])

    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can require chaos contract registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    await writeCloudChaosContractRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--require-chaos-contract-registry-parity",
      "--json",
    ])

    assert.equal(JSON.parse(stdout).status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects chaos contract registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillChaosContractManifest()
    await writeCloudChaosContractRegistry(cloudRoot, {
      ...manifest,
      replaySchema: "arroba.drill.chaos_replay.v2",
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--require-chaos-contract-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /chaos contract registry parity failed/)
        assert.match(error.stderr, /chaos_replay.v2/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects generated matrix registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeCloudGeneratedMatrixRegistry(cloudRoot, {
      matrices: [
        { name: "cloud-slice-runtime-matrix", repo: "cloud" },
      ],
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
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

test("distributed runtime gate can require runtime signal registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    await writeCloudRuntimeSignalsRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--require-runtime-signal-registry-parity",
      "--json",
    ])

    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can require runtime authority registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    await writeCloudRuntimeAuthorityRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--require-runtime-authority-registry-parity",
      "--json",
    ])

    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can require failure taxonomy registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    await writeCloudFailureTaxonomyRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--require-failure-taxonomy-registry-parity",
      "--json",
    ])

    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects failure taxonomy registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillFailureTaxonomyManifest()
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: manifest.classifications
        .map((classification) => classification.kind === "kernel-authority"
          ? { ...classification, owner: "runtime-state" }
          : classification),
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--require-failure-taxonomy-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /failure taxonomy registry parity failed/)
        assert.match(error.stderr, /kernel-authority/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects runtime signal registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillRuntimeSignalsManifest()
    await writeCloudRuntimeSignalsRegistry(cloudRoot, {
      signals: manifest.signals.filter((signal) => signal.id !== "home-extension-manifest-sync"),
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--require-runtime-signal-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /runtime signal registry parity failed/)
        assert.match(error.stderr, /home-extension-manifest-sync/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects runtime authority registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillRuntimeAuthorityManifest()
    await writeCloudRuntimeAuthorityRegistry(cloudRoot, {
      invariants: manifest.invariants.filter((invariant) => invariant.id !== "shared-runtime-primitives"),
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--require-runtime-authority-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /runtime authority registry parity failed/)
        assert.match(error.stderr, /shared-runtime-primitives/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects unsupported provider account aliases", async () => {
  for (const args of [
    ["--provider-account", "dev-stub=stub"],
    ["--provider-account=claude-headless=headless"],
  ]) {
    const rawAlias = args.at(-1).replace("--provider-account=", "")
    await assert.rejects(
      execFile(process.execPath, [scriptPath, ...args, "--json"]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /unsupported distributed-runtime provider account alias provider/)
        assert.doesNotMatch(error.stderr, new RegExp(rawAlias))
        assert.doesNotMatch(error.stdout, new RegExp(rawAlias))
        return true
      },
    )
  }
})

test("distributed runtime gate rejects requirement flags without values", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-runtime-signal", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-runtime-signal requires a value/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-failure-max-age-ms", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-failure-max-age-ms requires a value/)
      return true
    },
  )
})

test("distributed runtime gate rejects aggregate-only generated evidence requirements", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-evidence-kind", "matrix-report", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-evidence-kind is supported by drill-validation-gate-summary\.mjs/)
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
    execFile(process.execPath, [scriptPath, "--require-generated-validation-suite-failure-root", "/tmp/generated-suite/failed-run", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-validation-suite-failure-root is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
})
