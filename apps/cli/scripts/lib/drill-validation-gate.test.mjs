import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  drillValidationGateExitCode,
  formatDrillValidationGateSummary,
  runDrillValidationGate,
} from "./drill-validation-gate.mjs"
import { writeDrillPlatformBundle } from "./drill-platform-bundle.mjs"

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
    assert.equal(report.checks.matrices.status, "passed")
    assert.equal(report.checks.failures.status, "skipped")
    assert.deepEqual(report.nextActions, [])
    assert.match(formatDrillValidationGateSummary(report), /status=passed/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("passes with explicit matrix report paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "matrix.json")
    await writeMatrixReport(reportPath, matrixReport())

    const report = await runDrillValidationGate({
      matrixReports: [reportPath],
      requireComplete: true,
    })

    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.matrices.inputs, [reportPath])
    assert.deepEqual(report.checks.matrices.reportPaths, [reportPath])
    assert.match(formatDrillValidationGateSummary(report), /matrices=passed roots=0 inputs=1 reports=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when no validation checks are configured", async () => {
  const report = await runDrillValidationGate()

  assert.equal(report.status, "failed")
  assert.equal(report.checks.configuration.error, "no validation checks configured")
  assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
    { owner: "validation-harness", classification: "validation-gate" },
  ])
  assert.match(formatDrillValidationGateSummary(report), /configuration=failed/)
})

test("fails when configured matrix roots contain no reports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const report = await runDrillValidationGate({ matrixRoots: [rootDir] })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.matrices.error, "no matrix reports found")
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "validation-harness", classification: "matrix-artifacts" },
    ])
    assert.equal(drillValidationGateExitCode(report), 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when require-complete sees dry-run matrix scenarios", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    await writeMatrixReport(path.join(rootDir, "matrix.json"), matrixReport({
      status: "dry-run",
      dryRun: true,
      scenarios: [scenario("remote", "dry-run")],
    }))

    const report = await runDrillValidationGate({
      matrixRoots: [rootDir],
      requireComplete: true,
    })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.matrices.aggregate.status, "dry-run")
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "validation-harness", classification: "incomplete-matrix" },
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when preserved failure manifests are found", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    await writeFailureManifest(path.join(rootDir, "failed", "arroba-drill-failure.json"))

    const report = await runDrillValidationGate({ failureRoots: [rootDir] })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.failures.aggregate.total, 1)
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "provider-account", classification: "provider-auth" },
    ])
    assert.match(formatDrillValidationGateSummary(report), /failure_total=1/)
    assert.match(formatDrillValidationGateSummary(report), /next actions:/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails with explicit failure manifest paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const manifestPath = path.join(rootDir, "arroba-drill-failure.json")
    await writeFailureManifest(manifestPath)

    const report = await runDrillValidationGate({ failureInputs: [manifestPath] })

    assert.equal(report.status, "failed")
    assert.deepEqual(report.checks.failures.inputs, [manifestPath])
    assert.deepEqual(report.checks.failures.manifestPaths, [manifestPath])
    assert.match(formatDrillValidationGateSummary(report), /failures=failed roots=0 inputs=1 manifests=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("resolves explicit failure root inputs to manifest paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const failureRoot = path.join(rootDir, "failed")
    const manifestPath = path.join(failureRoot, "arroba-drill-failure.json")
    await writeFailureManifest(manifestPath)

    const report = await runDrillValidationGate({ failureInputs: [failureRoot] })

    assert.equal(report.status, "failed")
    assert.deepEqual(report.checks.failures.inputs, [failureRoot])
    assert.deepEqual(report.checks.failures.manifestPaths, [manifestPath])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeMatrixReport(file, report) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(report, null, 2)}\n`, "utf8")
}

async function writeFailureManifest(file) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.failure.v1",
    rootDir: path.dirname(file),
    failedAt: "2026-06-13T00:00:00.000Z",
    metadata: { drill: "failed-drill" },
    error: { name: "Error", message: "Token refresh failed: 401", stack: null },
  }, null, 2)}\n`, "utf8")
}

function matrixReport(overrides = {}) {
  const scenarios = overrides.scenarios ?? [scenario("local", "passed")]
  const status = overrides.status ?? (scenarios.some((entry) => entry.status === "failed") ? "failed" : "passed")
  const dryRun = overrides.dryRun ?? false
  return {
    schema: "arroba.drill.matrix.v1",
    matrix: "test-matrix",
    status,
    dryRun,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata: {},
    scenarios,
  }
}

function scenario(id, status) {
  return {
    id,
    description: `${id} scenario`,
    requires: [],
    exitCriteria: [],
    status,
    expectedFailure: false,
    classification: status === "failed" ? "child-process" : null,
    durationMs: status === "skipped" || status === "dry-run" ? 0 : 10,
    reason: status === "failed" ? "code=1" : status === "skipped" ? "not run" : null,
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
  }
}
