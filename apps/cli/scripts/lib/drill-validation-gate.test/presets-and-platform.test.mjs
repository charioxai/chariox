import {
  assert,
  describeDrillValidationGatePresets,
  distributedStateHealthPartialMatrixReport,
  drillValidationGateExitCode,
  findDrillValidationGateAggregatePaths,
  findDrillValidationGateReportPaths,
  formatDrillValidationGateAggregateSummary,
  formatDrillValidationGateSummary,
  mkdir,
  mkdtemp,
  os,
  path,
  readDrillValidationGateAggregate,
  readDrillValidationGateReport,
  rm,
  runDrillValidationGate,
  runtimeAuthorityMatrixReportFixtures,
  summarizeDrillValidationGateReports,
  test,
  validateDrillValidationGateAggregate,
  validateDrillValidationGateReport,
  writeDrillArtifactIndex,
  writeDrillPlatformBundle,
  writeFile,
  workspaceLiveSyncRequiredScenarioIds,
  emptyArtifactCoverageSummary,
  matrixReport,
  platformValidationPresetSummaries,
  rewriteArtifactIndexCreatedAt,
  scenario,
  workspaceLiveSyncRequiredScenarios,
  writeFailureManifest,
  writeMatrixReport,
} from '../drill-validation-gate.test-support.mjs'

test("describes validation gate presets", () => {
  const presets = describeDrillValidationGatePresets()
  assert.deepEqual(presets.map((preset) => preset.name), ["distributed-runtime", "distributed-state-health", "native-provider-tui", "remote-agent-runtime", "remote-home-extension", "runtime-authority", "slice-runtime", "workspace-live-sync"])
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["distributed-runtime"] })[0].requiredMatrices,
    ["browser-terminal-resilience-matrix", "cloud-slice-runtime-matrix", "native-provider-tui-matrix", "remote-agent-runtime-matrix", "remote-home-extension-matrix", "runtime-resilience-chaos-matrix", "slice-runtime-matrix", "workspace-live-sync-matrix"],
  )
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["runtime-authority"] })[0].requiredRuntimeSignals,
    ["agent-lifecycle", "client-projection-health", "lease-health", "permission-interaction", "provider-run-lifecycle", "runtime-projection-health", "runtime-transition-audit", "session-authority"],
  )
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["distributed-state-health"] })[0].requiredMatrices,
    ["browser-terminal-resilience-matrix", "cloud-slice-runtime-matrix", "remote-agent-runtime-matrix", "remote-home-extension-matrix", "runtime-resilience-chaos-matrix", "slice-runtime-matrix", "workspace-live-sync-matrix"],
  )
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["workspace-live-sync"] })[0].requiredMatrixClassifications,
    ["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"],
  )
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["native-provider-tui"] })[0].requiredScenarios,
    ["local-native-tui", "permission-visibility", "remote-native-tui", "slice-native-tui", "transcript-parity"],
  )
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["remote-agent-runtime"] })[0].requiredMatrixClassifications,
    ["kernel-authority", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "ui-client-projection", "worker-execution"],
  )
  assert.deepEqual(
    describeDrillValidationGatePresets({ names: ["slice-runtime"] })[0].requiredProviders,
    ["claude", "codex", "opencode"],
  )
  assert.throws(
    () => describeDrillValidationGatePresets({ names: ["workspace-live-synch"] }),
    /unknown validation gate preset: workspace-live-synch/,
  )
})

test("passes with valid platform bundle and complete matrix reports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    const matrixRoot = path.join(rootDir, "matrices")
    await writeDrillPlatformBundle(bundleDir)
    await writeMatrixReport(path.join(matrixRoot, "matrix.json"), matrixReport())

    const report = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      matrixRoots: [matrixRoot],
      requireComplete: true,
    })

    assert.equal(report.schema, "arroba.drill.validation_gate.v1")
    assert.equal(report.status, "passed")
    assert.equal(drillValidationGateExitCode(report), 0)
    assert.equal(report.checks.configuration.status, "passed")
    assert.equal(report.checks.platformBundle.status, "passed")
    assert.deepEqual(report.checks.platformBundle.validationSuite, {
      testCount: 102,
      coverageAreas: [
        { id: "distributed-observability", testCount: 5 },
        { id: "artifact-contracts", testCount: 20 },
        { id: "failure-diagnostics", testCount: 4 },
        { id: "matrix-validation", testCount: 37 },
        { id: "runtime-fixtures", testCount: 34 },
        { id: "suite-contract", testCount: 2 },
      ],
      validationPresets: platformValidationPresetSummaries(),
    })
    assert.equal(report.checks.platformBundle.failureTaxonomy.drill.includes("kernel-authority"), true)
    assert.equal(report.checks.platformBundle.failureTaxonomy.scenario.includes("remote-extension-sync"), true)
    assert.equal(report.checks.matrices.status, "passed")
    assert.equal(report.checks.failures.status, "skipped")
    assert.deepEqual(report.nextActions, [])
    assert.doesNotThrow(() => validateDrillValidationGateReport(report))
    assert.match(formatDrillValidationGateSummary(report), /status=passed/)
    assert.match(formatDrillValidationGateSummary(report), /platform_validation_suite_tests=102 coverage=distributed-observability:5/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates platform bundle validation suite coverage areas", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)

    const pass = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      requiredPlatformCoverageAreas: ["runtime-fixtures,matrix-validation"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.platformBundle.requiredCoverageAreas, ["matrix-validation", "runtime-fixtures"])
    assert.deepEqual(pass.checks.platformBundle.missingCoverageAreas, [])
    assert.match(formatDrillValidationGateSummary(pass), /platform_required_coverage_areas=matrix-validation,runtime-fixtures missing=none/)

    const fail = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      requiredPlatformCoverageAreas: ["runtime-fixtures", "hosted-cloud-drills"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.platformBundle.missingCoverageAreas, ["hosted-cloud-drills"])
    assert.match(fail.checks.platformBundle.error, /missing platform coverage areas: hosted-cloud-drills/)
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [
      {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: "provide a drill platform bundle covering: hosted-cloud-drills",
      },
      {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: "rebuild the drill platform bundle and verify it before using collected artifacts as evidence",
      },
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates platform bundle failure taxonomy classifications", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)

    const pass = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      requiredFailureClassifications: ["kernel-authority,remote-extension-sync", "workspace-live-sync-conflict"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.platformBundle.requiredFailureClassifications, [
      "kernel-authority",
      "remote-extension-sync",
      "workspace-live-sync-conflict",
    ])
    assert.deepEqual(pass.checks.platformBundle.missingFailureClassifications, [])
    assert.match(
      formatDrillValidationGateSummary(pass),
      /platform_required_failure_classifications=kernel-authority,remote-extension-sync,workspace-live-sync-conflict missing=none/,
    )
    assert.match(formatDrillValidationGateSummary(pass), /platform_failure_taxonomy=drill:\d+ scenario:\d+/)

    await assert.rejects(
      () => runDrillValidationGate({
        platformBundleDir: bundleDir,
        requiredFailureClassifications: ["kernel-authority", "kernel-authorities"],
      }),
      /unknown required failure classification: kernel-authorities/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates platform bundle runtime signal coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)

    const pass = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      requiredRuntimeSignals: ["session-authority,lease-health", "workspace-live-sync-state"],
      requiredRuntimeSignalOwners: ["kernel-authority", "worker-kernel"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.checks.platformBundle.requiredRuntimeSignals, [
      "lease-health",
      "session-authority",
      "workspace-live-sync-state",
    ])
    assert.deepEqual(pass.checks.platformBundle.missingRuntimeSignals, [])
    assert.deepEqual(pass.checks.platformBundle.requiredRuntimeSignalOwners, ["kernel-authority", "worker-kernel"])
    assert.deepEqual(pass.checks.platformBundle.missingRuntimeSignalOwners, [])
    assert.match(
      formatDrillValidationGateSummary(pass),
      /platform_required_runtime_signals=lease-health,session-authority,workspace-live-sync-state missing=none/,
    )
    assert.match(
      formatDrillValidationGateSummary(pass),
      /platform_required_runtime_signal_owners=kernel-authority,worker-kernel missing=none/,
    )

    const fail = await runDrillValidationGate({
      requiredRuntimeSignals: ["lease-health"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.checks.platformBundle.missingRuntimeSignals, ["lease-health"])
    assert.deepEqual(fail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [
      {
        owner: "kernel-authority",
        classification: "runtime-signal-coverage",
        nextAction: "add runtime-signal contract coverage for lease-health owned by kernel-authority to the drill platform bundle",
      },
      {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: "provide a drill platform bundle covering runtime signals: lease-health",
      },
      {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: "rebuild the drill platform bundle and verify it before using collected artifacts as evidence",
      },
    ])

    const ownerFail = await runDrillValidationGate({
      requiredRuntimeSignalOwners: ["worker-kernel"],
    })
    assert.equal(ownerFail.status, "failed")
    assert.deepEqual(ownerFail.checks.platformBundle.missingRuntimeSignalOwners, ["worker-kernel"])
    assert.deepEqual(ownerFail.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [
      {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: "provide a drill platform bundle covering runtime signal owners: worker-kernel",
      },
      {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: "rebuild the drill platform bundle and verify it before using collected artifacts as evidence",
      },
      {
        owner: "worker-kernel",
        classification: "runtime-signal-coverage",
        nextAction: "add runtime-signal contract coverage owned by worker-kernel to the drill platform bundle",
      },
    ])

    await assert.rejects(
      () => runDrillValidationGate({
        requiredRuntimeSignals: ["workspace-live-synch-state"],
      }),
      /unknown required runtime signal: workspace-live-synch-state/,
    )
    await assert.rejects(
      () => runDrillValidationGate({
        requiredRuntimeSignalOwners: ["worker-kenrel"],
      }),
      /unknown required runtime signal owner: worker-kenrel/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("applies validation gate requirement presets", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    const reportPath = path.join(rootDir, "workspace-live-sync.json")
    await writeDrillPlatformBundle(bundleDir)
    await writeMatrixReport(reportPath, matrixReport({
      matrix: "workspace-live-sync-matrix",
      metadata: {
        deploymentPresets: "hetzner,local,same-host-remote,self-hosted-relay",
        providers: "codex,opencode",
      },
      scenarios: workspaceLiveSyncRequiredScenarios(),
    }))

    const report = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      matrixReports: [reportPath],
      presets: ["workspace-live-sync"],
      requiredDeploymentPresets: ["local"],
      requiredProviders: ["codex"],
      requiredScenarios: ["local-managed-codex"],
    })

    assert.equal(report.status, "passed")
    assert.deepEqual(report.presets, ["workspace-live-sync"])
    assert.deepEqual(report.checks.platformBundle.requiredFailureClassifications, [
      "kernel-authority",
      "relay-target-freshness",
      "workspace-live-sync-conflict",
    ])
    assert.deepEqual(report.checks.matrices.requiredMatrices, ["workspace-live-sync-matrix"])
    assert.deepEqual(report.checks.matrices.requiredMatrixClassifications, [
      "kernel-authority",
      "relay-target-freshness",
      "workspace-live-sync-conflict",
    ])
    assert.deepEqual(report.checks.matrices.requiredMatrixRuntimeSignals, [
      "relay-target-freshness",
      "session-authority",
      "workspace-live-sync-state",
    ])
    assert.deepEqual(report.checks.matrices.requiredDeploymentPresets, ["hetzner", "local", "same-host-remote", "self-hosted-relay"])
    assert.deepEqual(report.checks.matrices.requiredProviders, ["codex", "opencode"])
    assert.deepEqual(report.checks.matrices.requiredScenarios, workspaceLiveSyncRequiredScenarioIds())
    assert.match(formatDrillValidationGateSummary(report), /presets=workspace-live-sync/)
    assert.match(formatDrillValidationGateSummary(report), /matrix_required_classifications=kernel-authority,relay-target-freshness,workspace-live-sync-conflict missing=none/)
    const aggregate = summarizeDrillValidationGateReports([report], { sources: ["workspace-live-sync.json"] })
    assert.deepEqual(aggregate.coverage.presets, { "workspace-live-sync": 1 })
    assert.deepEqual(aggregate.reports[0].presets, ["workspace-live-sync"])
    assert.match(formatDrillValidationGateAggregateSummary(aggregate), /presets: workspace-live-sync=1/)
    const requiredAggregate = summarizeDrillValidationGateReports([report], {
      sources: ["workspace-live-sync.json"],
      requiredPresets: ["workspace-live-sync"],
      requiredPlatformCoverageAreas: ["matrix-validation"],
      requiredFailureClassifications: ["kernel-authority"],
      requiredMatrices: ["workspace-live-sync-matrix"],
      requiredMatrixClassifications: ["workspace-live-sync-conflict"],
      requiredMatrixRuntimeSignals: ["workspace-live-sync-state"],
      requiredDeploymentPresets: ["local"],
      requiredProviders: ["codex"],
      requiredScenarios: ["local-managed-codex"],
    })
    assert.equal(requiredAggregate.status, "passed")
    assert.deepEqual(requiredAggregate.requiredPresets, ["workspace-live-sync"])
    assert.deepEqual(requiredAggregate.missingPresets, [])
    assert.deepEqual(requiredAggregate.missingPlatformCoverageAreas, [])
    assert.deepEqual(requiredAggregate.missingFailureClassifications, [])
    assert.deepEqual(requiredAggregate.missingMatrices, [])
    assert.deepEqual(requiredAggregate.missingMatrixClassifications, [])
    assert.deepEqual(requiredAggregate.missingMatrixRuntimeSignals, [])
    assert.deepEqual(requiredAggregate.missingDeploymentPresets, [])
    assert.deepEqual(requiredAggregate.missingProviders, [])
    assert.deepEqual(requiredAggregate.missingScenarios, [])
    assert.match(formatDrillValidationGateAggregateSummary(requiredAggregate), /required_presets=workspace-live-sync missing=none/)
    assert.match(formatDrillValidationGateAggregateSummary(requiredAggregate), /required_providers=codex missing=none/)
    const missingAggregate = summarizeDrillValidationGateReports([report], {
      sources: ["workspace-live-sync.json"],
      requiredPresets: ["remote-home-extension"],
      requiredProviders: ["claude"],
    })
    assert.equal(missingAggregate.status, "failed")
    assert.deepEqual(missingAggregate.requiredPresets, ["remote-home-extension"])
    assert.deepEqual(missingAggregate.missingPresets, ["remote-home-extension"])
    assert.deepEqual(missingAggregate.requiredProviders, ["claude"])
    assert.deepEqual(missingAggregate.missingProviders, ["claude"])
    assert.deepEqual(missingAggregate.nextActions.map(({ classification, nextAction }) => ({ classification, nextAction })), [{
      classification: "matrix-coverage",
      nextAction: "provide validation gate reports requiring providers: claude",
    }, {
      classification: "validation-gate",
      nextAction: "provide validation gate reports for presets: remote-home-extension",
    }])
    assert.match(formatDrillValidationGateAggregateSummary(missingAggregate), /required_presets=remote-home-extension missing=remote-home-extension/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("runtime authority preset gates shared kernel-owned path evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    const matrixReports = []
    for (const fixture of runtimeAuthorityMatrixReportFixtures()) {
      const reportPath = path.join(rootDir, fixture.fileName)
      await writeMatrixReport(reportPath, fixture.report)
      matrixReports.push(reportPath)
    }

    const report = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      matrixReports,
      presets: ["runtime-authority"],
    })

    assert.equal(report.status, "passed")
    assert.deepEqual(report.presets, ["runtime-authority"])
    assert.deepEqual(report.checks.platformBundle.requiredRuntimeSignals, [
      "agent-lifecycle",
      "client-projection-health",
      "lease-health",
      "permission-interaction",
      "provider-run-lifecycle",
      "runtime-projection-health",
      "runtime-transition-audit",
      "session-authority",
    ])
    assert.deepEqual(report.checks.matrices.requiredMatrices, [
      "native-provider-tui-matrix",
      "remote-agent-runtime-matrix",
      "slice-runtime-matrix",
    ])
    assert.deepEqual(report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.checks.matrices.missingMatrixRuntimeSignals, [])
    assert.deepEqual(report.checks.matrices.missingScenarios, [])
    assert.deepEqual(report.nextActions, [])
    assert.match(formatDrillValidationGateSummary(report), /presets=runtime-authority/)
    assert.match(formatDrillValidationGateSummary(report), /matrix_required_runtime_signals=agent-lifecycle,client-projection-health,lease-health,permission-interaction,provider-run-lifecycle,runtime-projection-health,session-authority missing=none/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed state health preset reports owner-routed missing diagnostics", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    const reportPath = path.join(rootDir, "remote-agent-runtime.json")
    await writeDrillPlatformBundle(bundleDir)
    await writeMatrixReport(reportPath, distributedStateHealthPartialMatrixReport())

    const report = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      matrixReports: [reportPath],
      presets: ["distributed-state-health"],
    })

    assert.equal(report.status, "failed")
    assert.deepEqual(report.presets, ["distributed-state-health"])
    assert.deepEqual(report.checks.matrices.missingMatrices, [
      "browser-terminal-resilience-matrix",
      "cloud-slice-runtime-matrix",
      "remote-home-extension-matrix",
      "runtime-resilience-chaos-matrix",
      "slice-runtime-matrix",
      "workspace-live-sync-matrix",
    ])
    assert.deepEqual(report.checks.matrices.missingMatrixRuntimeSignals, [
      "home-extension-manifest-sync",
      "relay-target-freshness",
      "runtime-projection-health",
      "slice-auth-state",
      "slice-runtime-state",
      "workspace-live-sync-state",
    ])
    assert.equal(report.nextActions.some((action) =>
      action.owner === "runtime-network"
        && action.classification === "runtime-signal-coverage"
        && action.nextAction.includes("relay-target-freshness owned by runtime-network")
    ), true)
    assert.equal(report.nextActions.some((action) =>
      action.owner === "provider-account"
        && action.classification === "runtime-signal-coverage"
        && action.nextAction.includes("slice-auth-state owned by provider-account")
    ), true)
    assert.equal(report.nextActions.some((action) =>
      action.owner === "runtime-state"
        && action.classification === "runtime-signal-coverage"
        && action.nextAction.includes("workspace-live-sync-state owned by runtime-state")
    ), true)
    assert.match(formatDrillValidationGateSummary(report), /matrix_required_runtime_signals=home-extension-manifest-sync,lease-health,provider-run-lifecycle,relay-target-freshness,runtime-projection-health,slice-auth-state,slice-runtime-state,workspace-live-sync-state missing=home-extension-manifest-sync,relay-target-freshness,runtime-projection-health,slice-auth-state,slice-runtime-state,workspace-live-sync-state/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("suppresses only preset-derived matrix classification requirements", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "workspace-live-sync.json")
    await writeMatrixReport(reportPath, matrixReport({
      matrix: "workspace-live-sync-matrix",
      metadata: {
        deploymentPresets: "local",
        providers: "codex",
      },
      scenarios: [
        scenario("local-managed-codex", "passed", {
          classification: null,
          runtimeSignals: ["session-authority", "workspace-live-sync-state"],
        }),
      ],
    }))

    const suppressed = await runDrillValidationGate({
      matrixReports: [reportPath],
      presets: ["workspace-live-sync"],
      suppressedPresetRequirements: ["requiredMatrixClassifications"],
    })
    assert.deepEqual(suppressed.presets, ["workspace-live-sync"])
    assert.deepEqual(suppressed.checks.matrices.requiredMatrixClassifications, [])
    assert.deepEqual(suppressed.checks.matrices.missingMatrixClassifications, [])

    const explicit = await runDrillValidationGate({
      matrixReports: [reportPath],
      presets: ["workspace-live-sync"],
      requiredMatrixClassifications: ["kernel-authority"],
      suppressedPresetRequirements: ["requiredMatrixClassifications"],
    })
    assert.deepEqual(explicit.checks.matrices.requiredMatrixClassifications, ["kernel-authority"])
    assert.deepEqual(explicit.checks.matrices.missingMatrixClassifications, ["kernel-authority"])

    await assert.rejects(
      () => runDrillValidationGate({ suppressedPresetRequirements: ["requiredMatrices"] }),
      /unsupported suppressed preset requirement "requiredMatrices"/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("slice runtime preset accepts hosted Cloud evidence from a separate matrix report", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    const sliceRuntimeReport = path.join(rootDir, "slice-runtime.json")
    const hostedSliceReport = path.join(rootDir, "cloud-slice-runtime.json")
    await writeDrillPlatformBundle(bundleDir)
    await writeMatrixReport(sliceRuntimeReport, matrixReport({
      matrix: "slice-runtime-matrix",
      metadata: {
        deploymentPresets: "local,self-hosted-relay",
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("slice-lifecycle", "passed", { classification: "slice-runtime", runtimeSignals: ["slice-runtime-state"] }),
        scenario("provider-auth", "passed", { classification: "slice-auth", runtimeSignals: ["slice-auth-state"] }),
        scenario("session-start", "passed", { classification: "kernel-authority", runtimeSignals: ["session-authority"] }),
        scenario("agent-reuse", "passed", { classification: "worker-execution", runtimeSignals: ["agent-lifecycle"] }),
        scenario("ui-projection", "passed", { classification: "ui-client-projection", runtimeSignals: ["client-projection-health", "runtime-projection-health"] }),
        scenario("docker-browser-state", "passed", { classification: "docker-runtime", runtimeSignals: ["slice-runtime-state"] }),
      ],
    }))
    await writeMatrixReport(hostedSliceReport, matrixReport({
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("hosted-slice-browser-e2e", "passed", { classification: "ui-client-projection", runtimeSignals: ["client-projection-health", "runtime-projection-health"] }),
        scenario("hosted-vault-view-slice", "passed", { classification: "kernel-authority", runtimeSignals: ["provider-run-lifecycle", "session-authority"] }),
      ],
    }))

    const report = await runDrillValidationGate({
      platformBundleDir: bundleDir,
      matrixReports: [sliceRuntimeReport, hostedSliceReport],
      presets: ["slice-runtime"],
      requireComplete: true,
    })

    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.matrices.requiredMatrices, ["cloud-slice-runtime-matrix", "slice-runtime-matrix"])
    assert.deepEqual(report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.checks.matrices.requiredDeploymentPresets, ["hosted-cloud", "local", "self-hosted-relay"])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.deepEqual(report.checks.matrices.requiredProviders, ["claude", "codex", "opencode"])
    assert.deepEqual(report.checks.matrices.missingProviders, [])
    assert.deepEqual(report.checks.matrices.missingMatrixRuntimeSignals, [])
    assert.deepEqual(report.checks.matrices.requiredScenarios, ["agent-reuse", "provider-auth", "session-start", "slice-lifecycle", "ui-projection"])
    assert.deepEqual(report.checks.matrices.missingScenarios, [])
    assert.match(formatDrillValidationGateSummary(report), /presets=slice-runtime/)
    assert.match(formatDrillValidationGateSummary(report), /matrix_required_deployment_presets=hosted-cloud,local,self-hosted-relay missing=none/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects unknown validation gate presets", async () => {
  await assert.rejects(
    () => runDrillValidationGate({
      presets: ["workspace-live-synch"],
    }),
    /unknown validation gate preset: workspace-live-synch/,
  )
})

test("fails platform coverage requirements without a platform bundle", async () => {
  const report = await runDrillValidationGate({
    requiredPlatformCoverageAreas: ["runtime-fixtures"],
  })

  assert.equal(report.status, "failed")
  assert.equal(report.checks.platformBundle.status, "failed")
  assert.equal(report.checks.platformBundle.error, "no platform bundle provided")
  assert.deepEqual(report.checks.platformBundle.missingCoverageAreas, ["runtime-fixtures"])
})

test("fails platform failure classification requirements without a platform bundle", async () => {
  const report = await runDrillValidationGate({
    requiredFailureClassifications: ["kernel-authority", "remote-extension-sync"],
  })

  assert.equal(report.status, "failed")
  assert.equal(report.checks.platformBundle.status, "failed")
  assert.equal(report.checks.platformBundle.error, "no platform bundle provided")
  assert.deepEqual(report.checks.platformBundle.missingFailureClassifications, ["kernel-authority", "remote-extension-sync"])
  assert.deepEqual(report.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [
    {
      owner: "validation-harness",
      classification: "platform-bundle",
      nextAction: "provide a drill platform bundle covering failure classifications: kernel-authority, remote-extension-sync",
    },
    {
      owner: "validation-harness",
      classification: "platform-bundle",
      nextAction: "rebuild the drill platform bundle and verify it before using collected artifacts as evidence",
    },
  ])
})

test("rejects unknown required failure classifications", async () => {
  await assert.rejects(
    () => runDrillValidationGate({
      requiredFailureClassifications: ["kernel-authority", "remote-extension-synch"],
    }),
    /unknown required failure classification: remote-extension-synch/,
  )
})
