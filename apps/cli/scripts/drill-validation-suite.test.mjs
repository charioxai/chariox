import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
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
    "artifact-contracts",
    "failure-diagnostics",
    "matrix-validation",
    "runtime-fixtures",
    "suite-contract",
  ])
  assert.deepEqual(manifest.validationPresets.map((preset) => preset.name), [
    "native-provider-tui",
    "remote-home-extension",
    "slice-runtime",
    "workspace-live-sync",
  ])
  assert.deepEqual(
    manifest.validationPresets.find((preset) => preset.name === "workspace-live-sync").requiredMatrices,
    ["workspace-live-sync-matrix"],
  )
  assert.deepEqual(
    manifest.validationPresets.find((preset) => preset.name === "native-provider-tui").requiredDeploymentPresets,
    ["hetzner", "local", "same-host-remote", "self-hosted-relay"],
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
