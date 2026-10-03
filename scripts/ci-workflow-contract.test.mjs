import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const workflow = await readFile(new URL("../.github/workflows/ci.yml", import.meta.url), "utf8")

test("CI exposes exactly one manual dispatch trigger", () => {
  const declarations = workflow.match(/^  workflow_dispatch:\s*$/gm) ?? []
  assert.equal(declarations.length, 1)
})

test("MP-01/MP-03 Rust CI installs Bubblewrap before isolation tests", () => {
  const rustJob = workflow.slice(workflow.indexOf("  rust:"))
  const install = rustJob.indexOf("sudo apt-get install -y bubblewrap")
  const firstTest = rustJob.indexOf("cargo test")
  assert.ok(install >= 0, "the runner must explicitly provide the product launcher")
  assert.ok(install < firstTest, "the launcher must be installed before Rust tests")
})
