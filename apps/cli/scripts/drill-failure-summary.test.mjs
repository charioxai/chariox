import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-failure-summary.mjs", import.meta.url))

test("failure summary max-depth limits manifest discovery", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-failure-summary-"))
  const rootManifest = path.join(dir, "arroba-drill-failure.json")
  const nestedManifest = path.join(dir, ".artifacts", "run", "arroba-drill-failure.json")
  await writeManifest(rootManifest, failureManifest({ rootDir: dir, drill: "root" }))
  await writeManifest(nestedManifest, failureManifest({ rootDir: path.dirname(nestedManifest), drill: "nested" }))

  const shallow = await runSummary(["--find", dir, "--max-depth", "0", "--json"])
  const broad = await runSummary(["--find", dir, "--json"])

  assert.equal(shallow.total, 1)
  assert.deepEqual(shallow.failures.map((failure) => failure.source), [rootManifest])
  assert.equal(broad.total, 2)
  assert.deepEqual(broad.failures.map((failure) => failure.source), [nestedManifest, rootManifest].sort())

  await rm(dir, { recursive: true, force: true })
})

test("failure summary rejects invalid max-depth", async () => {
  await assert.rejects(
    () => execFile(process.execPath, [scriptPath, "--find", ".", "--max-depth", "nope"]),
    /--max-depth must be a non-negative integer/,
  )
})

test("failure summary writes artifact index for output", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-failure-summary-"))
  try {
    const manifestPath = path.join(dir, "arroba-drill-failure.json")
    const outputPath = path.join(dir, "aggregate.json")
    const artifactIndexPath = path.join(dir, "arroba-drill-artifacts.json")
    await writeManifest(manifestPath, failureManifest({
      rootDir: dir,
      drill: "root",
      runtimeSignals: "lease-health,session-authority",
    }))

    const aggregate = await runSummary([
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
      manifestPath,
    ])
    const fileAggregate = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.equal(aggregate.total, 1)
    assert.deepEqual(fileAggregate, aggregate)
    assert.equal(artifactIndex.metadata.drill, "failure-summary")
    assert.equal(artifactIndex.metadata.total, 1)
    assert.equal(artifactIndex.metadata.owners, "runtime-network")
    assert.equal(artifactIndex.metadata.classifications, "relay-runtime")
    assert.equal(artifactIndex.metadata.runtimeSignals, "lease-health,session-authority")
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "aggregate.json",
      schema: "arroba.drill.failure.aggregate.v1",
    }])
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test("failure summary rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

async function runSummary(args) {
  const { stdout } = await execFile(process.execPath, [scriptPath, ...args])
  return JSON.parse(stdout)
}

async function writeManifest(file, manifest) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(manifest)}\n`, "utf8")
}

function failureManifest({ rootDir, drill, runtimeSignals = null }) {
  return {
    schema: "arroba.drill.failure.v1",
    rootDir,
    failedAt: "2026-06-13T00:00:00.000Z",
    metadata: {
      drill,
      ...(runtimeSignals ? { runtimeSignals } : {}),
    },
    error: {
      name: "Error",
      message: "relay target stale",
      stack: null,
    },
  }
}
