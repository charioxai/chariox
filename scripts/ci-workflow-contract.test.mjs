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


test("MP-08/MP-10 optimized kernel regression isolates the App test fixture override", () => {
  const start = workflow.indexOf("      - name: Release session project assignment regression")
  assert.ok(start >= 0)
  const end = workflow.indexOf("      - name:", start + 1)
  const step = workflow.slice(start, end < 0 ? undefined : end)
  assert.ok(step.includes("profile.release.package.chariox-kernel.codegen-units=256"))
  assert.ok(step.includes("profile.release.package.chariox-app-runtime.debug-assertions=true"))
  assert.ok(step.includes("test --release -p chariox-kernel --lib"))
  assert.ok(!step.includes("chariox-kernel.debug-assertions"))
  assert.ok(!step.includes("opt-level="))
})
