import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
import { runDrillValidationGate } from "./lib/drill-validation-gate.mjs"
import { writeDrillPlatformBundle } from "./lib/drill-platform-bundle.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-validation-gate-summary.mjs", import.meta.url))

test("drill validation gate summary aggregates discovered reports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const passedReportPath = path.join(rootDir, "reports", "passed.json")
    const failedReportPath = path.join(rootDir, "reports", "failed.json")
    await writeGateReport(passedReportPath, await passingGateReport(rootDir))
    await writeGateReport(failedReportPath, await runDrillValidationGate())

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--gate-root",
        path.join(rootDir, "reports"),
        "--json",
        "--output",
        outputPath,
        "--output-artifact-index",
        artifactIndexPath,
      ]),
      (error) => {
        const stdoutAggregate = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(stdoutAggregate.status, "failed")
        assert.deepEqual(stdoutAggregate.totals, { reports: 2, passed: 1, failed: 1 })
        assert.deepEqual(stdoutAggregate.reports.map((report) => report.source), [
          failedReportPath,
          passedReportPath,
        ])
        return true
      },
    )

    const fileAggregate = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)
    assert.equal(fileAggregate.status, "failed")
    assert.deepEqual(fileAggregate.totals, { reports: 2, passed: 1, failed: 1 })
    assert.equal(artifactIndex.metadata.drill, "validation-gate-summary")
    assert.equal(artifactIndex.metadata.status, "failed")
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "aggregate.json",
      schema: "arroba.drill.validation_gate.aggregate.v1",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

test("drill validation gate summary accepts explicit report paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-summary-"))
  try {
    const reportPath = path.join(rootDir, "gate.json")
    await writeGateReport(reportPath, await passingGateReport(rootDir))

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--gate-report",
      reportPath,
      "--json",
    ])
    const aggregate = JSON.parse(stdout)

    assert.equal(aggregate.status, "passed")
    assert.deepEqual(aggregate.totals, { reports: 1, passed: 1, failed: 0 })
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation gate summary rejects empty inputs", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /no validation gate reports found/)
      return true
    },
  )
})

async function passingGateReport(rootDir) {
  const bundleDir = path.join(rootDir, `bundle-${Date.now()}-${Math.random().toString(16).slice(2)}`)
  await writeDrillPlatformBundle(bundleDir)
  return runDrillValidationGate({ platformBundleDir: bundleDir })
}

async function writeGateReport(file, report) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(report, null, 2)}\n`, "utf8")
}
