import assert from "node:assert/strict"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  drillMatrixReportExitCode,
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
})

test("formats failed and skipped scenarios with next actions", () => {
  const report = matrixReport({
    scenarios: [
      scenario("remote", "failed", {
        classification: "provider-account",
        reason: "insufficient balance",
        exitCriteria: ["remote worker executes the selected provider turn", "home observes completion"],
      }),
      scenario("cloud", "skipped", { reason: "skipped after previous failure" }),
    ],
  })

  const text = formatDrillMatrixReportSummary(report, { source: "/tmp/report.json" })

  assert.match(text, /matrix report: test-matrix \(\/tmp\/report\.json\)/)
  assert.match(text, /status=failed scenarios=2 passed=0 failed=1 skipped=1 dry_run=0/)
  assert.match(text, /classifications: provider-account=1/)
  assert.match(text, /- remote classification=provider-account reason=insufficient balance/)
  assert.match(text, /criteria: remote worker executes the selected provider turn; home observes completion/)
  assert.match(text, /next: check provider quota or billing/)
  assert.match(text, /skipped scenarios: cloud/)
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
})

test("aggregates multiple matrix reports for CI", () => {
  const failed = matrixReport({
    matrix: "remote",
    scenarios: [
      scenario("local", "passed"),
      scenario("remote", "failed", { classification: "provider-auth", reason: "expired token" }),
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

  const aggregate = summarizeDrillMatrixReports([failed, dryRun])

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
  assert.deepEqual(aggregate.failedScenarios, [{ matrix: "remote", id: "remote", classification: "provider-auth", reason: "expired token" }])
  assert.deepEqual(aggregate.skippedScenarios, [{ matrix: "remote", id: "hetzner", reason: "skipped after previous failure" }])
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
    ...overrides,
  }
}
