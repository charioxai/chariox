import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-failure-taxonomy.mjs", import.meta.url))

test("drill failure taxonomy prints scenario manifest", async () => {
  const { stdout } = await execFile(process.execPath, [scriptPath])
  const manifest = JSON.parse(stdout)

  assert.equal(manifest.schema, "arroba.drill.failure_taxonomy.v1")
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
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-failure-taxonomy-"))
  const outputPath = path.join(rootDir, "taxonomy.json")
  try {
    const { stdout } = await execFile(process.execPath, [scriptPath, "--output", outputPath])
    const stdoutManifest = JSON.parse(stdout)
    const fileManifest = JSON.parse(await readFile(outputPath, "utf8"))

    assert.deepEqual(fileManifest, stdoutManifest)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})
