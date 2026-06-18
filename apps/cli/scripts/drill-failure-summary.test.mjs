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
    assert.deepEqual(aggregate.runtimeSignalOwners, {
      "kernel-authority": 2,
    })
    assert.deepEqual(fileAggregate, aggregate)
    assert.equal(artifactIndex.metadata.drill, "failure-summary")
    assert.equal(artifactIndex.metadata.total, 1)
    assert.equal(artifactIndex.metadata.owners, "runtime-network")
    assert.equal(artifactIndex.metadata.classifications, "relay-runtime")
    assert.equal(artifactIndex.metadata.runtimeSignals, "lease-health,session-authority")
    assert.equal(artifactIndex.metadata.runtimeSignalOwners, "kernel-authority")
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

test("failure summary gates stale failure manifests", async () => {
  const dir = await mkdtemp(path.join(os.tmpdir(), "arroba-failure-summary-"))
  try {
    const manifestPath = path.join(dir, "arroba-drill-failure.json")
    const failedAt = new Date(Date.now() - 500).toISOString()
    await writeManifest(manifestPath, failureManifest({
      rootDir: dir,
      drill: "stale-root",
      failedAt,
    }))

    const fresh = await runSummary([
      "--json",
      "--require-failure-max-age-ms",
      "3600000",
      manifestPath,
    ])
    assert.equal(fresh.requiredFailureMaxAgeMs, 3_600_000)
    assert.deepEqual(fresh.staleFailureManifests, [])

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--json",
        "--require-failure-max-age-ms",
        "100",
        manifestPath,
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const stale = JSON.parse(error.stdout)
        assert.equal(stale.requiredFailureMaxAgeMs, 100)
        assert.equal(stale.staleFailureManifests.length, 1)
        assert.equal(stale.staleFailureManifests[0].source, manifestPath)
        assert.deepEqual(
          stale.nextActions
            .filter(({ classification }) => classification === "failure-artifacts")
            .map(({ nextAction, count, sourceDetails }) => ({ nextAction, count, sourceDetails })),
          [{
            nextAction: "regenerate stale preserved failure bundles or rerun the failing drills before routing them",
            count: 1,
            sourceDetails: [{
              source: "stale-root",
              reportPath: manifestPath,
            }],
          }],
        )
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--require-failure-max-age-ms=100",
        manifestPath,
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /failure_required_max_age_ms=100 stale_manifests=1/)
        assert.match(error.stdout, /stale_failure_manifest=.*arroba-drill-failure\.json drill=stale-root/)
        assert.match(error.stdout, /sources: stale-root report=.*arroba-drill-failure\.json/)
        return true
      },
    )
  } finally {
    await rm(dir, { recursive: true, force: true })
  }
})

test("failure summary rejects invalid failure freshness age", async () => {
  await assert.rejects(
    () => execFile(process.execPath, [scriptPath, "--find", ".", "--require-failure-max-age-ms", "old"]),
    /--require-failure-max-age-ms must be a non-negative integer/,
  )
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

function failureManifest({ rootDir, drill, failedAt = "2026-06-13T00:00:00.000Z", runtimeSignals = null }) {
  return {
    schema: "arroba.drill.failure.v1",
    rootDir,
    failedAt,
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
