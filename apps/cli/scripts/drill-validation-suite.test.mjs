import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

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

  assert.equal(manifest.schema, "arroba.drill.validation_suite.v1")
  assert.equal(manifest.testCount, SHARED_DRILL_TEST_PATHS.length)
  assert.deepEqual(manifest.testPaths, SHARED_DRILL_TEST_PATHS)
  assert.match(manifest.command, /^node --test /)
})

test("drill validation suite writes coverage manifest", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-suite-"))
  const outputPath = path.join(rootDir, "suite.json")
  try {
    const { stdout } = await execFile(process.execPath, [scriptPath, "--json", "--output", outputPath])
    const stdoutManifest = JSON.parse(stdout)
    const fileManifest = JSON.parse(await readFile(outputPath, "utf8"))

    assert.deepEqual(fileManifest, stdoutManifest)
    assert.equal(fileManifest.schema, "arroba.drill.validation_suite.v1")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill validation suite checks configured paths", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--check"])

  assert.match(stdout, /validation suite paths ok/)
})
