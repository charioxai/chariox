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

export {
  assert,
  drillMatrixReportCompletionExitCode,
  drillMatrixReportExitCode,
  findDrillMatrixReportPaths,
  formatDrillMatrixAggregateSummary,
  formatDrillMatrixReportSummary,
  mkdir,
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
}

export function matrixReport(overrides = {}) {
  const scenarios = overrides.scenarios ?? [scenario("local", "passed")]
  const status = overrides.status ?? matrixStatusForScenarios(scenarios)
  const dryRun = overrides.dryRun ?? status === "dry-run"
  const startedAt = overrides.startedAt ?? "2026-06-13T00:00:00.000Z"
  const durationMs = overrides.durationMs ?? 1000
  const completedAt = overrides.completedAt ?? new Date(Date.parse(startedAt) + durationMs).toISOString()
  return {
    schema: "chariox.drill.matrix.v1",
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

export function matrixStatusForScenarios(scenarios) {
  if (scenarios.some((entry) => entry.status === "failed")) return "failed"
  if (scenarios.length > 0 && scenarios.every((entry) => entry.status === "dry-run")) return "dry-run"
  return "passed"
}

export function scenario(id, status, overrides = {}) {
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

export async function writeFileWithDir(file, contents) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, contents, "utf8")
}
