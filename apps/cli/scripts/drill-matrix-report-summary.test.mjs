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

function matrixReport({ matrix }) {
  return {
    schema: "arroba.drill.matrix.v1",
    matrix,
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata: {},
    scenarios: [{
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
    }],
  }
}
