import test from "node:test"
import assert from "node:assert/strict"

import { classifyCommand } from "../src/controlled-exec-harness.mjs"
import { runFakeScenarios, runProviderScenarios } from "../src/scenarios.mjs"

test("classifyCommand detects deletes and redirects", () => {
  const classified = classifyCommand("echo hi > out.txt && rm -rf old", "/tmp/demo")
  assert.deepEqual(classified.writes, ["/tmp/demo/out.txt"])
  assert.deepEqual(classified.deletes, [])
})

test("fake scenarios pass", async () => {
  const result = await runFakeScenarios()
  assert.equal(result.passed, true)
  assert.equal(result.results.length >= 5, true)
})

test("provider scenario runner is exported for live drills", () => {
  assert.equal(typeof runProviderScenarios, "function")
})
