import assert from "node:assert/strict"
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  defaultDrillMatrixArtifactIndexPath,
  defaultDrillMatrixReportPath,
  extractDrillArtifactHints,
  parseDrillScenarioIds,
  quoteDrillCommand,
  runDrillMatrix,
  selectDrillMatrixScenarios,
  validateDrillMatrixScenarioDefinitions,
} from "./drill-matrix-runner.mjs"
import { verifyDrillArtifactIndex } from "./drill-artifacts.mjs"

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

test("validates matrix scenario definitions", () => {
  validateDrillMatrixScenarioDefinitions([{
    id: "local",
    description: "local scenario",
    requires: ["remote"],
    exitCriteria: ["runtime path is exercised"],
    expectedFailure: false,
    classification: "kernel-authority",
  }])

  assert.throws(() => validateDrillMatrixScenarioDefinitions([]), /must not be empty/)
  assert.throws(() => validateDrillMatrixScenarioDefinitions([{ id: "one", description: "first" }, { id: "one", description: "second" }]), /duplicate matrix scenario id/)
  assert.throws(() => validateDrillMatrixScenarioDefinitions([{ id: "", description: "missing id" }]), /missing id/)
  assert.throws(() => validateDrillMatrixScenarioDefinitions([{ id: "local" }]), /missing description/)
  assert.throws(() => validateDrillMatrixScenarioDefinitions([{ id: "local", description: "bad requires", requires: [1] }]), /invalid requires/)
  assert.throws(() => validateDrillMatrixScenarioDefinitions([{ id: "local", description: "bad criteria", exitCriteria: [""] }]), /invalid exitCriteria/)
  assert.throws(() => validateDrillMatrixScenarioDefinitions([{ id: "local", description: "bad expected failure", expectedFailure: "yes" }]), /invalid expectedFailure/)
  assert.throws(() => validateDrillMatrixScenarioDefinitions([{ id: "local", description: "bad expected output", expectedOutputIncludes: "" }]), /invalid expectedOutputIncludes/)
  assert.throws(() => validateDrillMatrixScenarioDefinitions([{ id: "local", description: "bad classification", classification: "" }]), /invalid classification/)
  assert.throws(() => validateDrillMatrixScenarioDefinitions([{ id: "local", description: "unknown classification", classification: "not-real" }]), /unknown classification "not-real"/)

  assert.doesNotThrow(() => validateDrillMatrixScenarioDefinitions([{ id: "selection-only" }], { requireDescription: false }))
})

test("rejects malformed selected matrix scenarios before running", async () => {
  const dir = await fixtureDir()
  let called = false

  await assert.rejects(
    () => runDrillMatrix({
      matrixName: "test-matrix",
      scenarios: [{ id: "bad" }],
      commandForScenario: () => {
        called = true
        return { command: process.execPath, args: ["should-not-run.mjs"] }
      },
      cwd: dir,
    }),
    /missing description/,
  )
  assert.equal(called, false)
  await rm(dir, { recursive: true, force: true })
})

test("rejects artifact index output without a matrix report path", async () => {
  const dir = await fixtureDir()
  const script = await writeFixtureScript(dir, "pass.mjs", "console.log('ok')")

  await assert.rejects(
    () => runDrillMatrix({
      matrixName: "test-matrix",
      scenarios: [{ id: "pass", description: "passing scenario", script }],
      commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
      cwd: dir,
      artifactIndexPath: path.join(dir, "arroba-drill-artifacts.json"),
    }),
    /artifactIndexPath requires reportPath/,
  )
  await rm(dir, { recursive: true, force: true })
})

test("rejects malformed matrix commands before spawning", async () => {
  const dir = await fixtureDir()

  await assert.rejects(
    () => runDrillMatrix({
      matrixName: "test-matrix",
      scenarios: [{ id: "bad-command", description: "bad command" }],
      commandForScenario: () => ({ command: "", args: [] }),
      cwd: dir,
    }),
    /bad-command command is missing command/,
  )
  await assert.rejects(
    () => runDrillMatrix({
      matrixName: "test-matrix",
      scenarios: [{ id: "bad-args", description: "bad args" }],
      commandForScenario: () => ({ command: process.execPath, args: [1] }),
      cwd: dir,
    }),
    /bad-args command has invalid args/,
  )
  await assert.rejects(
    () => runDrillMatrix({
      matrixName: "test-matrix",
      scenarios: [{ id: "bad-env", description: "bad env" }],
      commandForScenario: () => ({ command: process.execPath, args: [], env: { ARROBA_TEST: 42 } }),
      cwd: dir,
    }),
    /bad-env command has invalid env/,
  )
  await rm(dir, { recursive: true, force: true })
})

test("rejects later malformed matrix commands before spawning earlier scenarios", async () => {
  const dir = await fixtureDir()
  const marker = path.join(dir, "should-not-exist")
  const script = await writeFixtureScript(dir, "mark.mjs", `import { writeFileSync } from "node:fs"; writeFileSync(${JSON.stringify(marker)}, "ran")`)

  await assert.rejects(
    () => runDrillMatrix({
      matrixName: "test-matrix",
      scenarios: [
        { id: "first", description: "would write marker", script },
        { id: "second", description: "bad later command" },
      ],
      commandForScenario: (scenario) => scenario.id === "first"
        ? { command: process.execPath, args: [scenario.script] }
        : { command: "", args: [] },
      cwd: dir,
    }),
    /second command is missing command/,
  )
  assert.equal(await exists(marker), false)
  await rm(dir, { recursive: true, force: true })
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

test("builds stable default matrix artifact index paths", () => {
  assert.equal(
    defaultDrillMatrixArtifactIndexPath("/repo/.artifacts/drill-matrices/test-matrix/2026-06-14T00-00-01-234Z.json"),
    "/repo/.artifacts/drill-matrices/test-matrix/2026-06-14T00-00-01-234Z-artifacts/arroba-drill-artifacts.json",
  )
  assert.throws(() => defaultDrillMatrixArtifactIndexPath(""), /reportPath is required/)
})

test("extracts artifact hints from structured and text drill output", () => {
  const hints = extractDrillArtifactHints([
    '[drill] preserved-failed-run {"rootDir":"/tmp/arroba-drill-one","manifestPath":"/tmp/arroba-drill-one/arroba-drill-failure.json","token":"secret"}',
    'remote workspace live sync permission drill artifacts kept at /tmp/arroba-drill-two',
    'bad artifacts kept at /tmp/arroba-drill-sk-this-should-not-persist',
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
  const logs = []
  const originalLog = console.log
  console.log = (...args) => logs.push(args.join(" "))

  let results
  try {
    results = await runDrillMatrix({
      matrixName: "test-matrix",
      scenarios: [{
        id: "pass",
        description: "passing scenario",
        script,
        exitCriteria: ["child command exits zero"],
        runtimeSignals: ["session-authority", "provider-run-lifecycle"],
      }],
      commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
      cwd: dir,
      reportPath,
    })
  } finally {
    console.log = originalLog
  }

  assert.equal(results.length, 1)
  assert.equal(results[0].ok, true)
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  assert.equal(report.schema, "arroba.drill.matrix.v1")
  assert.equal(report.status, "passed")
  assert.equal(report.scenarios[0].id, "pass")
  assert.deepEqual(report.scenarios[0].exitCriteria, ["child command exits zero"])
  assert.deepEqual(report.scenarios[0].exitCriteriaEvidence, [{
    id: "pass:exit-01",
    criterion: "child command exits zero",
    status: "satisfied",
    reason: null,
  }])
  assert.deepEqual(report.scenarios[0].runtimeSignals, ["provider-run-lifecycle", "session-authority"])
  assert.equal(report.metadata.runtimeSignals, "provider-run-lifecycle,session-authority")
  assert.equal(report.metadata.runtimeSignalOwners, "kernel-authority,provider-runtime")
  assert.equal(report.scenarios[0].command, process.execPath)
  assert(logs.some((line) => line.includes("runtime_signals provider-run-lifecycle=1 session-authority=1")))
  assert(logs.some((line) => line.includes("runtime_signal_owners kernel-authority=1 provider-runtime=1")))
  await rm(dir, { recursive: true, force: true })
})

test("passes per-scenario environment to matrix commands", async () => {
  const dir = await fixtureDir()
  const marker = path.join(dir, "env-marker.txt")
  const script = await writeFixtureScript(dir, "env.mjs", [
    'import { writeFileSync } from "node:fs"',
    `writeFileSync(${JSON.stringify(marker)}, process.env.ARROBA_MATRIX_ENV_MARKER ?? "")`,
  ].join("\n"))
  const reportPath = path.join(dir, "env-report.json")

  const results = await runDrillMatrix({
    matrixName: "test-matrix",
    scenarios: [{ id: "env", description: "env scenario", script }],
    commandForScenario: (scenario) => ({
      command: process.execPath,
      args: [scenario.script],
      env: { ARROBA_MATRIX_ENV_MARKER: "from-scenario" },
    }),
    cwd: dir,
    reportPath,
  })

  assert.equal(results[0].ok, true)
  assert.equal(await readFile(marker, "utf8"), "from-scenario")
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  assert.equal(report.scenarios[0].command, process.execPath)
  assert.deepEqual(report.scenarios[0].args, [script])
  assert.equal(Object.hasOwn(report.scenarios[0], "env"), false)
  await rm(dir, { recursive: true, force: true })
})

test("preserves diagnostic classification for passing matrix scenarios", async () => {
  const dir = await fixtureDir()
  const script = await writeFixtureScript(dir, "pass.mjs", "console.log('ok')")
  const reportPath = path.join(dir, "classified.json")

  const results = await runDrillMatrix({
    matrixName: "test-matrix",
    scenarios: [{
      id: "classified",
      description: "passing classified scenario",
      script,
      classification: "remote-extension-sync",
    }],
    commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
    cwd: dir,
    reportPath,
  })

  assert.equal(results[0].ok, true)
  assert.equal(results[0].classification, "remote-extension-sync")
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  assert.equal(report.status, "passed")
  assert.equal(report.scenarios[0].classification, "remote-extension-sync")
  assert.equal(report.scenarios[0].owner, "kernel-authority")
  assert.match(report.scenarios[0].nextAction, /remote extension manifest sync status/)
  await rm(dir, { recursive: true, force: true })
})

test("writes artifact index for matrix reports", async () => {
  const dir = await fixtureDir()
  const script = await writeFixtureScript(dir, "pass.mjs", "console.log('ok')")
  const reportPath = path.join(dir, "reports", "matrix.json")
  const artifactIndexPath = path.join(dir, "reports", "arroba-drill-artifacts.json")

  await runDrillMatrix({
    matrixName: "test-matrix",
    scenarios: [{
      id: "pass",
      description: "passing scenario",
      script,
      runtimeSignals: ["lease-health", "session-authority"],
    }],
    commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
    cwd: dir,
    reportPath,
    artifactIndexPath,
  })

  const index = await verifyDrillArtifactIndex(artifactIndexPath)
  assert.equal(index.metadata.matrix, "test-matrix")
  assert.equal(index.metadata.status, "passed")
  assert.equal(index.metadata.dryRun, false)
  assert.equal(index.metadata.scenarios, 1)
  assert.equal(index.metadata.runtimeSignals, "lease-health,session-authority")
  assert.equal(index.metadata.runtimeSignalOwners, "kernel-authority")
  assert.deepEqual(index.artifacts.map((artifact) => ({
    path: artifact.path,
    schema: artifact.schema,
  })), [{
    path: "matrix.json",
    schema: "arroba.drill.matrix.v1",
  }])
  await rm(dir, { recursive: true, force: true })
})

test("writes dry-run report without executing scenarios", async () => {
  const dir = await fixtureDir()
  const script = await writeFixtureScript(dir, "fail-if-executed.mjs", "process.exit(9)")
  const reportPath = path.join(dir, "dry-run.json")
  const logs = []
  const originalLog = console.log
  console.log = (...args) => logs.push(args.join(" "))

  let results
  try {
    results = await runDrillMatrix({
      matrixName: "test-matrix",
      scenarios: [{ id: "dry", description: "dry-run scenario", script, classification: "kernel-authority", exitCriteria: "dry command is selected" }],
      commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
      cwd: dir,
      dryRun: true,
      reportPath,
    })
  } finally {
    console.log = originalLog
  }

  assert.equal(results[0].dryRun, true)
  assert(logs.some((line) => line.includes("dry-run dry classification=kernel-authority")))
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  assert.equal(report.status, "dry-run")
  assert.equal(report.scenarios[0].status, "dry-run")
  assert.equal(report.scenarios[0].classification, null)
  assert.deepEqual(report.scenarios[0].exitCriteriaEvidence, [{
    id: "dry:exit-01",
    criterion: "dry command is selected",
    status: "dry-run",
    reason: "scenario command was selected but not executed",
  }])
  await rm(dir, { recursive: true, force: true })
})

test("refuses to write matrix reports with secret metadata", async () => {
  const dir = await fixtureDir()
  const script = await writeFixtureScript(dir, "pass.mjs", "console.log('ok')")
  const reportPath = path.join(dir, "secret.json")

  await assert.rejects(
    () => runDrillMatrix({
      matrixName: "test-matrix",
      scenarios: [{ id: "pass", description: "passing scenario", script }],
      commandForScenario: (scenario) => ({ command: process.execPath, args: [scenario.script] }),
      cwd: dir,
      reportPath,
      metadata: { apiKey: "sk-this-should-not-be-written" },
    }),
    /sensitive metadata key/,
  )
  assert.equal(await exists(reportPath), false)
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
      { id: "fail", description: "failing scenario", script: fail, exitCriteria: ["failing command exits zero"] },
      { id: "pass", description: "passing scenario", script: pass, exitCriteria: ["passing command runs after failure"] },
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
  assert.equal(report.scenarios[0].owner, "provider-account")
  assert.equal(report.scenarios[0].nextAction, "refresh provider login for the profile used by this drill, then rerun the scenario")
  assert.deepEqual(report.scenarios[0].exitCriteriaEvidence, [{
    id: "fail:exit-01",
    criterion: "failing command exits zero",
    status: "failed",
    reason: "code=3 signal=none",
  }])
  assert.deepEqual(report.scenarios[1].exitCriteriaEvidence, [{
    id: "pass:exit-01",
    criterion: "passing command runs after failure",
    status: "skipped",
    reason: "skipped after previous failure",
  }])
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

async function exists(file) {
  return Boolean(await stat(file).catch(() => null))
}
