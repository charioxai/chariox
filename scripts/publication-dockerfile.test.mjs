import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { test } from "node:test"

const dockerfile = await readFile(new URL("../docker/publication/Dockerfile", import.meta.url), "utf8")
const workflowCode = await readFile(new URL("../apps/kernel/src/workflow_code.rs", import.meta.url), "utf8")

test("publication image copies compile-time workflow examples before building the kernel", () => {
  assert.match(workflowCode, /include_str!\("\.\.\/\.\.\/\.\.\/examples\/workflow-code\//)

  const rustStageStart = dockerfile.indexOf("FROM rust:1.88-bookworm AS rust-builder")
  const nextStageStart = dockerfile.indexOf("\nFROM ", rustStageStart + 1)
  assert.notEqual(rustStageStart, -1)
  assert.notEqual(nextStageStart, -1)

  const rustStage = dockerfile.slice(rustStageStart, nextStageStart)
  const examplesCopy = rustStage.indexOf("COPY examples/workflow-code examples/workflow-code")
  const kernelBuild = rustStage.indexOf("RUN cargo build --manifest-path apps/kernel/Cargo.toml")
  assert.ok(examplesCopy >= 0, "the Rust build stage must copy compile-time workflow examples")
  assert.ok(kernelBuild >= 0, "the Rust build stage must compile the kernel")
  assert.ok(examplesCopy < kernelBuild, "compile-time workflow examples must be copied before the kernel build")
})
