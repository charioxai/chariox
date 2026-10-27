import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-failure-taxonomy.mjs", import.meta.url))

test("drill failure taxonomy prints scenario manifest", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath])
  const manifest = JSON.parse(stdout)

  assert.equal(manifest.schema, "chariox.drill.failure_taxonomy.v1")
  assert.equal(manifest.target, "scenario")
  assert(manifest.classifications.some((entry) => (
    entry.kind === "kernel-authority"
      && entry.owner === "kernel-authority"
      && entry.nextAction.endsWith("rerunning the scenario")
  )))
})

test("drill failure taxonomy prints drill-target next actions", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath, "--target", "drill"])
  const manifest = JSON.parse(stdout)

  assert.equal(manifest.target, "drill")
  assert(manifest.classifications.some((entry) => (
    entry.kind === "kernel-authority"
      && entry.nextAction.endsWith("rerunning the drill")
  )))
})

test("drill failure taxonomy writes manifest", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "chariox-failure-taxonomy-"))
  const outputPath = path.join(rootDir, "taxonomy.json")
  const artifactIndexPath = path.join(rootDir, "chariox-drill-artifacts.json")
  try {
    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--target",
      "drill",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const stdoutManifest = JSON.parse(stdout)
    const fileManifest = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileManifest, stdoutManifest)
    assert.equal(fileManifest.target, "drill")
    assert.equal(artifactIndex.metadata.drill, "failure-taxonomy")
    assert.equal(artifactIndex.metadata.target, "drill")
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "taxonomy.json",
      schema: "chariox.drill.failure_taxonomy.v1",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill failure taxonomy rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/chariox-drill-artifacts.json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})
