import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { promisify } from "node:util"
import { fileURLToPath } from "node:url"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-matrix-report-summary.mjs", import.meta.url))

test("matrix report summary max-depth limits discovery", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-matrix-summary-"))
  const rootReport = path.join(dir, "matrix.json")
  const nestedReport = path.join(dir, ".artifacts", "drill-matrices", "nested", "matrix.json")
  await writeReport(rootReport, matrixReport({ matrix: "root" }))
  await writeReport(nestedReport, matrixReport({ matrix: "nested" }))

  const shallow = await runSummary(["--find", dir, "--max-depth", "0", "--json"])
  const broad = await runSummary(["--find", dir, "--json"])

  assert.equal(shallow.totals.reports, 1)
  assert.deepEqual(shallow.reports.map((report) => report.source), [rootReport])
  assert.equal(broad.totals.reports, 2)
  assert.deepEqual(broad.reports.map((report) => report.source), [nestedReport, rootReport].sort())

  await rm(dir, { recursive: true, force: true })
})

test("matrix report summary rejects invalid max-depth", async () => {
  await assert.rejects(
    () => execFile(process.execPath, [scriptPath, "--find", ".", "--max-depth", "nope"]),
    /--max-depth must be a non-negative integer/,
  )
})

test("matrix report summary writes artifact index for output", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-matrix-summary-"))
  try {
    const reportPath = path.join(dir, "matrix.json")
    const outputPath = path.join(dir, "aggregate.json")
    const artifactIndexPath = path.join(dir, "arroba-drill-artifacts.json")
    await writeReport(reportPath, matrixReport({ matrix: "root" }))

    const aggregate = await runSummary([
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
      reportPath,
    ])
    const fileAggregate = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.equal(aggregate.status, "passed")
    assert.deepEqual(fileAggregate, aggregate)
    assert.equal(artifactIndex.metadata.drill, "matrix-report-summary")
    assert.equal(artifactIndex.metadata.status, "passed")
    assert.equal(artifactIndex.metadata.owners, "")
    assert.equal(artifactIndex.metadata.classifications, "")
    assert.equal(artifactIndex.metadata.runtimeSignals, "")
    assert.equal(artifactIndex.metadata.runtimeSignalOwners, "")
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "aggregate.json",
      schema: "arroba.drill.matrix.aggregate.v1",
    }])
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test("matrix report summary indexes failure owner and classification metadata", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-matrix-summary-"))
  try {
    const reportPath = path.join(dir, "matrix.json")
    const outputPath = path.join(dir, "aggregate.json")
    const artifactIndexPath = path.join(dir, "arroba-drill-artifacts.json")
    await writeReport(reportPath, matrixReport({
      matrix: "remote-agent-runtime-matrix",
      status: "failed",
      scenarios: [{
        id: "remote",
        description: "remote scenario",
        requires: [],
        exitCriteria: [],
        status: "failed",
        expectedFailure: false,
        classification: "provider-auth",
        durationMs: 10,
        reason: "expired token",
        command: "node",
        args: ["remote.mjs"],
        artifactHints: [],
        runtimeSignals: ["provider-run-lifecycle", "lease-health"],
      }],
    }))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--json",
        "--output",
        outputPath,
        "--output-artifact-index",
        artifactIndexPath,
        reportPath,
      ]),
      (error) => {
        assert.equal(error.code, 1)
        return true
      },
    )

    const aggregate = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)
    assert.equal(aggregate.status, "failed")
    assert.equal(artifactIndex.metadata.status, "failed")
    assert.equal(artifactIndex.metadata.owners, "provider-account")
    assert.equal(artifactIndex.metadata.classifications, "provider-auth")
    assert.equal(artifactIndex.metadata.runtimeSignals, "lease-health,provider-run-lifecycle")
    assert.equal(artifactIndex.metadata.runtimeSignalOwners, "kernel-authority,provider-runtime")
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test("matrix report summary prints incomplete exit criteria", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-matrix-summary-"))
  try {
    const reportPath = path.join(dir, "dry-run-matrix.json")
    await writeReport(reportPath, matrixReport({
      matrix: "dry-run-matrix",
      status: "dry-run",
      dryRun: true,
      scenarios: [{
        id: "remote",
        description: "remote scenario",
        requires: [],
        exitCriteria: ["remote worker acknowledges projection"],
        status: "dry-run",
        expectedFailure: false,
        classification: null,
        durationMs: 0,
        reason: null,
        command: "node",
        args: ["remote.mjs"],
        artifactHints: [],
      }],
    }))

    const { stdout } = await execFile(process.execPath, [scriptPath, reportPath])

    assert.match(stdout, /matrix report: dry-run-matrix/)
    assert.match(stdout, /incomplete exit criteria:/)
    assert.match(stdout, /remote\/remote:exit-01 status=dry-run reason=scenario command was selected but not executed: remote worker acknowledges projection/)
    assert.match(stdout, /next: run or reconcile incomplete criteria before treating this matrix report as complete/)
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test("matrix report summary rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

async function runSummary(args) {
  const { stdout } = await execFile(process.execPath, [scriptPath, ...args])
  return JSON.parse(stdout)
}

async function writeReport(file, report) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(report)}\n`, "utf8")
}

function matrixReport({ matrix, ...overrides }) {
  const scenarios = overrides.scenarios ?? [{
    id: "local",
    description: "local scenario",
    requires: [],
    exitCriteria: [],
    status: "passed",
    expectedFailure: false,
    classification: null,
    durationMs: 10,
    reason: null,
    command: "node",
    args: ["local.mjs"],
    artifactHints: [],
  }]
  const status = overrides.status ?? "passed"
  return {
    schema: "arroba.drill.matrix.v1",
    matrix,
    status,
    dryRun: overrides.dryRun ?? status === "dry-run",
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata: {},
    scenarios,
    ...overrides,
  }
}
