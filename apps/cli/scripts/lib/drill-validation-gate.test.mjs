import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  drillValidationGateExitCode,
  findDrillValidationGateAggregatePaths,
  findDrillValidationGateReportPaths,
  formatDrillValidationGateAggregateSummary,
  formatDrillValidationGateSummary,
  readDrillValidationGateAggregate,
  readDrillValidationGateReport,
  runDrillValidationGate,
  summarizeDrillValidationGateReports,
  validateDrillValidationGateAggregate,
  validateDrillValidationGateReport,
} from "./drill-validation-gate.mjs"
import { writeDrillArtifactIndex } from "./drill-artifacts.mjs"
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
    assert.deepEqual(report.checks.platformBundle.validationSuite, {
      testCount: 25,
      coverageAreas: [
        { id: "artifact-contracts", testCount: 7 },
        { id: "failure-diagnostics", testCount: 3 },
        { id: "matrix-validation", testCount: 6 },
        { id: "runtime-fixtures", testCount: 7 },
        { id: "suite-contract", testCount: 2 },
      ],
    })
    assert.equal(report.checks.matrices.status, "passed")
    assert.equal(report.checks.failures.status, "skipped")
    assert.deepEqual(report.nextActions, [])
    assert.doesNotThrow(() => validateDrillValidationGateReport(report))
    assert.match(formatDrillValidationGateSummary(report), /status=passed/)
    assert.match(formatDrillValidationGateSummary(report), /platform_validation_suite_tests=25 coverage=artifact-contracts:7/)
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

test("passes with explicit artifact index paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, "{\"schema\":\"arroba.drill.validation_gate.v1\"}\n", "utf8")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
    })
    const indexPath = path.join(rootDir, "arroba-drill-artifacts.json")

    const report = await runDrillValidationGate({ artifactIndexes: [indexPath] })

    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.inputs, [indexPath])
    assert.deepEqual(report.checks.artifacts.indexPaths, [indexPath])
    assert.equal(report.checks.artifacts.aggregate.totals.artifacts, 1)
    assert.match(formatDrillValidationGateSummary(report), /artifacts=passed roots=0 inputs=1 indexes=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when configured artifact roots contain no indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const report = await runDrillValidationGate({ artifactRoots: [rootDir] })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.artifacts.error, "no artifact indexes found")
    assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
      { owner: "validation-harness", classification: "artifact-index" },
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when artifact indexes point at tampered artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, "{\"schema\":\"arroba.drill.validation_gate.v1\"}\n", "utf8")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
    })
    await writeFile(reportPath, "{\"schema\":\"tampered\"}\n", "utf8")

    const report = await runDrillValidationGate({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
    })

    assert.equal(report.status, "failed")
    assert.equal(report.checks.artifacts.status, "failed")
    assert.match(report.checks.artifacts.error, /sha256 mismatch/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects validation gate reports with mismatched top-level status", async () => {
  const report = await runDrillValidationGate()

  assert.throws(
    () => validateDrillValidationGateReport({ ...report, status: "passed" }),
    /status does not match check statuses/,
  )
})

test("rejects malformed platform bundle artifact evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    const report = await runDrillValidationGate({ platformBundleDir: bundleDir })
    const malformed = {
      ...report,
      checks: {
        ...report.checks,
        platformBundle: {
          ...report.checks.platformBundle,
          artifacts: [{
            ...report.checks.platformBundle.artifacts[0],
            sha256: "not-a-sha",
          }],
        },
      },
    }

    assert.throws(
      () => formatDrillValidationGateSummary(malformed),
      /artifacts\[0\] has invalid sha256/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects inconsistent platform bundle validation suite evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    const report = await runDrillValidationGate({ platformBundleDir: bundleDir })
    const malformed = {
      ...report,
      checks: {
        ...report.checks,
        platformBundle: {
          ...report.checks.platformBundle,
          validationSuite: {
            ...report.checks.platformBundle.validationSuite,
            coverageAreas: report.checks.platformBundle.validationSuite.coverageAreas.slice(1),
          },
        },
      },
    }

    assert.throws(
      () => formatDrillValidationGateSummary(malformed),
      /coverageAreas do not match testCount/,
    )
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

test("reads and discovers validation gate report artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const bundleDir = path.join(rootDir, "bundle")
    const reportPath = path.join(rootDir, "reports", "gate.json")
    await writeDrillPlatformBundle(bundleDir)
    const report = await runDrillValidationGate({ platformBundleDir: bundleDir })
    await mkdir(path.dirname(reportPath), { recursive: true })
    await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`, "utf8")
    await writeFile(path.join(rootDir, "reports", "unrelated.json"), "{\"schema\":\"other\"}\n", "utf8")

    assert.deepEqual(await findDrillValidationGateReportPaths([rootDir]), [reportPath])
    assert.deepEqual(await readDrillValidationGateReport(reportPath), report)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("summarizes validation gate reports", async () => {
  const passed = await runDrillValidationGate({
    failureRoots: ["/tmp/no-such-arroba-failure-root"],
  })
  const failed = await runDrillValidationGate()
  const aggregate = summarizeDrillValidationGateReports([passed, failed], {
    sources: ["passed.json", "failed.json"],
  })

  assert.equal(aggregate.schema, "arroba.drill.validation_gate.aggregate.v1")
  assert.equal(aggregate.status, "failed")
  assert.deepEqual(aggregate.totals, { reports: 2, passed: 1, failed: 1 })
  assert.deepEqual(aggregate.reports.map((report) => report.source), ["passed.json", "failed.json"])
  assert.deepEqual(aggregate.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
    { owner: "validation-harness", classification: "validation-gate" },
  ])
  assert.doesNotThrow(() => validateDrillValidationGateAggregate(aggregate))
  assert.match(formatDrillValidationGateAggregateSummary(aggregate), /status=failed reports=2 passed=1 failed=1/)
})

test("reads and discovers validation gate aggregate artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-"))
  try {
    const aggregatePath = path.join(rootDir, "reports", "aggregate.json")
    const aggregate = summarizeDrillValidationGateReports([await runDrillValidationGate({
      failureRoots: ["/tmp/no-such-arroba-failure-root"],
    })])
    await mkdir(path.dirname(aggregatePath), { recursive: true })
    await writeFile(aggregatePath, `${JSON.stringify(aggregate, null, 2)}\n`, "utf8")
    await writeFile(path.join(rootDir, "reports", "gate.json"), `${JSON.stringify(await runDrillValidationGate(), null, 2)}\n`, "utf8")

    assert.deepEqual(await findDrillValidationGateAggregatePaths([rootDir]), [aggregatePath])
    assert.deepEqual(await readDrillValidationGateAggregate(aggregatePath), aggregate)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects inconsistent validation gate aggregates", async () => {
  const aggregate = summarizeDrillValidationGateReports([await runDrillValidationGate()])

  assert.throws(
    () => validateDrillValidationGateAggregate({
      ...aggregate,
      totals: {
        ...aggregate.totals,
        failed: 0,
      },
    }),
    /totals do not match reports/,
  )
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
