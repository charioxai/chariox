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

test("drill artifact index summary gates provider account aliases", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "chariox-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "chariox.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-provider-account-alias",
      "codex=work",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredProviderAccountAliases, ["codex=work"])
    assert.deepEqual(aggregate.missingProviderAccountAliases, [])
    assert.equal(artifactIndex.metadata.requiredProviderAccountAliases, "codex=work")
    assert.equal(artifactIndex.metadata.missingProviderAccountAliases, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-provider-account-alias=opencode=zen",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /provider_account_aliases_required=opencode=zen missing=opencode=zen/)
        assert.match(error.stdout, /next: include drill artifact indexes that record provider account aliases: opencode=zen/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-provider-account-alias",
        "cdoex=work",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-provider-account-alias has invalid value/)
        assert.match(error.stderr, /unknown provider account alias provider: cdoex/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates planned dry-run diagnostics", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "chariox-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "two", "chariox.drill.matrix.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-planned-owner",
      "validation-harness",
      "--require-planned-classification",
      "matrix-coverage",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredPlannedOwners, ["validation-harness"])
    assert.deepEqual(aggregate.missingPlannedOwners, [])
    assert.deepEqual(aggregate.requiredPlannedClassifications, ["matrix-coverage"])
    assert.deepEqual(aggregate.missingPlannedClassifications, [])
    assert.deepEqual(aggregate.nextActions, [])
    assert.equal(artifactIndex.metadata.requiredPlannedOwners, "validation-harness")
    assert.equal(artifactIndex.metadata.requiredPlannedClassifications, "matrix-coverage")
    assert.equal(artifactIndex.metadata.missingPlannedOwners, undefined)
    assert.equal(artifactIndex.metadata.missingPlannedClassifications, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-planned-owner=kernel-authority",
        "--require-planned-classification=workspace-live-sync-conflict",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /planned_owners_required=kernel-authority missing=kernel-authority/)
        assert.match(error.stdout, /planned_classifications_required=workspace-live-sync-conflict missing=workspace-live-sync-conflict/)
        assert.match(error.stdout, /next: include dry-run drill matrix artifact indexes with planned owner coverage: kernel-authority/)
        assert.match(error.stdout, /next: include dry-run drill matrix artifact indexes with planned classification coverage: workspace-live-sync-conflict/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-planned-owner=kernel-authority",
        "--require-planned-classification=workspace-live-sync-conflict",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const missing = JSON.parse(error.stdout)
        assert.deepEqual(missing.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [
          {
            owner: "kernel-authority",
            classification: "artifact-coverage",
            nextAction: "include dry-run drill matrix artifact indexes with planned owner coverage: kernel-authority",
          },
          {
            owner: "runtime-state",
            classification: "workspace-live-sync-conflict",
            nextAction: "include dry-run drill matrix artifact indexes with planned classification coverage: workspace-live-sync-conflict",
          },
        ])
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-planned-owner",
        "Bearer abcdefghijklmnop",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-planned-owner includes secret-looking diagnostic text/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates validation presets", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "chariox-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "chariox.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-validation-preset",
      "distributed-runtime",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredValidationPresets, ["distributed-runtime"])
    assert.deepEqual(aggregate.missingValidationPresets, [])
    assert.equal(artifactIndex.metadata.requiredValidationPresets, "distributed-runtime")
    assert.equal(artifactIndex.metadata.missingValidationPresets, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-validation-preset=workspace-live-sync",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /validation_presets_required=workspace-live-sync missing=workspace-live-sync/)
        assert.match(error.stdout, /next: include drill artifact indexes that record validation presets: workspace-live-sync/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-validation-preset",
        "distributed-runtmie",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-validation-preset has unknown validation preset: distributed-runtmie/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates required failure classifications", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "chariox-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "chariox.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-failure-classification",
      "kernel-authority",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredFailureClassificationRequirements, ["kernel-authority"])
    assert.deepEqual(aggregate.missingFailureClassificationRequirements, [])
    assert.deepEqual(aggregate.nextActions, [])
    assert.equal(artifactIndex.metadata.requiredFailureClassifications, "kernel-authority")
    assert.equal(artifactIndex.metadata.missingFailureClassifications, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-failure-classification=workspace-live-sync-conflict",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /failure_classifications_required=workspace-live-sync-conflict missing=workspace-live-sync-conflict/)
        assert.match(error.stdout, /next: include drill artifact indexes with required failure classification coverage: workspace-live-sync-conflict/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-failure-classification",
        "kernel-autohority",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-failure-classification has unknown failure classification "kernel-autohority"/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates runtime authority invariants", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "chariox-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "chariox.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-runtime-authority-invariant",
      "home-session-authority",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredRuntimeAuthorityInvariantRequirements, ["home-session-authority"])
    assert.deepEqual(aggregate.missingRuntimeAuthorityInvariantRequirements, [])
    assert.deepEqual(aggregate.nextActions, [])
    assert.equal(artifactIndex.metadata.requiredRuntimeAuthorityInvariants, "home-session-authority")
    assert.equal(artifactIndex.metadata.missingRuntimeAuthorityInvariants, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-authority-invariant=worker-execution-authority",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /runtime_authority_invariants_required=worker-execution-authority missing=worker-execution-authority/)
        assert.match(error.stdout, /next: include drill artifact indexes with runtime authority invariant coverage: worker-execution-authority/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-authority-invariant",
        "home-session-authroity",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-runtime-authority-invariant has unknown runtime authority invariant "home-session-authroity"/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates runtime signals with owner-routed next actions", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "chariox-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "chariox.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-runtime-signal",
      "lease-health,session-authority",
      "--require-runtime-signal-owner",
      "kernel-authority",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredRuntimeSignalRequirements, ["lease-health", "session-authority"])
    assert.deepEqual(aggregate.missingRuntimeSignalRequirements, [])
    assert.deepEqual(aggregate.requiredRuntimeSignalOwnerRequirements, ["kernel-authority"])
    assert.deepEqual(aggregate.missingRuntimeSignalOwnerRequirements, [])
    assert.deepEqual(aggregate.nextActions, [])
    assert.equal(artifactIndex.metadata.requiredRuntimeSignals, "lease-health,session-authority")
    assert.equal(artifactIndex.metadata.requiredRuntimeSignalOwners, "kernel-authority")
    assert.equal(artifactIndex.metadata.missingRuntimeSignals, undefined)
    assert.equal(artifactIndex.metadata.missingRuntimeSignalOwners, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-signal=relay-target-freshness",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /runtime_signals_required=relay-target-freshness missing=relay-target-freshness/)
        assert.match(error.stdout, /next: include drill artifact indexes proving relay-target-freshness owned by runtime-network/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-signal=relay-target-freshness",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const missing = JSON.parse(error.stdout)
        assert.deepEqual(missing.nextActions.map(({ owner, classification, count }) => ({ owner, classification, count })), [{
          owner: "runtime-network",
          classification: "runtime-signal-coverage",
          count: 1,
        }])
        assert.match(missing.nextActions[0].nextAction, /relay-target-freshness owned by runtime-network/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-signal-owner=runtime-network",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /runtime_signal_owners_required=runtime-network missing=runtime-network/)
        assert.match(error.stdout, /next: include drill artifact indexes with runtime signal owner coverage: runtime-network/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-signal",
        "workspace-live-synch-state",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-runtime-signal has unknown runtime signal: workspace-live-synch-state/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-signal-owner",
        "kernel-authoritiy",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-runtime-signal-owner has unknown runtime signal owner: kernel-authoritiy/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

