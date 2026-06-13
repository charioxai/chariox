import assert from "node:assert/strict"
import { mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
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

test("runs a passing matrix scenario", async () => {
  const dir = await fixtureDir()
  const script = await writeFixtureScript(dir, "pass.mjs", "console.log('ok')")

  const results = await runDrillMatrix({
    matrixName: "test-matrix",
    scenarios: [{ id: "pass", description: "passing scenario", script }],
    commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
    cwd: dir,
  })

  assert.equal(results.length, 1)
  assert.equal(results[0].ok, true)
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

  const results = await runDrillMatrix({
    matrixName: "test-matrix",
    scenarios: [
      { id: "fail", description: "failing scenario", script: fail },
      { id: "pass", description: "passing scenario", script: pass },
    ],
    commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
    cwd: dir,
  })

  assert.equal(results.length, 1)
  assert.equal(results[0].ok, false)
  assert.equal(results[0].classification, "provider-auth")
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
