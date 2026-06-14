import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import {
  verifyDrillArtifactIndex,
  writeDrillArtifactIndex,
} from "./lib/drill-artifacts.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-artifact-index-summary.mjs", import.meta.url))

test("drill artifact index summary aggregates discovered indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const firstIndexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")
    const secondIndexPath = await writeIndexedReport(rootDir, "two", "arroba.drill.matrix.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-root",
      rootDir,
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const stdoutAggregate = JSON.parse(stdout)
    const fileAggregate = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileAggregate, stdoutAggregate)
    assert.equal(stdoutAggregate.schema, "arroba.drill.artifact_index.aggregate.v1")
    assert.equal(stdoutAggregate.totals.indexes, 2)
    assert.equal(stdoutAggregate.totals.artifacts, 2)
    assert(stdoutAggregate.totals.sizeBytes > 0)
    assert.deepEqual(stdoutAggregate.runtimeSignals, {
      "lease-health": 1,
      "session-authority": 2,
      "workspace-live-sync-state": 1,
    })
    assert.deepEqual(stdoutAggregate.indexes.map((index) => index.source), [
      firstIndexPath,
      secondIndexPath,
    ])
    assert.equal(artifactIndex.metadata.drill, "artifact-index-summary")
    assert.equal(artifactIndex.metadata.indexes, 2)
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [{
      path: "aggregate.json",
      schema: "arroba.drill.artifact_index.aggregate.v1",
    }])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

test("drill artifact index summary accepts explicit index paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--json",
    ])
    const aggregate = JSON.parse(stdout)

    assert.equal(aggregate.totals.indexes, 1)
    assert.equal(aggregate.totals.artifacts, 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary rejects empty inputs", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /no drill artifact indexes found/)
      return true
    },
  )
})

test("drill artifact index summary rejects tampered artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")
    await writeFile(path.join(rootDir, "one", "reports", "report.json"), "{\"schema\":\"tampered\"}\n", "utf8")

    await assert.rejects(
      execFile(process.execPath, [scriptPath, "--artifact-index", indexPath, "--json"]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /sha256 mismatch/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeIndexedReport(rootDir, name, schema) {
  const drillRoot = path.join(rootDir, name)
  await mkdir(path.join(drillRoot, "reports"), { recursive: true })
  await writeFile(path.join(drillRoot, "reports", "report.json"), `${JSON.stringify({ schema })}\n`, "utf8")
  await writeDrillArtifactIndex({
    rootDir: drillRoot,
    artifacts: ["reports/report.json"],
    metadata: {
      runtimeSignals: name === "one"
        ? "session-authority,lease-health"
        : "session-authority,workspace-live-sync-state",
    },
  })
  return path.join(drillRoot, "arroba-drill-artifacts.json")
}
