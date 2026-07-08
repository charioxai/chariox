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

test("summarizes matrix report status and scenario counts", () => {
  const report = matrixReport({
    scenarios: [
      scenario("local", "passed", { runtimeSignals: ["session-authority"] }),
      scenario("remote", "failed", {
        classification: "provider-auth",
        reason: "Token refresh failed: 401",
        runtimeSignals: ["lease-health", "provider-run-lifecycle"],
      }),
      scenario("hetzner", "skipped", { reason: "skipped after previous failure" }),
    ],
  })

  const summary = summarizeDrillMatrixReport(report)

  assert.equal(summary.status, "failed")
  assert.deepEqual(summary.counts, { passed: 1, failed: 1, skipped: 1, dryRun: 0 })
  assert.deepEqual(summary.classifications, { "provider-auth": 1 })
  assert.deepEqual(summary.runtimeSignals, {
    "lease-health": 1,
    "provider-run-lifecycle": 1,
    "session-authority": 1,
  })
  assert.deepEqual(summary.runtimeSignalScenarios, {
    "lease-health": [{ id: "remote", status: "failed" }],
    "provider-run-lifecycle": [{ id: "remote", status: "failed" }],
    "session-authority": [{ id: "local", status: "passed" }],
  })
  assert.equal(drillMatrixReportExitCode([report]), 1)
  assert.equal(drillMatrixReportCompletionExitCode([report]), 1)
})

test("formats failed and skipped scenarios with next actions", () => {
  const report = matrixReport({
    scenarios: [
      scenario("remote", "failed", {
        classification: "provider-account",
        reason: "insufficient balance",
        exitCriteria: ["remote worker executes the selected provider turn", "home observes completion"],
        artifactHints: ["/tmp/arroba-drill-remote", { kind: "manifest", path: ".artifacts/remote.json" }],
        runtimeSignals: ["lease-health", "provider-run-lifecycle"],
      }),
      scenario("cloud", "failed", { classification: "cloud-runtime", reason: "deployment did not become ready" }),
      scenario("hetzner", "skipped", { reason: "skipped after previous failure" }),
    ],
  })

  const text = formatDrillMatrixReportSummary(report, { source: "/tmp/report.json" })

  assert.match(text, /matrix report: test-matrix \(\/tmp\/report\.json\)/)
  assert.match(text, /status=failed scenarios=3 passed=0 failed=2 skipped=1 dry_run=0/)
  assert.match(text, /classifications: cloud-runtime=1 provider-account=1/)
  assert.match(text, /runtime_signals: lease-health=1 provider-run-lifecycle=1/)
  assert.match(text, /runtime_signal_owners: kernel-authority=1 provider-runtime=1/)
  assert.match(text, /- remote classification=provider-account owner=provider-account reason=insufficient balance/)
  assert.match(text, /criteria: remote worker executes the selected provider turn; home observes completion/)
  assert.match(text, /artifacts: \/tmp\/arroba-drill-remote, manifest:\.artifacts\/remote\.json/)
  assert.match(text, /next: check provider quota or billing/)
  assert.match(text, /- cloud classification=cloud-runtime owner=cloud-deployment reason=deployment did not become ready/)
  assert.match(text, /next: inspect Cloud deployment\/control-plane status/)
  assert.match(text, /skipped scenarios: hetzner/)
})

test("formats dry-run reports without failures", () => {
  const report = matrixReport({
    status: "dry-run",
    dryRun: true,
    scenarios: [scenario("local", "dry-run", {
      exitCriteria: ["local runtime path is selected"],
      owner: "cloud-web",
      plannedClassification: "kernel-authority",
      plannedOwner: "kernel-authority",
      plannedNextAction: "inspect session, agent, lease, provider-run, and projection authority state before rerunning the scenario",
    })],
  })

  const text = formatDrillMatrixReportSummary(report)

  assert.match(text, /status=dry-run/)
  assert.equal(report.scenarios[0].owner, "cloud-web")
  assert.match(text, /selected scenario criteria:/)
  assert.match(text, /- local: local runtime path is selected/)
  assert.match(text, /incomplete exit criteria:/)
  assert.match(text, /local\/local:exit-01 status=dry-run owner=cloud-web reason=scenario command was selected but not executed: local runtime path is selected/)
  assert.match(text, /next: run or reconcile incomplete criteria before treating this matrix report as complete/)
  assert.equal(drillMatrixReportExitCode([report]), 0)
  assert.equal(drillMatrixReportCompletionExitCode([report]), 2)
})

test("tracks criterion-level completion evidence", () => {
  const report = matrixReport({
    scenarios: [
      scenario("projection", "passed", {
        exitCriteria: [
          "client projection renders authority state",
          "worker acknowledgement is observed",
        ],
        exitCriteriaEvidence: [
          {
            id: "projection:exit-01",
            criterion: "client projection renders authority state",
            status: "satisfied",
            reason: null,
          },
          {
            id: "projection:exit-02",
            criterion: "worker acknowledgement is observed",
            status: "dry-run",
            reason: "scenario command was selected but not executed",
          },
        ],
      }),
    ],
  })

  const summary = summarizeDrillMatrixReport(report)
  const aggregate = summarizeDrillMatrixReports([report], { sources: ["/tmp/projection-matrix.json"] })

  assert.equal(summary.status, "passed")
  assert.deepEqual(summary.exitCriteria, { "dry-run": 1, satisfied: 1 })
  assert.deepEqual(summary.incompleteExitCriteria, [{
    matrix: "test-matrix",
    source: null,
    scenarioId: "projection",
    id: "projection:exit-02",
    criterion: "worker acknowledgement is observed",
    status: "dry-run",
    reason: "scenario command was selected but not executed",
  }])
  assert.deepEqual(aggregate.exitCriteria, { "dry-run": 1, satisfied: 1 })
  assert.deepEqual(aggregate.incompleteExitCriteria, [{
    matrix: "test-matrix",
    source: "/tmp/projection-matrix.json",
    scenarioId: "projection",
    id: "projection:exit-02",
    criterion: "worker acknowledgement is observed",
    status: "dry-run",
    reason: "scenario command was selected but not executed",
  }])
  assert.equal(drillMatrixReportExitCode([report]), 0)
  assert.equal(drillMatrixReportCompletionExitCode([report]), 2)

  const text = formatDrillMatrixAggregateSummary(aggregate)
  assert.match(text, /exit_criteria: dry-run=1 satisfied=1/)
  assert.match(text, /incomplete exit criteria:/)
  assert.match(text, /test-matrix\/projection\/projection:exit-02 status=dry-run reason=scenario command was selected but not executed source=\/tmp\/projection-matrix\.json: worker acknowledgement is observed/)
})

test("validates incomplete exit criteria independent of object key order", () => {
  assert.doesNotThrow(() => validateDrillMatrixReport(matrixReport({
    status: "dry-run",
    dryRun: true,
    scenarios: [scenario("projection", "dry-run", {
      owner: "cloud-web",
      exitCriteria: ["worker acknowledgement is observed"],
      exitCriteriaEvidence: [{
        id: "projection:exit-01",
        criterion: "worker acknowledgement is observed",
        status: "dry-run",
        reason: "scenario command was selected but not executed",
      }],
    })],
    incompleteExitCriteria: [{
      scenarioId: "projection",
      id: "projection:exit-01",
      owner: "cloud-web",
      status: "dry-run",
      criterion: "worker acknowledgement is observed",
      reason: "scenario command was selected but not executed",
    }],
  })))
})

test("carries ownership diagnostics for incomplete exit criteria", () => {
  const report = matrixReport({
    scenarios: [
      scenario("auth", "failed", {
        classification: "provider-auth",
        reason: "token refresh failed",
        exitCriteria: ["provider account is usable"],
        exitCriteriaEvidence: [{
          id: "auth:exit-01",
          criterion: "provider account is usable",
          status: "failed",
          reason: "token refresh failed",
        }],
      }),
    ],
  })

  const summary = summarizeDrillMatrixReport(report)
  const aggregate = summarizeDrillMatrixReports([report], { sources: ["/tmp/auth-matrix.json"] })

  assert.deepEqual(summary.incompleteExitCriteria, [{
    matrix: "test-matrix",
    source: null,
    scenarioId: "auth",
    id: "auth:exit-01",
    criterion: "provider account is usable",
    status: "failed",
    reason: "token refresh failed",
    owner: "provider-account",
    classification: "provider-auth",
    nextAction: "refresh provider login for the profile used by this drill, then rerun the scenario",
  }])
  assert.deepEqual(aggregate.incompleteExitCriteria, [{
    matrix: "test-matrix",
    source: "/tmp/auth-matrix.json",
    scenarioId: "auth",
    id: "auth:exit-01",
    criterion: "provider account is usable",
    status: "failed",
    reason: "token refresh failed",
    owner: "provider-account",
    classification: "provider-auth",
    nextAction: "refresh provider login for the profile used by this drill, then rerun the scenario",
  }])
  assert.match(
    formatDrillMatrixAggregateSummary(aggregate),
    /test-matrix\/auth\/auth:exit-01 status=failed owner=provider-account classification=provider-auth reason=token refresh failed source=\/tmp\/auth-matrix\.json: provider account is usable next=refresh provider login/,
  )
})

