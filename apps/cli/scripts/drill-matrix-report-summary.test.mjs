import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { promisify } from "node:util"
import { fileURLToPath } from "node:url"

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
