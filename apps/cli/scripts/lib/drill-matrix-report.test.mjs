import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  drillMatrixReportCompletionExitCode,
  drillMatrixReportExitCode,
  findDrillMatrixReportPaths,
  formatDrillMatrixAggregateSummary,
  formatDrillMatrixReportSummary,
  readDrillMatrixReport,
  summarizeDrillMatrixReport,
  summarizeDrillMatrixReports,
  validateDrillMatrixAggregate,
  validateDrillMatrixReport,
} from "./drill-matrix-report.mjs"

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
    })],
  })

  const text = formatDrillMatrixReportSummary(report)

  assert.match(text, /status=dry-run/)
  assert.equal(report.scenarios[0].owner, "cloud-web")
  assert.match(text, /selected scenario criteria:/)
  assert.match(text, /- local: local runtime path is selected/)
  assert.match(text, /incomplete exit criteria:/)
  assert.match(text, /local\/local:exit-01 status=dry-run reason=scenario command was selected but not executed: local runtime path is selected/)
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
    scenarios: [scenario("tracked", "dry-run", { runtimeSignals: ["workspace-live-sync-state"] })],
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
  })), [
    { owner: "provider-account", classification: "provider-auth", count: 1 },
  ])
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
  assert.match(text, /next: refresh provider login/)
  assert.match(text, /incomplete scenarios:/)
  assert.match(text, /- remote\/hetzner status=skipped reason=skipped after previous failure source=\/tmp\/remote-matrix.json/)
  assert.match(text, /- workspace\/tracked status=dry-run source=\/tmp\/workspace-matrix.json/)
})

test("rejects inconsistent matrix aggregates", () => {
  const aggregate = summarizeDrillMatrixReports([
    matrixReport({
      matrix: "remote",
      scenarios: [
        scenario("remote", "failed", {
          classification: "provider-auth",
          reason: "expired token",
          runtimeSignals: ["provider-run-lifecycle"],
        }),
      ],
    }),
  ])

  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    totals: { ...aggregate.totals, failed: 2 },
  }), /scenario total does not match status counts/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    totals: { ...aggregate.totals, durationMs: 10.5 },
  }), /aggregate totals has invalid durationMs/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    totals: { ...aggregate.totals, scenarios: 2, failed: 2 },
  }), /totals.scenarios does not match reports/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      scenarioCount: 1.5,
    }],
  }), /reports\[0\] has invalid scenarioCount/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      durationMs: 10.5,
    }],
  }), /reports\[0\] has invalid durationMs/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      counts: { ...aggregate.reports[0].counts, failed: 2 },
    }],
  }), /reports\[0\] scenarioCount does not match counts/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      status: "passed",
    }],
  }), /reports\[0\] status does not match counts/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      counts: { ...aggregate.reports[0].counts, failed: -1 },
    }],
  }), /reports\[0\]\.counts has invalid failed/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      counts: { ...aggregate.reports[0].counts, failed: 0.5 },
    }],
  }), /reports\[0\]\.counts has invalid failed/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    owners: { "runtime-network": 1 },
  }), /owners do not match failedScenarios/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    classifications: { "provider-error": 1 },
  }), /classifications do not match reports/)
  assert.throws(() => validateDrillMatrixAggregate({
    ...aggregate,
    classifications: { "provider-ath": 1 },
  }), /aggregate\.classifications has unknown classification "provider-ath"/)
  assert.throws(() => validateDrillMatrixAggregate({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      classifications: { "provider-ath": 1 },
    }],
  }), /reports\[0\]\.classifications has unknown classification "provider-ath"/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    matrixNames: { remote: 2 },
  }), /matrixNames do not match reports/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    deploymentPresets: { local: 1 },
  }), /deploymentPresets do not match reports/)
  assert.throws(() => validateDrillMatrixAggregate({
    ...aggregate,
    deploymentPresets: { "same-host-remtoe": 1 },
  }), /aggregate\.deploymentPresets\[0\] has unknown deployment preset "same-host-remtoe"/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    providers: { codex: 2 },
  }), /providers do not match reports/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    scenarioIds: { remote: 2 },
  }), /scenarioIds do not match reports/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    exitCriteria: { "dry-run": 1 },
  }), /exitCriteria do not match incompleteExitCriteria/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      deploymentPresets: ["same-host-remtoe"],
    }],
  }), /reports\[0\]\.deploymentPresets\[0\] has unknown deployment preset "same-host-remtoe"/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      exitCriteria: { "not-real": 1 },
    }],
  }), /reports\[0\]\.exitCriteria has invalid status/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    runtimeSignalScenarios: {
      "provider-run-lifecycle": [{
        matrix: "other",
        source: null,
        id: "remote",
        status: "failed",
      }],
    },
  }), /runtimeSignalScenarios do not match reports/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    runtimeSignalOwners: { "kernel-authority": 1 },
  }), /runtimeSignalOwners do not match runtimeSignals/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    runtimeSignals: { "workspace-live-synch-state": 1 },
  }), /aggregate\.runtimeSignals has unknown runtime signal "workspace-live-synch-state"/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      runtimeSignals: { "workspace-live-synch-state": 1 },
    }],
  }), /reports\[0\]\.runtimeSignals has unknown runtime signal "workspace-live-synch-state"/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      runtimeSignalScenarios: {
        "provider-run-lifecycle": [],
      },
    }],
  }), /runtimeSignalScenarios\.provider-run-lifecycle has invalid scenarios/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    reports: [{
      ...aggregate.reports[0],
      scenarioIds: [],
    }],
  }), /reports\[0\] scenarioIds do not match scenarioCount/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    nextActions: [],
  }), /nextActions do not match failedScenarios/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    failedScenarios: [{
      ...aggregate.failedScenarios[0],
      classification: "not-real",
    }],
  }), /failedScenarios\[0\] has unknown classification/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    failedScenarios: [{
      ...aggregate.failedScenarios[0],
      owner: "runtime-network",
    }],
  }), /failedScenarios\[0\] owner does not match classification/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    failedScenarios: [{
      ...aggregate.failedScenarios[0],
      nextAction: "try something else",
    }],
  }), /failedScenarios\[0\] nextAction does not match classification/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    failedScenarios: [{
      ...aggregate.failedScenarios[0],
      artifactHints: ["Bearer abcdefghijklmnopqrstuvwxyz"],
    }],
  }), /failedScenarios\[0\] includes secret-looking artifactHints/)
  assert.throws(() => formatDrillMatrixAggregateSummary({
    ...aggregate,
    status: "passed",
  }), /status does not match totals/)
})

test("rejects unknown deployment preset labels in report metadata", () => {
  assert.throws(
    () => validateDrillMatrixReport(matrixReport({
      metadata: {
        deploymentPresets: "local,same-host-remtoe",
        deploymentPresetCount: 2,
      },
    })),
    /metadata\.deploymentPresets\[1\] has unknown deployment preset "same-host-remtoe"/,
  )
  assert.throws(
    () => validateDrillMatrixReport(matrixReport({
      metadata: {
        deploymentPresets: "local,hetzner",
        deploymentPresetCount: 1,
      },
    })),
    /metadata\.deploymentPresetCount does not match deploymentPresets/,
  )
})

test("reads and validates report files", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-report-"))
  const file = path.join(dir, "matrix.json")
  await writeFile(file, `${JSON.stringify(matrixReport())}\n`, "utf8")

  const report = await readDrillMatrixReport(file)

  assert.equal(report.schema, "arroba.drill.matrix.v1")
  assert.throws(() => validateDrillMatrixReport({ schema: "other", scenarios: [] }), /unsupported schema/)
  await rm(dir, { recursive: true, force: true })
})

test("discovers matrix reports below artifact roots", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-drill-report-find-"))
  const first = path.join(dir, ".artifacts", "drill-matrices", "one", "matrix.json")
  const second = path.join(dir, ".artifacts", "drill-matrices", "two", "matrix.json")
  const unrelated = path.join(dir, ".artifacts", "drill-matrices", "two", "other.json")
  const pruned = path.join(dir, "node_modules", "package", "matrix.json")
  const deep = path.join(dir, "one", "two", "three", "four", "matrix.json")
  await writeFileWithDir(first, `${JSON.stringify(matrixReport({ matrix: "one" }))}\n`)
  await writeFileWithDir(second, `${JSON.stringify(matrixReport({ matrix: "two" }))}\n`)
  await writeFileWithDir(unrelated, `${JSON.stringify({ schema: "other" })}\n`)
  await writeFileWithDir(pruned, `${JSON.stringify(matrixReport({ matrix: "pruned" }))}\n`)
  await writeFileWithDir(deep, `${JSON.stringify(matrixReport({ matrix: "deep" }))}\n`)

  const reports = await findDrillMatrixReportPaths([path.join(dir, ".artifacts")])
  const broadReports = await findDrillMatrixReportPaths([dir])
  const shallowReports = await findDrillMatrixReportPaths([dir], { maxDepth: 2 })

  assert.deepEqual(reports, [first, second].sort())
  assert.deepEqual(broadReports, [deep, first, second].sort())
  assert.deepEqual(shallowReports, [])
  await rm(dir, { recursive: true, force: true })
})

test("rejects malformed matrix reports", () => {
  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    status: "unknown",
  }), /invalid status/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({ scenarios: [] }),
  }), /has no scenarios/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      status: "passed",
      scenarios: [scenario("remote", "failed", { classification: "provider-auth", reason: "expired token" })],
    }),
  }), /status does not match scenario statuses/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      status: "dry-run",
      dryRun: false,
      scenarios: [scenario("remote", "dry-run")],
    }),
  }), /dryRun does not match scenario statuses/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    durationMs: -1,
  }), /invalid durationMs/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      durationMs: 1000.5,
      completedAt: "2026-06-13T00:00:01.500Z",
    }),
  }), /invalid durationMs/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    durationMs: 999,
  }), /durationMs must match completedAt - startedAt/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    startedAt: "2026-06-13",
  }), /startedAt must be an ISO timestamp/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    startedAt: "2026-06-13T00:00:02.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
  }), /completedAt must not be before startedAt/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), command: "" }],
  }), /scenarios\[0\] is missing command/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), args: [1] }],
  }), /scenarios\[0\] has invalid args/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), exitCriteria: [1] }],
  }), /scenarios\[0\] has invalid exitCriteria/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [scenario("broken", "passed", {
      exitCriteria: ["criterion"],
      exitCriteriaEvidence: [],
    })],
  }), /scenarios\[0\]\.exitCriteriaEvidence length does not match exitCriteria/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [scenario("broken", "passed", {
      exitCriteria: ["criterion"],
      exitCriteriaEvidence: [{
        id: "broken:exit-01",
        criterion: "different criterion",
        status: "satisfied",
        reason: null,
      }],
    })],
  }), /scenarios\[0\]\.exitCriteriaEvidence\[0\] criterion does not match exitCriteria/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [scenario("broken", "passed", {
      exitCriteria: ["criterion"],
      exitCriteriaEvidence: [{
        id: "broken:exit-01",
        criterion: "criterion",
        status: "dry-run",
        reason: null,
      }],
    })],
  }), /scenarios\[0\]\.exitCriteriaEvidence\[0\] incomplete criterion is missing reason/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), artifactHints: [1] }],
  }), /scenarios\[0\] has invalid artifactHints/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), artifactHints: ["/tmp/arroba-drill-sk-this-should-not-persist"] }],
  }), /scenarios\[0\] includes secret-looking artifactHints/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), artifactHints: [{ kind: "manifest", path: "Bearer abcdefghijklmnopqrstuvwxyz" }] }],
  }), /scenarios\[0\] includes secret-looking artifactHints/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "failed", { classification: "child-process", reason: "" })],
    }),
  }), /scenarios\[0\] failed scenario is missing reason/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "failed", { reason: "code=1" })],
    }),
  }), /scenarios\[0\] failed scenario is missing classification/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "failed", { classification: "typo-runtime", reason: "code=1" })],
    }),
  }), /scenarios\[0\] has unknown classification "typo-runtime"/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "failed", {
        classification: "provider-auth",
        owner: "runtime-network",
        reason: "expired token",
      })],
    }),
  }), /scenarios\[0\] owner does not match classification/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "failed", {
        classification: "provider-auth",
        nextAction: "try something else",
        reason: "expired token",
      })],
    }),
  }), /scenarios\[0\] nextAction does not match classification/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "skipped", { reason: null })],
    }),
  }), /scenarios\[0\] skipped scenario is missing reason/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "skipped", { reason: "not selected", durationMs: 1 })],
    }),
  }), /scenarios\[0\] skipped scenario must have zero durationMs/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "passed", { durationMs: 1.5 })],
    }),
  }), /scenarios\[0\] has invalid durationMs/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      status: "dry-run",
      dryRun: true,
      scenarios: [scenario("broken", "dry-run", { durationMs: 1 })],
    }),
  }), /scenarios\[0\] dry-run scenario must have zero durationMs/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      status: "dry-run",
      dryRun: true,
      scenarios: [scenario("broken", "dry-run", { reason: "not run" })],
    }),
  }), /scenarios\[0\] dry-run scenario must not include reason/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      status: "dry-run",
      dryRun: true,
      scenarios: [scenario("broken", "dry-run", { classification: "expected-failure" })],
    }),
  }), /scenarios\[0\] dry-run scenario must not include classification/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      scenarios: [scenario("broken", "passed", { reason: "unexpected warning" })],
    }),
  }), /scenarios\[0\] passed scenario must not include reason/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { relayToken: "redacted-or-not-it-should-not-be-here" },
  }), /sensitive metadata key/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { provider: "Bearer abcdefghijklmnopqrstuvwxyz" },
  }), /secret-looking metadata value/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex,opencode", providerCount: 3 },
  }), /metadata\.providerCount does not match providers/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex,cdoex", providerCount: 2 },
  }), /metadata\.providers\[\d+\] has unknown provider "cdoex"/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex", providerModelOverrides: "opencode" },
  }), /metadata\.providerModelOverrides includes provider not in providers/)

  assert.doesNotThrow(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex,opencode", providerAccountAliases: "codex=work" },
  }))

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex", providerAccountAliases: "opencode=zen" },
  }), /metadata\.providerAccountAliases includes provider not in providers/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { providers: "codex", providerAccountAliases: "codex=user@example.test" },
  }), /metadata\.providerAccountAliases includes invalid account alias/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      metadata: { providers: "codex,opencode", providerCount: 2 },
      scenarios: [scenario("local", "passed", { providers: ["codex", ""] })],
    }),
  }), /scenarios\[0\]\.providers has invalid providers/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      metadata: { providers: "codex", providerCount: 1 },
      scenarios: [scenario("local", "passed", { providers: ["codex", "cdoex"] })],
    }),
  }), /scenarios\[0\]\.providers\[1\] has unknown provider "cdoex"/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport({
      metadata: { providers: "codex,opencode", providerCount: 2 },
      scenarios: [scenario("local", "passed", { providers: ["codex"] })],
    }),
  }), /metadata\.providers do not match scenario providers/)

  const aggregate = summarizeDrillMatrixReports([matrixReport({
    metadata: { providers: "codex", providerCount: 1 },
    scenarios: [scenario("local", "passed", { providers: ["codex"] })],
  })])
  assert.throws(() => validateDrillMatrixAggregate({
    ...aggregate,
    providers: { cdoex: 1 },
  }), /aggregate\.providers\[0\] has unknown provider "cdoex"/)
})

function matrixReport(overrides = {}) {
  const scenarios = overrides.scenarios ?? [scenario("local", "passed")]
  const status = overrides.status ?? matrixStatusForScenarios(scenarios)
  const dryRun = overrides.dryRun ?? status === "dry-run"
  const startedAt = overrides.startedAt ?? "2026-06-13T00:00:00.000Z"
  const durationMs = overrides.durationMs ?? 1000
  const completedAt = overrides.completedAt ?? new Date(Date.parse(startedAt) + durationMs).toISOString()
  return {
    schema: "arroba.drill.matrix.v1",
    matrix: "test-matrix",
    status,
    dryRun,
    startedAt,
    completedAt,
    durationMs,
    metadata: {},
    scenarios,
    ...overrides,
  }
}

function matrixStatusForScenarios(scenarios) {
  if (scenarios.some((entry) => entry.status === "failed")) return "failed"
  if (scenarios.length > 0 && scenarios.every((entry) => entry.status === "dry-run")) return "dry-run"
  return "passed"
}

function scenario(id, status, overrides = {}) {
  return {
    id,
    description: `${id} scenario`,
    requires: [],
    exitCriteria: [],
    status,
    expectedFailure: false,
    classification: null,
    durationMs: status === "skipped" || status === "dry-run" ? 0 : 10,
    reason: null,
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
    ...overrides,
  }
}

async function writeFileWithDir(file, contents) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, contents, "utf8")
}
