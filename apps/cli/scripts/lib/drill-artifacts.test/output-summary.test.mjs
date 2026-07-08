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

test("writes JSON artifact output with optional index", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-output-"))
  try {
    const outputPath = path.join(root, "reports", "gate.json")
    const artifactIndexPath = path.join(root, "reports", "arroba-drill-artifacts.json")
    const artifactIndex = await writeDrillJsonArtifactOutput({
      outputPath,
      artifactIndexPath,
      value: {
        schema: "arroba.drill.validation_gate.v1",
        status: "passed",
      },
      metadata: {
        drill: "validation-gate",
        token: "sk-this-should-not-persist",
      },
    })
    const fileValue = JSON.parse(await readFile(outputPath, "utf8"))
    const verified = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.equal(fileValue.status, "passed")
    assert.deepEqual(verified, artifactIndex)
    assert.equal(artifactIndex.metadata.drill, "validation-gate")
    assert.equal(artifactIndex.metadata.token, "<redacted>")
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "gate.json",
      schema: "arroba.drill.validation_gate.v1",
    }])
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("discovers drill artifact indexes", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-index-"))
  try {
    await mkdir(path.join(root, "one", "reports"), { recursive: true })
    await mkdir(path.join(root, "two", "reports"), { recursive: true })
    await writeFile(path.join(root, "one", "reports", "gate.json"), "{\"schema\":\"one\"}\n", "utf8")
    await writeFile(path.join(root, "two", "reports", "gate.json"), "{\"schema\":\"two\"}\n", "utf8")
    const firstIndexPath = path.join(root, "one", "arroba-drill-artifacts.json")
    const secondIndexPath = path.join(root, "two", "arroba-drill-artifacts.json")
    await writeDrillArtifactIndex({
      rootDir: path.join(root, "one"),
      artifacts: ["reports/gate.json"],
    })
    await writeDrillArtifactIndex({
      rootDir: path.join(root, "two"),
      artifacts: ["reports/gate.json"],
    })
    await writeFile(path.join(root, "arroba-drill-artifacts.json"), "{\"schema\":\"other\"}\n", "utf8")

    assert.deepEqual(await findDrillArtifactIndexPaths([root]), [
      firstIndexPath,
      secondIndexPath,
    ])
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})

test("summarizes drill artifact indexes", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-artifacts-index-"))
  try {
    await mkdir(path.join(root, "one", "reports"), { recursive: true })
    await mkdir(path.join(root, "two", "reports"), { recursive: true })
    await writeFile(path.join(root, "one", "reports", "gate.json"), `${JSON.stringify({
      schema: "arroba.drill.validation_gate.v1",
    })}\n`, "utf8")
    await writeFile(path.join(root, "one", "reports", "notes.log"), "plain log\n", "utf8")
    await writeFile(path.join(root, "two", "reports", "matrix.json"), `${JSON.stringify({
      schema: "arroba.drill.matrix.v1",
    })}\n`, "utf8")
    const first = await writeDrillArtifactIndex({
      rootDir: path.join(root, "one"),
      artifacts: ["reports/gate.json", "reports/notes.log"],
      metadata: {
        classifications: "validation-gate,artifact-coverage",
        owners: "validation-harness",
        runtimeSignals: "session-authority,provider-run-lifecycle",
        runtimeSignalOwners: "kernel-authority,provider-runtime",
        requiredRuntimeSignals: "provider-run-lifecycle,session-authority",
        requiredRuntimeSignalOwners: "kernel-authority,provider-runtime",
        missingRuntimeSignals: "relay-target-freshness",
        missingRuntimeSignalOwners: "runtime-network",
        runtimeAuthorityInvariants: "client-render-request,home-session-authority",
        validationPresets: "distributed-runtime,workspace-live-sync",
        requiredFailureClassifications: "kernel-authority,provider-auth",
        missingFailureClassifications: "provider-auth",
        artifactKinds: "validation-gate,validation-suite-run",
        generatedEvidenceKinds: "validation-suite-run",
        generatedValidationSuiteArtifactIndexes: "/tmp/generated-suite/arroba-drill-artifacts.json",
        generatedValidationSuiteFailureRoots: "/tmp/generated-suite/failed-run",
        generatedEvidenceRepos: "oss",
        requiredGeneratedEvidenceKinds: "matrix-report,validation-suite-run",
        missingGeneratedEvidenceKinds: "matrix-report",
        requiredGeneratedValidationSuiteArtifactIndexes: "/tmp/generated-suite/arroba-drill-artifacts.json",
        missingGeneratedValidationSuiteArtifactIndexes: "/tmp/generated-suite/missing-artifacts.json",
        requiredGeneratedValidationSuiteFailureRoots: "/tmp/generated-suite/failed-run",
        missingGeneratedValidationSuiteFailureRoots: "/tmp/generated-suite/missing-run",
        providerAccountAliases: "codex=work,opencode=zen",
        evidenceRepos: "oss",
      },
    })
    const second = await writeDrillArtifactIndex({
      rootDir: path.join(root, "two"),
      artifacts: ["reports/matrix.json"],
      metadata: {
        classifications: "matrix-coverage",
        owners: "validation-harness,runtime-network",
        plannedClassifications: "workspace-live-sync-conflict",
        plannedOwners: "runtime-state",
        exitCriterionStatuses: "dry-run",
        incompleteExitCriterionStatuses: "dry-run",
        runtimeSignals: "session-authority,lease-health",
        runtimeSignalOwners: "kernel-authority",
        requiredRuntimeSignals: "lease-health,session-authority",
        requiredRuntimeSignalOwners: "kernel-authority",
        validationPresets: "distributed-runtime,slice-runtime",
        requiredFailureClassifications: "relay-target-freshness,workspace-live-sync-conflict",
        artifactKinds: "matrix-report",
        generatedEvidenceKinds: "matrix-report",
        generatedMatrixArtifactIndexes: "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
        generatedMatrixLimitations: "dry-run-classification-coverage",
        generatedMatrixNames: "workspace-live-sync-matrix",
        generatedMatrixRepos: "oss",
        generatedEvidenceRepos: "cloud,oss",
        requiredGeneratedEvidenceKinds: "matrix-report",
        requiredGeneratedMatrixArtifactIndexes: "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
        missingGeneratedMatrixArtifactIndexes: "/tmp/generated-matrix/missing-matrix-artifacts.json",
        requiredGeneratedMatrixLimitations: "dry-run-classification-coverage",
        missingGeneratedMatrixLimitations: "dry-run-classification-coverage",
        requiredGeneratedMatrixNames: "workspace-live-sync-matrix",
        missingGeneratedMatrixNames: "remote-agent-runtime-matrix",
        requiredGeneratedMatrixRepos: "oss",
        missingGeneratedMatrixRepos: "cloud",
        providerAccountAliases: "codex=work,claude=team",
        evidenceRepos: "cloud,oss",
        artifactCoverageInputSources: "artifact metadata inputs",
      },
    })

    const aggregate = summarizeDrillArtifactIndexes([first, second], {
      sources: ["one/arroba-drill-artifacts.json", "two/arroba-drill-artifacts.json"],
    })

    assert.equal(aggregate.schema, DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA)
    assert.equal(aggregate.totals.indexes, 2)
    assert.equal(aggregate.totals.artifacts, 3)
    assert.deepEqual(aggregate.schemas, {
      "arroba.drill.matrix.v1": 1,
      "arroba.drill.validation_gate.v1": 1,
      none: 1,
    })
    assert.deepEqual(aggregate.runtimeSignals, {
      "lease-health": 1,
      "provider-run-lifecycle": 1,
      "session-authority": 2,
    })
    assert.deepEqual(aggregate.runtimeSignalOwners, {
      "kernel-authority": 2,
      "provider-runtime": 1,
    })
    assert.deepEqual(aggregate.requiredRuntimeSignals, {
      "lease-health": 1,
      "provider-run-lifecycle": 1,
      "session-authority": 2,
    })
    assert.deepEqual(aggregate.requiredRuntimeSignalOwners, {
      "kernel-authority": 2,
      "provider-runtime": 1,
    })
    assert.deepEqual(aggregate.missingRuntimeSignals, {
      "relay-target-freshness": 1,
    })
    assert.deepEqual(aggregate.missingRuntimeSignalOwners, {
      "runtime-network": 1,
    })
    assert.deepEqual(aggregate.runtimeAuthorityInvariants, {
      "client-render-request": 1,
      "home-session-authority": 1,
    })
    assert.deepEqual(aggregate.validationPresets, {
      "distributed-runtime": 2,
      "slice-runtime": 1,
      "workspace-live-sync": 1,
    })
    assert.deepEqual(aggregate.owners, {
      "runtime-network": 1,
      "validation-harness": 2,
    })
    assert.deepEqual(aggregate.classifications, {
      "artifact-coverage": 1,
      "matrix-coverage": 1,
      "validation-gate": 1,
    })
    assert.deepEqual(aggregate.requiredFailureClassifications, {
      "kernel-authority": 1,
      "provider-auth": 1,
      "relay-target-freshness": 1,
      "workspace-live-sync-conflict": 1,
    })
    assert.deepEqual(aggregate.missingFailureClassifications, {
      "provider-auth": 1,
    })
    assert.deepEqual(aggregate.plannedClassifications, {
      "workspace-live-sync-conflict": 1,
    })
    assert.deepEqual(aggregate.plannedOwners, {
      "runtime-state": 1,
    })
    assert.deepEqual(aggregate.exitCriterionStatuses, {
      "dry-run": 1,
    })
    assert.deepEqual(aggregate.incompleteExitCriterionStatuses, {
      "dry-run": 1,
    })
    assert.deepEqual(aggregate.artifactKinds, {
      "matrix-report": 1,
      "validation-gate": 1,
      "validation-suite-run": 1,
    })
    assert.deepEqual(aggregate.generatedEvidenceKinds, {
      "matrix-report": 1,
      "validation-suite-run": 1,
    })
    assert.deepEqual(aggregate.generatedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(aggregate.generatedMatrixArtifactIndexes, {
      "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json": 1,
    })
    assert.deepEqual(aggregate.generatedMatrixNames, {
      "workspace-live-sync-matrix": 1,
    })
    assert.deepEqual(aggregate.generatedMatrixRepos, {
      oss: 1,
    })
    assert.deepEqual(aggregate.generatedEvidenceRepos, {
      cloud: 1,
      oss: 2,
    })
    assert.deepEqual(aggregate.generatedValidationSuiteArtifactIndexes, {
      "/tmp/generated-suite/arroba-drill-artifacts.json": 1,
    })
    assert.deepEqual(aggregate.generatedValidationSuiteFailureRoots, {
      "/tmp/generated-suite/failed-run": 1,
    })
    assert.deepEqual(aggregate.requiredGeneratedEvidenceKinds, {
      "matrix-report": 2,
      "validation-suite-run": 1,
    })
    assert.deepEqual(aggregate.missingGeneratedEvidenceKinds, {
      "matrix-report": 1,
    })
    assert.deepEqual(aggregate.requiredGeneratedMatrixArtifactIndexes, {
      "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json": 1,
    })
    assert.deepEqual(aggregate.missingGeneratedMatrixArtifactIndexes, {
      "/tmp/generated-matrix/missing-matrix-artifacts.json": 1,
    })
    assert.deepEqual(aggregate.requiredGeneratedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(aggregate.missingGeneratedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(aggregate.requiredGeneratedMatrixNames, {
      "workspace-live-sync-matrix": 1,
    })
    assert.deepEqual(aggregate.missingGeneratedMatrixNames, {
      "remote-agent-runtime-matrix": 1,
    })
    assert.deepEqual(aggregate.requiredGeneratedMatrixRepos, {
      oss: 1,
    })
    assert.deepEqual(aggregate.missingGeneratedMatrixRepos, {
      cloud: 1,
    })
    assert.deepEqual(aggregate.requiredGeneratedValidationSuiteArtifactIndexes, {
      "/tmp/generated-suite/arroba-drill-artifacts.json": 1,
    })
    assert.deepEqual(aggregate.missingGeneratedValidationSuiteArtifactIndexes, {
      "/tmp/generated-suite/missing-artifacts.json": 1,
    })
    assert.deepEqual(aggregate.requiredGeneratedValidationSuiteFailureRoots, {
      "/tmp/generated-suite/failed-run": 1,
    })
    assert.deepEqual(aggregate.missingGeneratedValidationSuiteFailureRoots, {
      "/tmp/generated-suite/missing-run": 1,
    })
    assert.deepEqual(aggregate.providerAccountAliases, {
      "claude=team": 1,
      "codex=work": 2,
      "opencode=zen": 1,
    })
    assert.deepEqual(aggregate.evidenceRepos, {
      cloud: 1,
      oss: 2,
    })
    assert.deepEqual(aggregate.artifactCoverageInputSources, {
      "artifact metadata inputs": 1,
    })
    assert.deepEqual(aggregate.indexes.map((index) => index.source), [
      "one/arroba-drill-artifacts.json",
      "two/arroba-drill-artifacts.json",
    ])
    assert.deepEqual(aggregate.indexes.map((index) => index.runtimeSignals), [
      {
        "provider-run-lifecycle": 1,
        "session-authority": 1,
      },
      {
        "lease-health": 1,
        "session-authority": 1,
      },
    ])
    assert.deepEqual(aggregate.indexes.map((index) => index.runtimeSignalOwners), [
      {
        "kernel-authority": 1,
        "provider-runtime": 1,
      },
      {
        "kernel-authority": 1,
      },
    ])
    assert.deepEqual(aggregate.indexes.map((index) => index.owners), [
      {
        "validation-harness": 1,
      },
      {
        "runtime-network": 1,
        "validation-harness": 1,
      },
    ])
    assert.deepEqual(diagnosticMetadataForDrillArtifactIndexAggregate(aggregate), {
      artifactKinds: "matrix-report,validation-gate,validation-suite-run",
      artifactCoverageInputCount: "1",
      artifactCoverageInputSources: "artifact metadata inputs",
      classifications: "artifact-coverage,matrix-coverage,validation-gate",
      requiredFailureClassifications: "kernel-authority,provider-auth,relay-target-freshness,workspace-live-sync-conflict",
      missingFailureClassifications: "provider-auth",
      exitCriterionStatuses: "dry-run",
      incompleteExitCriterionStatuses: "dry-run",
      evidenceRepos: "cloud,oss",
      generatedEvidenceKinds: "matrix-report,validation-suite-run",
      generatedMatrixArtifactIndexes: "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
      generatedMatrixLimitations: "dry-run-classification-coverage",
      generatedMatrixNames: "workspace-live-sync-matrix",
      generatedMatrixRepos: "oss",
      generatedEvidenceRepos: "cloud,oss",
      generatedValidationSuiteArtifactIndexes: "/tmp/generated-suite/arroba-drill-artifacts.json",
      generatedValidationSuiteFailureRoots: "/tmp/generated-suite/failed-run",
      missingGeneratedEvidenceKinds: "matrix-report",
      missingGeneratedMatrixArtifactIndexes: "/tmp/generated-matrix/missing-matrix-artifacts.json",
      missingGeneratedMatrixLimitations: "dry-run-classification-coverage",
      missingGeneratedMatrixNames: "remote-agent-runtime-matrix",
      missingGeneratedMatrixRepos: "cloud",
      missingRuntimeSignalOwners: "runtime-network",
      missingRuntimeSignals: "relay-target-freshness",
      missingGeneratedValidationSuiteArtifactIndexes: "/tmp/generated-suite/missing-artifacts.json",
      missingGeneratedValidationSuiteFailureRoots: "/tmp/generated-suite/missing-run",
      owners: "runtime-network,validation-harness",
      plannedClassifications: "workspace-live-sync-conflict",
      plannedOwners: "runtime-state",
      providerAccountAliases: "claude=team,codex=work,opencode=zen",
      requiredGeneratedEvidenceKinds: "matrix-report,validation-suite-run",
      requiredGeneratedMatrixArtifactIndexes: "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
      requiredGeneratedMatrixLimitations: "dry-run-classification-coverage",
      requiredGeneratedMatrixNames: "workspace-live-sync-matrix",
      requiredGeneratedMatrixRepos: "oss",
      requiredGeneratedValidationSuiteArtifactIndexes: "/tmp/generated-suite/arroba-drill-artifacts.json",
      requiredGeneratedValidationSuiteFailureRoots: "/tmp/generated-suite/failed-run",
      requiredRuntimeSignalOwners: "kernel-authority,provider-runtime",
      requiredRuntimeSignals: "lease-health,provider-run-lifecycle,session-authority",
      runtimeAuthorityInvariants: "client-render-request,home-session-authority",
      runtimeSignalOwners: "kernel-authority,provider-runtime",
      runtimeSignals: "lease-health,provider-run-lifecycle,session-authority",
      validationPresets: "distributed-runtime,slice-runtime,workspace-live-sync",
    })
    assert.deepEqual(DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS, [
      "runtimeSignals",
      "runtimeSignalOwners",
      "requiredRuntimeSignals",
      "requiredRuntimeSignalOwners",
      "missingRuntimeSignals",
      "missingRuntimeSignalOwners",
      "runtimeAuthorityInvariants",
      "requiredRuntimeAuthorityInvariants",
      "missingRuntimeAuthorityInvariants",
      "coverageAreas",
      "validationPresets",
      "owners",
      "classifications",
      "requiredFailureClassifications",
      "missingFailureClassifications",
      "plannedOwners",
      "plannedClassifications",
      "exitCriterionStatuses",
      "incompleteExitCriterionStatuses",
      "artifactKinds",
      "generatedEvidenceKinds",
      "generatedMatrixArtifactIndexes",
      "generatedMatrixLimitations",
      "generatedMatrixNames",
      "generatedMatrixRepos",
      "generatedEvidenceRepos",
      "generatedValidationSuiteArtifactIndexes",
      "generatedValidationSuiteFailureRoots",
      "requiredGeneratedEvidenceKinds",
      "missingGeneratedEvidenceKinds",
      "requiredGeneratedMatrixArtifactIndexes",
      "missingGeneratedMatrixArtifactIndexes",
      "requiredGeneratedMatrixLimitations",
      "missingGeneratedMatrixLimitations",
      "requiredGeneratedMatrixNames",
      "missingGeneratedMatrixNames",
      "requiredGeneratedMatrixRepos",
      "missingGeneratedMatrixRepos",
      "requiredGeneratedValidationSuiteArtifactIndexes",
      "missingGeneratedValidationSuiteArtifactIndexes",
      "requiredGeneratedValidationSuiteFailureRoots",
      "missingGeneratedValidationSuiteFailureRoots",
      "providerAccountAliases",
      "evidenceRepos",
      "artifactCoverageInputSources",
    ])
    assert.deepEqual(DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS, [
      "schemas",
      ...DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS,
    ])
    for (const key of DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS) {
      assert(Object.hasOwn(aggregate, key), `aggregate should preserve ${key}`)
      assert(aggregate.indexes.every((index) => Object.hasOwn(index, key)), `index summaries should preserve ${key}`)
    }
    assert.doesNotThrow(() => validateDrillArtifactDiagnosticDimensions(aggregate))
    assert.doesNotThrow(() => validateDrillArtifactIndexAggregate(aggregate))
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /indexes=2 artifacts=3/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /runtime_signals: lease-health=1 provider-run-lifecycle=1 session-authority=2/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /runtime_signal_owners: kernel-authority=2 provider-runtime=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /required_runtime_signals: lease-health=1 provider-run-lifecycle=1 session-authority=2/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /missing_runtime_signals: relay-target-freshness=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /runtime_authority_invariants: client-render-request=1 home-session-authority=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /validation_presets: distributed-runtime=2 slice-runtime=1 workspace-live-sync=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /owners: runtime-network=1 validation-harness=2/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /classifications: artifact-coverage=1 matrix-coverage=1 validation-gate=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /required_failure_classifications: kernel-authority=1 provider-auth=1 relay-target-freshness=1 workspace-live-sync-conflict=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /missing_failure_classifications: provider-auth=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /planned_owners: runtime-state=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /planned_classifications: workspace-live-sync-conflict=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /exit_criterion_statuses: dry-run=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /incomplete_exit_criterion_statuses: dry-run=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /generated_evidence_repos: cloud=1 oss=2/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /provider_account_aliases: claude=team=1 codex=work=2 opencode=zen=1/)
    assert.match(formatDrillArtifactIndexAggregateSummary(aggregate), /artifact_coverage_input_count=1/)
  } finally {
    await finalizeDrillArtifacts({ rootDir: root, passed: true })
  }
})
