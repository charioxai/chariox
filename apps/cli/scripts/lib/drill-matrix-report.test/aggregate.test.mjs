import {
  assert,
  drillMatrixReportCompletionExitCode,
  drillMatrixReportExitCode,
  findDrillMatrixReportPaths,
  formatDrillMatrixAggregateSummary,
  formatDrillMatrixReportSummary,
  mkdtemp,
  os,
  path,
  readDrillMatrixReport,
  rm,
  summarizeDrillMatrixReport,
  summarizeDrillMatrixReports,
  test,
  validateDrillMatrixAggregate,
  validateDrillMatrixReport,
  writeFile,
  matrixReport,
  scenario,
  writeFileWithDir,
} from '../drill-matrix-report.test-support.mjs'

test("aggregates multiple matrix reports for CI", () => {
  const failed = matrixReport({
    matrix: "remote",
    metadata: {
      deploymentPresets: "local,self-hosted-relay,hetzner",
      providers: "codex,opencode",
    },
    scenarios: [
      scenario("local", "passed", { runtimeSignals: ["session-authority"] }),
      scenario("remote", "failed", {
        classification: "provider-auth",
        reason: "expired token",
        artifactHints: ["/tmp/arroba-drill-remote", { kind: "manifest", path: ".artifacts/remote.json" }],
        runtimeSignals: ["lease-health", "provider-run-lifecycle"],
      }),
      scenario("hetzner", "skipped", { reason: "skipped after previous failure" }),
    ],
  })
  const dryRun = matrixReport({
    matrix: "workspace",
    status: "dry-run",
    dryRun: true,
    durationMs: 25,
    metadata: {
      deploymentPresets: "hosted-cloud",
      providers: "claude",
    },
    scenarios: [scenario("tracked", "dry-run", {
      runtimeSignals: ["workspace-live-sync-state"],
      plannedClassification: "workspace-live-sync-conflict",
      plannedOwner: "runtime-state",
      plannedNextAction: "inspect workspace live sync status, conflicts, and preserved file snapshots; reconcile the conflict, then rerun the scenario",
    })],
  })

  const aggregate = summarizeDrillMatrixReports([failed, dryRun], {
    sources: ["/tmp/remote-matrix.json", "/tmp/workspace-matrix.json"],
  })

  assert.equal(aggregate.schema, "arroba.drill.matrix.aggregate.v1")
  assert.equal(aggregate.status, "failed")
  assert.deepEqual(aggregate.totals, {
    reports: 2,
    scenarios: 4,
    passed: 1,
    failed: 1,
    skipped: 1,
    dryRun: 1,
    durationMs: 1025,
  })
  assert.deepEqual(aggregate.classifications, { "provider-auth": 1 })
  assert.deepEqual(aggregate.runtimeSignals, {
    "lease-health": 1,
    "provider-run-lifecycle": 1,
    "session-authority": 1,
    "workspace-live-sync-state": 1,
  })
  assert.deepEqual(aggregate.runtimeSignalOwners, {
    "kernel-authority": 2,
    "provider-runtime": 1,
    "runtime-state": 1,
  })
  assert.deepEqual(aggregate.runtimeSignalScenarios, {
    "lease-health": [{
      matrix: "remote",
      source: "/tmp/remote-matrix.json",
      id: "remote",
      status: "failed",
    }],
    "provider-run-lifecycle": [{
      matrix: "remote",
      source: "/tmp/remote-matrix.json",
      id: "remote",
      status: "failed",
    }],
    "session-authority": [{
      matrix: "remote",
      source: "/tmp/remote-matrix.json",
      id: "local",
      status: "passed",
    }],
    "workspace-live-sync-state": [{
      matrix: "workspace",
      source: "/tmp/workspace-matrix.json",
      id: "tracked",
      status: "dry-run",
    }],
  })
  assert.deepEqual(aggregate.matrixNames, {
    remote: 1,
    workspace: 1,
  })
  assert.deepEqual(aggregate.deploymentPresets, {
    hetzner: 1,
    "hosted-cloud": 1,
    local: 1,
    "self-hosted-relay": 1,
  })
  assert.deepEqual(aggregate.providers, {
    claude: 1,
    codex: 1,
    opencode: 1,
  })
  assert.deepEqual(aggregate.scenarioIds, {
    hetzner: 1,
    local: 1,
    remote: 1,
    tracked: 1,
  })
  assert.deepEqual(aggregate.exitCriteria, {})
  assert.deepEqual(aggregate.owners, { "provider-account": 1 })
  assert.deepEqual(aggregate.nextActions.map((action) => ({
    owner: action.owner,
    classification: action.classification,
    count: action.count,
    sourceDetails: action.sourceDetails,
  })), [
    {
      owner: "provider-account",
      classification: "provider-auth",
      count: 1,
      sourceDetails: [{
        source: "remote/remote",
        matrix: "remote",
        scenarioId: "remote",
        reportPath: "/tmp/remote-matrix.json",
      }],
    },
  ])
  assert.deepEqual(aggregate.plannedNextActions, [{
    owner: "runtime-state",
    classification: "workspace-live-sync-conflict",
    plannedNextAction: "inspect workspace live sync status, conflicts, and preserved file snapshots; reconcile the conflict, then rerun the scenario",
    count: 1,
    sourceDetails: [{
      source: "workspace/tracked",
      matrix: "workspace",
      scenarioId: "tracked",
      reportPath: "/tmp/workspace-matrix.json",
    }],
  }])
  assert.deepEqual(aggregate.reports.map((report) => ({ matrix: report.matrix, source: report.source })), [
    { matrix: "remote", source: "/tmp/remote-matrix.json" },
    { matrix: "workspace", source: "/tmp/workspace-matrix.json" },
  ])
  assert.deepEqual(aggregate.reports.map((report) => ({
    matrix: report.matrix,
    deploymentPresets: report.deploymentPresets,
    providers: report.providers,
    scenarioIds: report.scenarioIds,
  })), [
    { matrix: "remote", deploymentPresets: ["hetzner", "local", "self-hosted-relay"], providers: ["codex", "opencode"], scenarioIds: ["local", "remote", "hetzner"] },
    { matrix: "workspace", deploymentPresets: ["hosted-cloud"], providers: ["claude"], scenarioIds: ["tracked"] },
  ])
  assert.deepEqual(aggregate.failedScenarios, [{
    matrix: "remote",
    source: "/tmp/remote-matrix.json",
    id: "remote",
    classification: "provider-auth",
    owner: "provider-account",
    reason: "expired token",
    artifactHints: ["/tmp/arroba-drill-remote", "manifest:.artifacts/remote.json"],
    nextAction: "refresh provider login for the profile used by this drill, then rerun the scenario",
  }])
  assert.deepEqual(aggregate.skippedScenarios, [{
    matrix: "remote",
    source: "/tmp/remote-matrix.json",
    id: "hetzner",
    reason: "skipped after previous failure",
  }])
  assert.deepEqual(aggregate.incompleteScenarios, [
    {
      matrix: "remote",
      source: "/tmp/remote-matrix.json",
      id: "hetzner",
      status: "skipped",
      reason: "skipped after previous failure",
    },
    {
      matrix: "workspace",
      source: "/tmp/workspace-matrix.json",
      id: "tracked",
      status: "dry-run",
      reason: null,
      plannedClassification: "workspace-live-sync-conflict",
      plannedOwner: "runtime-state",
      plannedNextAction: "inspect workspace live sync status, conflicts, and preserved file snapshots; reconcile the conflict, then rerun the scenario",
    },
  ])

  const text = formatDrillMatrixAggregateSummary(aggregate)
  assert.match(text, /matrix aggregate:/)
  assert.match(text, /status=failed reports=2 scenarios=4 passed=1 failed=1 skipped=1 dry_run=1/)
  assert.match(text, /- remote\/remote classification=provider-auth owner=provider-account reason=expired token source=\/tmp\/remote-matrix.json/)
  assert.match(text, /artifacts: \/tmp\/arroba-drill-remote, manifest:\.artifacts\/remote\.json/)
  assert.match(text, /owners: provider-account=1/)
  assert.match(text, /matrix_names: remote=1 workspace=1/)
  assert.match(text, /deployment_presets: hetzner=1 hosted-cloud=1 local=1 self-hosted-relay=1/)
  assert.match(text, /providers: claude=1 codex=1 opencode=1/)
  assert.match(text, /scenario_ids: hetzner=1 local=1 remote=1 tracked=1/)
  assert.match(text, /runtime_signals: lease-health=1 provider-run-lifecycle=1 session-authority=1 workspace-live-sync-state=1/)
  assert.match(text, /runtime_signal_owners: kernel-authority=2 provider-runtime=1 runtime-state=1/)
  assert.match(text, /runtime_signal_sources:/)
  assert.match(text, /- lease-health: remote\/remote\(failed\) source=\/tmp\/remote-matrix\.json/)
  assert.match(text, /- workspace-live-sync-state: workspace\/tracked\(dry-run\) source=\/tmp\/workspace-matrix\.json/)
  assert.match(text, /next actions:/)
  assert.match(text, /owner=provider-account classification=provider-auth count=1: refresh provider login/)
  assert.match(text, /sources: remote\/remote report=\/tmp\/remote-matrix\.json/)
  assert.match(text, /planned next actions:/)
  assert.match(text, /owner=runtime-state classification=workspace-live-sync-conflict count=1: inspect workspace live sync status/)
  assert.match(text, /sources: workspace\/tracked report=\/tmp\/workspace-matrix\.json/)
  assert.match(text, /next: refresh provider login/)
  assert.match(text, /incomplete scenarios:/)
  assert.match(text, /- remote\/hetzner status=skipped reason=skipped after previous failure source=\/tmp\/remote-matrix.json/)
  assert.match(text, /- workspace\/tracked status=dry-run source=\/tmp\/workspace-matrix.json planned_owner=runtime-state planned_classification=workspace-live-sync-conflict planned_next=inspect workspace live sync status/)
})

