import assert from "node:assert/strict"
import test from "node:test"

import {
  SHARED_DRILL_TEST_PATHS,
  drillValidationSuiteArgs,
  drillValidationSuiteCommand,
} from "./drill-validation-suite.mjs"

test("shared drill validation suite lists stable test paths", () => {
  assert(SHARED_DRILL_TEST_PATHS.includes("apps/cli/scripts/lib/drill-matrix-runner.test.mjs"))
  assert(SHARED_DRILL_TEST_PATHS.includes("apps/cli/scripts/drill-matrix-report-summary.test.mjs"))
  assert.deepEqual([...SHARED_DRILL_TEST_PATHS].sort(), [...SHARED_DRILL_TEST_PATHS])
  assert.equal(new Set(SHARED_DRILL_TEST_PATHS).size, SHARED_DRILL_TEST_PATHS.length)
})

test("formats shared drill validation suite command", () => {
  assert.deepEqual(drillValidationSuiteArgs({ testPaths: ["one.test.mjs", "two words.test.mjs"] }), [
    "--test",
    "one.test.mjs",
    "two words.test.mjs",
  ])
  assert.equal(
    drillValidationSuiteCommand({ nodeCommand: "node", testPaths: ["one.test.mjs", "two words.test.mjs"] }),
    'node --test one.test.mjs "two words.test.mjs"',
  )
})
