import {
  assert,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
  os,
  path,
  test,
  DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA,
  DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS,
  DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS,
  DRILL_ARTIFACT_INDEX_SCHEMA,
  diagnosticMetadataForDrillArtifactIndexAggregate,
  drillFailureTaxonomyManifest,
  drillRuntimeSignalsManifest,
  emptyDrillArtifactDiagnosticDimensions,
  findDrillArtifactIndexPaths,
  focusedRuntimeGateReportArtifact,
  formatDrillArtifactIndexAggregateSummary,
  matrixReportArtifact,
  prepareDrillArtifacts,
  readDrillArtifactIndex,
  summarizeDrillArtifactIndexes,
  validateDrillArtifactDiagnosticDimensions,
  validateDrillArtifactIndex,
  validateDrillArtifactIndexAggregate,
  validationGateReportArtifact,
  validationSuiteManifestArtifact,
  validationSuiteRunArtifact,
  verifyDrillArtifactIndex,
  writeDrillArtifactIndex,
  writeDrillJsonArtifactOutput,
  finalizeDrillArtifacts,
} from '../drill-artifacts.test-support.mjs'

test("drill artifacts are removed after a passing run", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-pass-"))
  await prepareDrillArtifacts(root)

  const result = await finalizeDrillArtifacts({ rootDir: root, passed: true })

  assert.equal(result.preserved, false)
  assert.equal(result.rootDir, root)
  await assert.rejects(stat(root), /ENOENT/)
})

test("drill artifacts can be preserved after a passing run", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-pass-preserved-"))
  const events = []
  await prepareDrillArtifacts(root)
  await writeFile(path.join(root, "evidence.txt"), "kept\n", "utf8")

  const result = await finalizeDrillArtifacts({
    rootDir: root,
    passed: true,
    preserveOnSuccess: true,
    log: (name, details) => events.push({ name, details }),
  })

  assert.equal(result.preserved, true)
  assert.equal(result.rootDir, root)
  assert.equal(await readFile(path.join(root, "evidence.txt"), "utf8"), "kept\n")
  assert.deepEqual(events, [{ name: "preserved-successful-run", details: { rootDir: root } }])

  await rm(root, { recursive: true, force: true })
})

test("normalizes drill artifact lifecycle roots to absolute paths", async () => {
  const targetRoot = path.join(process.cwd(), "target")
  await mkdir(targetRoot, { recursive: true })
  const absoluteRoot = await mkdtemp(path.join(targetRoot, "arroba-drill-artifacts-lifecycle-"))
  const relativeRoot = path.relative(process.cwd(), absoluteRoot)
  const events = []

  await prepareDrillArtifacts(relativeRoot)
  const result = await finalizeDrillArtifacts({
    rootDir: relativeRoot,
    passed: false,
    failure: new Error("relative failure"),
    log: (name, details) => events.push({ name, details }),
  })

  assert.equal(result.rootDir, absoluteRoot)
  assert.equal(result.manifestPath, path.join(absoluteRoot, "arroba-drill-failure.json"))
  assert.deepEqual(events, [{
    name: "preserved-failed-run",
    details: {
      rootDir: absoluteRoot,
      manifestPath: path.join(absoluteRoot, "arroba-drill-failure.json"),
    },
  }])
  const manifest = JSON.parse(await readFile(result.manifestPath, "utf8"))
  assert.equal(manifest.rootDir, absoluteRoot)

  await assert.rejects(
    prepareDrillArtifacts(""),
    /rootDir is required/,
  )
  await finalizeDrillArtifacts({ rootDir: absoluteRoot, passed: true })
})

test("drill artifacts are preserved with a failure manifest after a failed run", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-fail-"))
  const events = []
  await prepareDrillArtifacts(root)

  const failure = new Error("relay target was stale with Bearer abcdefghijklmnopqrstuvwxyz")
  failure.stack = "Error: relay target was stale with sk-this-should-not-persist\n    at drill"

  const result = await finalizeDrillArtifacts({
    rootDir: root,
    passed: false,
    failure,
    metadata: {
      drill: "hosted-cloud-relay",
      relayToken: "relay-token-should-not-persist",
      provider: "Bearer abcdefghijklmnopqrstuvwxyz",
      nested: { apiKey: "sk-this-should-not-persist" },
    },
    log: (name, details) => events.push({ name, details }),
  })

  assert.equal(result.preserved, true)
  assert.equal(events[0].name, "preserved-failed-run")
  const manifest = JSON.parse(await readFile(result.manifestPath, "utf8"))
  assert.equal(manifest.schema, "arroba.drill.failure.v1")
  assert.equal(manifest.metadata.drill, "hosted-cloud-relay")
  assert.equal(manifest.metadata.relayToken, "<redacted>")
  assert.equal(manifest.metadata.provider, "<redacted>")
  assert.equal(manifest.metadata.nested.apiKey, "<redacted>")
  assert.equal(manifest.error.message, "relay target was stale with <redacted>")
  assert.match(manifest.error.stack, /Error: relay target was stale with <redacted>/)
  assert.doesNotMatch(JSON.stringify(manifest), /should-not-persist|abcdefghijklmnopqrstuvwxyz/)

  await finalizeDrillArtifacts({ rootDir: root, passed: true })
})

test("writes and verifies drill artifact indexes", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-index-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "gate.json"), `${JSON.stringify({
      schema: "arroba.drill.validation_gate.v1",
      status: "passed",
    })}\n`, "utf8")
    await writeFile(path.join(root, "reports", "notes.log"), "plain log\n", "utf8")

    const index = await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/notes.log", "reports/gate.json"],
      metadata: {
        drill: "validation-gate",
        relayToken: "relay-token-should-not-persist",
      },
    })
    const indexPath = path.join(root, "arroba-drill-artifacts.json")
    const readIndex = await readDrillArtifactIndex(indexPath)
    const verified = await verifyDrillArtifactIndex(indexPath)

    assert.equal(index.schema, DRILL_ARTIFACT_INDEX_SCHEMA)
    assert.equal(index.rootDir, root)
    assert.deepEqual(readIndex, index)
    assert.deepEqual(verified, index)
    assert.equal(index.metadata.relayToken, "<redacted>")
    assert.deepEqual(index.artifacts.map((artifact) => artifact.path), [
      "reports/gate.json",
      "reports/notes.log",
    ])
    assert.deepEqual(index.artifacts.map((artifact) => artifact.schema), [
      "arroba.drill.validation_gate.v1",
      null,
    ])
    assert.doesNotMatch(JSON.stringify(index), /should-not-persist/)
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("normalizes drill artifact index roots to absolute paths", async () => {
  const targetRoot = path.join(process.cwd(), "target")
  await mkdir(targetRoot, { recursive: true })
  const root = await mkdtemp(path.join(targetRoot, "arroba-drill-artifacts-relative-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "gate.json"), "{\"schema\":\"arroba.drill.validation_gate.v1\"}\n", "utf8")
    const relativeRoot = path.relative(process.cwd(), root)

    const index = await writeDrillArtifactIndex({
      rootDir: relativeRoot,
      artifacts: ["reports/gate.json"],
    })
    const verified = await verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json"))

    assert.equal(index.rootDir, root)
    assert.deepEqual(verified, index)
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("verifies runtime signal manifests embedded in validation artifacts", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-runtime-signals-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact(), null, 2)}\n`, "utf8")

    const index = await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
      metadata: {
        runtimeSignals: "session-authority",
        runtimeSignalOwners: "kernel-authority",
      },
    })

    assert.deepEqual(await verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")), index)
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation artifacts that advertise runtime signals without a manifest", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-runtime-signals-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact({
      manifest: {
        schema: "arroba.drill.validation_suite.v1",
        command: "node --test apps/cli/scripts/lib/drill-artifacts.test.mjs",
        testCount: 1,
        testPaths: ["apps/cli/scripts/lib/drill-artifacts.test.mjs"],
      },
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
      metadata: {
        runtimeSignals: "session-authority",
        runtimeSignalOwners: "kernel-authority",
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json is missing manifest\.runtimeSignalsManifest/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation artifacts that advertise required runtime signals without a manifest", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-runtime-signals-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact({
      manifest: {
        schema: "arroba.drill.validation_suite.v1",
        command: "node --test apps/cli/scripts/lib/drill-artifacts.test.mjs",
        testCount: 1,
        testPaths: ["apps/cli/scripts/lib/drill-artifacts.test.mjs"],
      },
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
      metadata: {
        requiredRuntimeSignals: "session-authority",
        requiredRuntimeSignalOwners: "kernel-authority",
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json is missing manifest\.runtimeSignalsManifest/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation artifacts that advertise required runtime authority invariants without a manifest", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-runtime-authority-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact(), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
      metadata: {
        requiredRuntimeAuthorityInvariants: "home-session-authority",
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json is missing manifest\.runtimeAuthorityManifest/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation artifacts that advertise required failure classifications without a taxonomy manifest", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-failure-taxonomy-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact({
      manifest: {
        schema: "arroba.drill.validation_suite.v1",
        command: "node --test apps/cli/scripts/lib/drill-artifacts.test.mjs",
        testCount: 1,
        testPaths: ["apps/cli/scripts/lib/drill-artifacts.test.mjs"],
      },
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
      metadata: {
        requiredFailureClassifications: "kernel-authority",
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json is missing manifest\.failureTaxonomyManifest/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation artifacts with malformed runtime signal manifests", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-runtime-signals-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    const manifest = drillRuntimeSignalsManifest()
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact({
      manifest: {
        schema: "arroba.drill.validation_suite.v1",
        command: "node --test apps/cli/scripts/lib/drill-artifacts.test.mjs",
        testCount: 1,
        testPaths: ["apps/cli/scripts/lib/drill-artifacts.test.mjs"],
        failureTaxonomyManifest: drillFailureTaxonomyManifest(),
        runtimeSignalsManifest: {
          ...manifest,
          signals: manifest.signals.filter((signal) => signal.id !== "lease-health"),
        },
      },
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
      metadata: {
        runtimeSignals: "session-authority",
        runtimeSignalOwners: "kernel-authority",
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json\.manifest\.runtimeSignalsManifest does not match required runtime signals/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects malformed validation suite run artifacts", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-suite-run-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact({
      ok: false,
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json ok does not match status/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation suite run artifacts with inconsistent manifest fields", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-suite-run-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact({
      testCount: 2,
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json\.testCount must match manifest\.testCount/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects malformed validation suite manifest artifacts", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-suite-manifest-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite.json"), `${JSON.stringify({
      schema: "arroba.drill.validation_suite.v1",
    }, null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite.json"],
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite\.json is missing command/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation suite artifacts with mismatched metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-suite-metadata-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite.json"), `${JSON.stringify(validationSuiteManifestArtifact(), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite.json"],
      metadata: {
        artifactKinds: "matrix-report",
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite\.json metadata\.artifactKinds must include validation-suite/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects validation suite run artifacts with stale metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-suite-metadata-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite-run.json"), `${JSON.stringify(validationSuiteRunArtifact(), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/suite-run.json"],
      metadata: {
        artifactKinds: "validation-suite-run",
        status: "failed",
        tests: 1,
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /suite-run\.json metadata\.status must match artifact status/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects malformed matrix report artifacts", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-matrix-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "matrix.json"), `${JSON.stringify({
      schema: "arroba.drill.matrix.v1",
    }, null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/matrix.json"],
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /matrix\.json is missing matrix/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("verifies focused runtime gate artifacts", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-focused-runtime-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "focused.json"), `${JSON.stringify(focusedRuntimeGateReportArtifact(), null, 2)}\n`, "utf8")

    const index = await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/focused.json"],
      metadata: {
        artifactKinds: "focused-runtime-gate",
        drill: "focused-runtime-gate",
        runtimeSignals: "session-authority",
        runtimeSignalOwners: "kernel-authority",
      },
    })
    const verified = await verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json"))

    assert.deepEqual(verified, index)
    assert.deepEqual(index.artifacts.map((artifact) => artifact.schema), ["arroba.drill.focused_runtime_gate.v1"])
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects malformed focused runtime gate artifacts", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-focused-runtime-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "focused.json"), `${JSON.stringify(focusedRuntimeGateReportArtifact({
      status: "failed",
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/focused.json"],
    })
    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /focused\.json status does not match embedded report statuses/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects focused runtime gate artifacts with stale metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-focused-runtime-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "focused.json"), `${JSON.stringify(focusedRuntimeGateReportArtifact(), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/focused.json"],
      metadata: {
        artifactKinds: "validation-gate",
      },
    })
    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /focused\.json metadata\.artifactKinds must include focused-runtime-gate/,
    )

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/focused.json"],
      metadata: {
        artifactKinds: "focused-runtime-gate",
        status: "failed",
      },
    })
    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /focused\.json metadata\.status must match artifact status/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects matrix report artifacts with stale metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-matrix-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "matrix.json"), `${JSON.stringify(matrixReportArtifact(), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/matrix.json"],
      metadata: {
        artifactKinds: "matrix-report",
        matrix: "other-matrix",
        status: "passed",
        dryRun: false,
        scenarios: 1,
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /matrix\.json metadata\.matrix must match artifact matrix/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects matrix report artifacts with stale planned diagnostic metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-matrix-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "matrix.json"), `${JSON.stringify(matrixReportArtifact({
      status: "dry-run",
      dryRun: true,
      scenarios: [{
        id: "dry",
        description: "dry scenario",
        requires: [],
        exitCriteria: [],
        exitCriteriaEvidence: [],
        runtimeSignals: [],
        status: "dry-run",
        expectedFailure: false,
        classification: null,
        owner: null,
        nextAction: null,
        plannedClassification: "workspace-live-sync-conflict",
        plannedOwner: "runtime-state",
        plannedNextAction: "inspect workspace live sync status, conflicts, and preserved file snapshots; reconcile the conflict, then rerun the scenario",
        durationMs: 0,
        reason: null,
        command: "node",
        args: ["--version"],
        artifactHints: [],
      }],
    }), null, 2)}\n`, "utf8")

    await writeDrillArtifactIndex({
      rootDir: root,
      artifacts: ["reports/matrix.json"],
      metadata: {
        plannedOwners: "kernel-authority",
        plannedClassifications: "workspace-live-sync-conflict",
      },
    })

    await assert.rejects(
      verifyDrillArtifactIndex(path.join(root, "arroba-drill-artifacts.json")),
      /matrix\.json metadata\.plannedOwners must match artifact planned diagnostics/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("rejects inconsistent drill runtime signal owner metadata", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-runtime-signals-"))
  try {
    await mkdir(path.join(root, "reports"), { recursive: true })
    await writeFile(path.join(root, "reports", "suite.json"), `${JSON.stringify({
      schema: "arroba.drill.validation_suite.v1",
      runtimeSignalsManifest: drillRuntimeSignalsManifest(),
    })}\n`, "utf8")

    await assert.rejects(
      writeDrillArtifactIndex({
        rootDir: root,
        artifacts: ["reports/suite.json"],
        metadata: { runtimeSignals: "session-authority" },
      }),
      /runtimeSignalOwners must match runtimeSignals/,
    )
    await assert.rejects(
      writeDrillArtifactIndex({
        rootDir: root,
        artifacts: ["reports/suite.json"],
        metadata: { runtimeSignalOwners: "kernel-authority" },
      }),
      /runtimeSignalOwners requires runtimeSignals/,
    )
    await assert.rejects(
      writeDrillArtifactIndex({
        rootDir: root,
        artifacts: ["reports/suite.json"],
        metadata: {
          runtimeSignals: "workspace-live-synch-state",
          runtimeSignalOwners: "runtime-state",
        },
      }),
      /drill runtime signals\[0\] has unknown runtime signal "workspace-live-synch-state"/,
    )
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})
