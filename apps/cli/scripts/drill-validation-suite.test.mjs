import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
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

test("drill validation suite checks configured paths", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--check"])

  assert.match(stdout, /validation suite paths ok/)
})
