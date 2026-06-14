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
  validateDrillMatrixReport,
} from "./drill-matrix-report.mjs"

test("summarizes matrix report status and scenario counts", () => {
  const report = matrixReport({
    scenarios: [
      scenario("local", "passed"),
      scenario("remote", "failed", { classification: "provider-auth", reason: "Token refresh failed: 401" }),
      scenario("hetzner", "skipped", { reason: "skipped after previous failure" }),
    ],
  })

  const summary = summarizeDrillMatrixReport(report)

  assert.equal(summary.status, "failed")
  assert.deepEqual(summary.counts, { passed: 1, failed: 1, skipped: 1, dryRun: 0 })
  assert.deepEqual(summary.classifications, { "provider-auth": 1 })
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
        artifactHints: ["/tmp/arroba-drill-remote"],
      }),
      scenario("cloud", "failed", { classification: "cloud-runtime", reason: "deployment did not become ready" }),
      scenario("hetzner", "skipped", { reason: "skipped after previous failure" }),
    ],
  })

  const text = formatDrillMatrixReportSummary(report, { source: "/tmp/report.json" })

  assert.match(text, /matrix report: test-matrix \(\/tmp\/report\.json\)/)
  assert.match(text, /status=failed scenarios=3 passed=0 failed=2 skipped=1 dry_run=0/)
  assert.match(text, /classifications: cloud-runtime=1 provider-account=1/)
  assert.match(text, /- remote classification=provider-account owner=provider-account reason=insufficient balance/)
  assert.match(text, /criteria: remote worker executes the selected provider turn; home observes completion/)
  assert.match(text, /artifacts: \/tmp\/arroba-drill-remote/)
  assert.match(text, /next: check provider quota or billing/)
  assert.match(text, /- cloud classification=cloud-runtime owner=cloud-deployment reason=deployment did not become ready/)
  assert.match(text, /next: inspect Cloud deployment\/control-plane status/)
  assert.match(text, /skipped scenarios: hetzner/)
})

test("formats dry-run reports without failures", () => {
  const report = matrixReport({
    status: "dry-run",
    dryRun: true,
    scenarios: [scenario("local", "dry-run", { exitCriteria: ["local runtime path is selected"] })],
  })

  const text = formatDrillMatrixReportSummary(report)

  assert.match(text, /status=dry-run/)
  assert.match(text, /selected scenario criteria:/)
  assert.match(text, /- local: local runtime path is selected/)
  assert.match(text, /next: run without --dry-run/)
  assert.equal(drillMatrixReportExitCode([report]), 0)
  assert.equal(drillMatrixReportCompletionExitCode([report]), 2)
})

test("aggregates multiple matrix reports for CI", () => {
  const failed = matrixReport({
    matrix: "remote",
    scenarios: [
      scenario("local", "passed"),
      scenario("remote", "failed", {
        classification: "provider-auth",
        reason: "expired token",
        artifactHints: ["/tmp/arroba-drill-remote"],
      }),
      scenario("hetzner", "skipped", { reason: "skipped after previous failure" }),
    ],
  })
  const dryRun = matrixReport({
    matrix: "workspace",
    status: "dry-run",
    dryRun: true,
    durationMs: 25,
    scenarios: [scenario("tracked", "dry-run")],
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
  assert.deepEqual(aggregate.failedScenarios, [{
    matrix: "remote",
    source: "/tmp/remote-matrix.json",
    id: "remote",
    classification: "provider-auth",
    owner: "provider-account",
    reason: "expired token",
    artifactHints: ["/tmp/arroba-drill-remote"],
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
  assert.match(text, /artifacts: \/tmp\/arroba-drill-remote/)
  assert.match(text, /owners: provider-account=1/)
  assert.match(text, /next actions:/)
  assert.match(text, /owner=provider-account classification=provider-auth count=1: refresh provider login/)
  assert.match(text, /next: refresh provider login/)
  assert.match(text, /incomplete scenarios:/)
  assert.match(text, /- remote\/hetzner status=skipped reason=skipped after previous failure source=\/tmp\/remote-matrix.json/)
  assert.match(text, /- workspace\/tracked status=dry-run source=\/tmp\/workspace-matrix.json/)
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
  await writeFileWithDir(first, `${JSON.stringify(matrixReport({ matrix: "one" }))}\n`)
  await writeFileWithDir(second, `${JSON.stringify(matrixReport({ matrix: "two" }))}\n`)
  await writeFileWithDir(unrelated, `${JSON.stringify({ schema: "other" })}\n`)

  const reports = await findDrillMatrixReportPaths([path.join(dir, ".artifacts")])

  assert.deepEqual(reports, [first, second].sort())
  await rm(dir, { recursive: true, force: true })
})

test("rejects malformed matrix reports", () => {
  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    status: "unknown",
  }), /invalid status/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    durationMs: -1,
  }), /invalid durationMs/)

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
    scenarios: [{ ...scenario("broken", "passed"), artifactHints: [1] }],
  }), /scenarios\[0\] has invalid artifactHints/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    scenarios: [{ ...scenario("broken", "passed"), artifactHints: ["/tmp/arroba-drill-sk-this-should-not-persist"] }],
  }), /scenarios\[0\] includes secret-looking artifactHints/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { relayToken: "redacted-or-not-it-should-not-be-here" },
  }), /sensitive metadata key/)

  assert.throws(() => validateDrillMatrixReport({
    ...matrixReport(),
    metadata: { provider: "Bearer abcdefghijklmnopqrstuvwxyz" },
  }), /secret-looking metadata value/)
})

function matrixReport(overrides = {}) {
  return {
    schema: "arroba.drill.matrix.v1",
    matrix: "test-matrix",
    status: "failed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata: {},
    scenarios: [scenario("local", "passed")],
    ...overrides,
  }
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
