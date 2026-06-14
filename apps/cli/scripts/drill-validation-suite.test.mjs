import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
import { DRILL_RUNTIME_SIGNAL_IDS } from "./lib/drill-runtime-signals.mjs"
import { SHARED_DRILL_TEST_PATHS } from "./lib/drill-validation-suite.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-validation-suite.mjs", import.meta.url))

test("drill validation suite lists selected tests", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--list"])

  assert.deepEqual(stdout.trim().split("\n"), SHARED_DRILL_TEST_PATHS)
})

test("drill validation suite prints runnable command", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--command"])

  assert.match(stdout, /^node --test /)
  assert.match(stdout, /apps\/cli\/scripts\/lib\/drill-matrix-runner\.test\.mjs/)
})

test("drill validation suite prints coverage manifest", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--json"])
  const manifest = JSON.parse(stdout)
  const covered = manifest.coverage.flatMap((area) => area.testPaths)

  assert.equal(manifest.schema, "arroba.drill.validation_suite.v1")
  assert.equal(manifest.testCount, SHARED_DRILL_TEST_PATHS.length)
  assert.deepEqual(manifest.testPaths, SHARED_DRILL_TEST_PATHS)
  assert.deepEqual(covered.sort(), [...SHARED_DRILL_TEST_PATHS])
  assert.deepEqual(manifest.coverage.map((area) => area.id), [
    "distributed-observability",
    "artifact-contracts",
    "failure-diagnostics",
    "matrix-validation",
    "runtime-fixtures",
    "suite-contract",
  ])
  assert.deepEqual(manifest.validationPresets.map((preset) => preset.name), [
    "distributed-runtime",
    "native-provider-tui",
    "remote-agent-runtime",
    "remote-home-extension",
    "slice-runtime",
    "workspace-live-sync",
  ])
  assert.deepEqual(
    manifest.validationPresets.find((preset) => preset.name === "distributed-runtime").requiredMatrices,
    ["cloud-slice-runtime-matrix", "native-provider-tui-matrix", "remote-agent-runtime-matrix", "remote-home-extension-matrix", "slice-runtime-matrix", "workspace-live-sync-matrix"],
  )
  assert.deepEqual(
    manifest.validationPresets.find((preset) => preset.name === "distributed-runtime").requiredArtifactSchemas,
    ["arroba.drill.validation_suite_run.v1"],
  )
  assert.deepEqual(
    manifest.validationPresets.find((preset) => preset.name === "workspace-live-sync").requiredMatrices,
    ["workspace-live-sync-matrix"],
  )
  assert.deepEqual(
    manifest.validationPresets.find((preset) => preset.name === "native-provider-tui").requiredDeploymentPresets,
    ["hetzner", "local", "same-host-remote", "self-hosted-relay"],
  )
  assert.deepEqual(
    manifest.validationPresets.find((preset) => preset.name === "remote-agent-runtime").requiredScenarios,
    ["collab-remote-agent", "lease-reconnect", "provider-run-binding", "remote-prompt-dispatch", "single-user-remote-agent"],
  )
  assert.deepEqual(
    manifest.validationPresets.find((preset) => preset.name === "slice-runtime").requiredScenarios,
    ["agent-reuse", "provider-auth", "session-start", "slice-lifecycle", "ui-projection"],
  )
  assert.match(manifest.command, /^node --test /)
})

test("drill validation suite writes coverage manifest", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-suite-"))
  const outputPath = path.join(rootDir, "suite.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const stdoutManifest = JSON.parse(stdout)
    const fileManifest = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileManifest, stdoutManifest)
    assert.equal(fileManifest.schema, "arroba.drill.validation_suite.v1")
    assert.equal(artifactIndex.metadata.drill, "validation-suite")
    assert.equal(artifactIndex.metadata.tests, SHARED_DRILL_TEST_PATHS.length)
    assert.equal(artifactIndex.metadata.owners, "validation-platform")
    assert.equal(artifactIndex.metadata.classifications, "validation-suite")
    assert.equal(artifactIndex.metadata.coverageAreas, "artifact-contracts,distributed-observability,failure-diagnostics,matrix-validation,runtime-fixtures,suite-contract")
    assert.equal(artifactIndex.metadata.validationPresets, "distributed-runtime,native-provider-tui,remote-agent-runtime,remote-home-extension,slice-runtime,workspace-live-sync")
    assert.equal(artifactIndex.metadata.runtimeSignals, DRILL_RUNTIME_SIGNAL_IDS.join(","))
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "suite.json",
      schema: "arroba.drill.validation_suite.v1",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation suite writes passing run report artifact output", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-suite-"))
  const testPath = path.join(rootDir, "passing.test.mjs")
  const outputPath = path.join(rootDir, "suite-run.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    await writeFile(testPath, [
      'import test from "node:test"',
      'import assert from "node:assert/strict"',
      'test("passes", () => assert.equal(1, 1))',
      "",
    ].join("\n"), "utf8")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--run-json",
      "--test-path",
      testPath,
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const stdoutReport = JSON.parse(stdout)
    const fileReport = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileReport, stdoutReport)
    assert.equal(fileReport.schema, "arroba.drill.validation_suite_run.v1")
    assert.equal(fileReport.status, "passed")
    assert.equal(fileReport.ok, true)
    assert.equal(fileReport.testCount, 1)
    assert.deepEqual(fileReport.testPaths, [testPath])
    assert.equal(fileReport.manifest.schema, "arroba.drill.validation_suite.v1")
    assert.equal(artifactIndex.metadata.drill, "validation-suite")
    assert.equal(artifactIndex.metadata.status, "passed")
    assert.equal(artifactIndex.metadata.tests, 1)
    assert.equal(artifactIndex.metadata.owners, "validation-platform")
    assert.equal(artifactIndex.metadata.classifications, "validation-suite")
    assert.equal(artifactIndex.metadata.coverageAreas, "custom-suite")
    assert.equal(artifactIndex.metadata.runtimeSignals, undefined)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation suite writes failing run report before exiting nonzero", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-suite-"))
  const testPath = path.join(rootDir, "failing.test.mjs")
  const outputPath = path.join(rootDir, "suite-run.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    await writeFile(testPath, [
      'import test from "node:test"',
      'import assert from "node:assert/strict"',
      'test("fails", () => assert.equal(1, 2))',
      "",
    ].join("\n"), "utf8")

    let rejected = null
    try {
      await execFile(process.execPath, [
        scriptPath,
        "--run-json",
        "--test-path",
        testPath,
        "--output",
        outputPath,
        "--output-artifact-index",
        artifactIndexPath,
      ])
    } catch (error) {
      rejected = error
    }
    assert(rejected)
    assert.equal(rejected.code, 1)
    const stdoutReport = JSON.parse(rejected.stdout)
    const fileReport = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileReport, stdoutReport)
    assert.equal(fileReport.schema, "arroba.drill.validation_suite_run.v1")
    assert.equal(fileReport.status, "failed")
    assert.equal(fileReport.ok, false)
    assert.equal(fileReport.exitCode, 1)
    assert.equal(artifactIndex.metadata.status, "failed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation suite rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--json", "--output-artifact-index", "/tmp/arroba-drill-artifacts.json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

test("drill validation suite checks configured paths", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--check"])

  assert.match(stdout, /validation suite paths ok/)
})
