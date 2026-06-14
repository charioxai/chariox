import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  defaultDrillMatrixReportPath,
  extractDrillArtifactHints,
  parseDrillScenarioIds,
  quoteDrillCommand,
  runDrillMatrix,
  selectDrillMatrixScenarios,
} from "./drill-matrix-runner.mjs"

test("parses comma-separated scenario ids", () => {
  assert.deepEqual(parseDrillScenarioIds("one, two,,three "), ["one", "two", "three"])
  assert.equal(parseDrillScenarioIds(null), null)
})

test("selects default scenarios by enabled requirements", () => {
  const scenarios = [
    { id: "local" },
    { id: "remote", requires: ["remote"] },
    { id: "hetzner", requires: ["remote", "hetzner"] },
  ]

  assert.deepEqual(
    selectDrillMatrixScenarios({
      scenarios,
      enabledRequirements: new Set(["remote"]),
      requirementLabels: { remote: "--include-remote", hetzner: "--include-hetzner" },
    }).map((scenario) => scenario.id),
    ["local", "remote"],
  )

  assert.throws(
    () => selectDrillMatrixScenarios({
      scenarios,
      requestedIds: ["hetzner"],
      enabledRequirements: new Set(["remote"]),
      requirementLabels: { hetzner: "--include-hetzner" },
    }),
    /hetzner requires --include-hetzner/,
  )
})

test("quotes commands consistently", () => {
  assert.equal(quoteDrillCommand("node", ["plain", "two words", "quote'here"]), 'node plain "two words" "quote\'here"')
})

test("builds stable default matrix report paths", () => {
  const reportPath = defaultDrillMatrixReportPath("workspace live/sync matrix", {
    rootDir: "/repo",
    now: new Date("2026-06-14T00:00:01.234Z"),
  })

  assert.equal(
    reportPath,
    "/repo/.artifacts/drill-matrices/workspace-live-sync-matrix/2026-06-14T00-00-01-234Z.json",
  )
})

test("extracts artifact hints from structured and text drill output", () => {
  const hints = extractDrillArtifactHints([
    '[drill] preserved-failed-run {"rootDir":"/tmp/arroba-drill-one","manifestPath":"/tmp/arroba-drill-one/arroba-drill-failure.json","token":"secret"}',
    'remote workspace live sync permission drill artifacts kept at /tmp/arroba-drill-two',
    'ignored token=/not-an-artifact-token',
  ].join("\n"))

  assert.deepEqual(hints, [
    "/tmp/arroba-drill-one",
    "/tmp/arroba-drill-one/arroba-drill-failure.json",
    "/tmp/arroba-drill-two",
  ])
})

test("runs a passing matrix scenario", async () => {
  const dir = await fixtureDir()
  const script = await writeFixtureScript(dir, "pass.mjs", "console.log('ok')")
  const reportPath = path.join(dir, "reports", "matrix.json")

  const results = await runDrillMatrix({
    matrixName: "test-matrix",
    scenarios: [{ id: "pass", description: "passing scenario", script, exitCriteria: ["child command exits zero"] }],
    commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
    cwd: dir,
    reportPath,
  })

  assert.equal(results.length, 1)
  assert.equal(results[0].ok, true)
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  assert.equal(report.schema, "arroba.drill.matrix.v1")
  assert.equal(report.status, "passed")
  assert.equal(report.scenarios[0].id, "pass")
  assert.deepEqual(report.scenarios[0].exitCriteria, ["child command exits zero"])
  assert.equal(report.scenarios[0].command, process.execPath)
  await rm(dir, { recursive: true, force: true })
})

test("writes dry-run report without executing scenarios", async () => {
  const dir = await fixtureDir()
  const script = await writeFixtureScript(dir, "fail-if-executed.mjs", "process.exit(9)")
  const reportPath = path.join(dir, "dry-run.json")

  const results = await runDrillMatrix({
    matrixName: "test-matrix",
    scenarios: [{ id: "dry", description: "dry-run scenario", script }],
    commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
    cwd: dir,
    dryRun: true,
    reportPath,
  })

  assert.equal(results[0].dryRun, true)
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  assert.equal(report.status, "dry-run")
  assert.equal(report.scenarios[0].status, "dry-run")
  await rm(dir, { recursive: true, force: true })
})

test("treats matching expected failures as pass", async () => {
  const dir = await fixtureDir()
  const script = await writeFixtureScript(dir, "expected-failure.mjs", "console.error('managed mode needs selective write fencing'); process.exit(7)")

  const results = await runDrillMatrix({
    matrixName: "test-matrix",
    scenarios: [{
      id: "expected",
      description: "expected failure",
      script,
      expectedFailure: true,
      expectedOutputIncludes: "selective write fencing",
    }],
    commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
    cwd: dir,
  })

  assert.equal(results[0].ok, true)
  assert.equal(results[0].expectedFailure, true)
  assert.equal(results[0].classification, "expected-failure")
  await rm(dir, { recursive: true, force: true })
})

test("stops after the first unexpected failure unless configured otherwise", async () => {
  const dir = await fixtureDir()
  const fail = await writeFixtureScript(dir, "fail.mjs", "console.error('Token refresh failed: 401'); process.exit(3)")
  const pass = await writeFixtureScript(dir, "pass.mjs", "console.log('ok')")
  const reportPath = path.join(dir, "stopped.json")

  const results = await runDrillMatrix({
    matrixName: "test-matrix",
    scenarios: [
      { id: "fail", description: "failing scenario", script: fail },
      { id: "pass", description: "passing scenario", script: pass },
    ],
    commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
    cwd: dir,
    reportPath,
  })

  assert.equal(results.length, 2)
  assert.equal(results[0].ok, false)
  assert.equal(results[0].classification, "provider-auth")
  assert.equal(results[1].skipped, true)
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  assert.equal(report.status, "failed")
  assert.equal(report.scenarios[1].status, "skipped")
  assert.deepEqual(report.scenarios[0].artifactHints, [])
  await rm(dir, { recursive: true, force: true })
})

test("records artifact hints in failed scenario reports", async () => {
  const dir = await fixtureDir()
  const fail = await writeFixtureScript(dir, "fail-artifacts.mjs", "console.error('artifacts kept at /tmp/arroba-drill-failed'); process.exit(3)")
  const reportPath = path.join(dir, "artifacts.json")

  const results = await runDrillMatrix({
    matrixName: "test-matrix",
    scenarios: [{ id: "fail", description: "failing scenario", script: fail }],
    commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
    cwd: dir,
    reportPath,
  })

  assert.deepEqual(results[0].artifactHints, ["/tmp/arroba-drill-failed"])
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  assert.deepEqual(report.scenarios[0].artifactHints, ["/tmp/arroba-drill-failed"])
  await rm(dir, { recursive: true, force: true })
})

test("continues after failure when configured", async () => {
  const dir = await fixtureDir()
  const fail = await writeFixtureScript(dir, "fail.mjs", "process.exit(3)")
  const pass = await writeFixtureScript(dir, "pass.mjs", "console.log('ok')")

  const results = await runDrillMatrix({
    matrixName: "test-matrix",
    scenarios: [
      { id: "fail", description: "failing scenario", script: fail },
      { id: "pass", description: "passing scenario", script: pass },
    ],
    commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
    cwd: dir,
    continueOnFailure: true,
  })

  assert.equal(results.length, 2)
  assert.equal(results[0].ok, false)
  assert.equal(results[1].ok, true)
  assert.equal(results[1].skipped, undefined)
  await rm(dir, { recursive: true, force: true })
})

async function fixtureDir() {
  return await mkdtemp(path.join(os.tmpdir(), "arroba-drill-matrix-"))
}

async function writeFixtureScript(dir, name, contents) {
  const file = path.join(dir, name)
  await writeFile(file, `${contents}\n`, "utf8")
  return file
}
