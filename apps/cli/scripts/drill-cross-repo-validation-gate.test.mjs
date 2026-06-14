import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile, mkdir } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
import { writeDrillPlatformBundle } from "./lib/drill-platform-bundle.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-cross-repo-validation-gate.mjs", import.meta.url))

test("cross repo validation gate combines OSS and Cloud matrix evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    const outputPath = path.join(rootDir, "gate.json")
    const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
    await writeDrillPlatformBundle(bundleDir)
    await writeMatrixReport(path.join(ossRoot, ".artifacts", "drill-matrices", "slice-runtime.json"), {
      matrix: "slice-runtime-matrix",
      metadata: {
        deploymentPresets: "local,self-hosted-relay",
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("slice-lifecycle", "slice-runtime"),
        scenario("provider-auth", "slice-auth"),
        scenario("session-start", "kernel-authority"),
        scenario("agent-reuse", "worker-execution"),
        scenario("docker-browser-state", "docker-runtime"),
      ],
    })
    await writeMatrixReport(path.join(cloudRoot, ".artifacts", "drill-matrices", "cloud-slice-runtime.json"), {
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
      },
      scenarios: [
        scenario("ui-projection", "ui-client-projection"),
      ],
    })

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--platform-bundle",
      bundleDir,
      "--preset",
      "slice-runtime",
      "--require-complete",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const report = JSON.parse(stdout)
    const fileReport = JSON.parse(await readFile(outputPath, "utf8"))
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(fileReport, report)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.matrices.requiredMatrices, ["slice-runtime-matrix"])
    assert.deepEqual(report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.deepEqual(report.checks.matrices.missingMatrixClassifications, [])
    assert.equal(report.checks.matrices.aggregate.matrixNames["slice-runtime-matrix"], 1)
    assert.equal(report.checks.matrices.aggregate.matrixNames["cloud-slice-runtime-matrix"], 1)
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["hosted-cloud"], 1)
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["local"], 1)
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["self-hosted-relay"], 1)
    assert.equal(artifactIndex.metadata.drill, "cross-repo-validation-gate")
    assert.equal(artifactIndex.metadata.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate can disable default roots for focused evidence checks", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossMatrixRoot = path.join(rootDir, "oss-matrices")
    const cloudMatrixRoot = path.join(rootDir, "cloud-matrices")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeMatrixReport(path.join(ossMatrixRoot, "slice-runtime.json"), {
      matrix: "slice-runtime-matrix",
      metadata: {
        deploymentPresets: "local,self-hosted-relay",
        providers: "claude,codex,opencode",
      },
      scenarios: [
        scenario("slice-lifecycle", "slice-runtime"),
        scenario("provider-auth", "slice-auth"),
        scenario("session-start", "kernel-authority"),
        scenario("agent-reuse", "worker-execution"),
        scenario("ui-projection", "ui-client-projection"),
        scenario("docker-browser-state", "docker-runtime"),
      ],
    })
    await writeMatrixReport(path.join(cloudMatrixRoot, "cloud-slice-runtime.json"), {
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
      },
      scenarios: [
        scenario("hosted-slice-browser-e2e", "ui-client-projection"),
      ],
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--no-default-roots",
        "--matrix-root",
        ossMatrixRoot,
        "--platform-bundle",
        bundleDir,
        "--preset",
        "slice-runtime",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.matrices.missingDeploymentPresets, ["hosted-cloud"])
        return true
      },
    )

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--no-default-roots",
      "--matrix-root",
      ossMatrixRoot,
      "--matrix-root",
      cloudMatrixRoot,
      "--platform-bundle",
      bundleDir,
      "--preset",
      "slice-runtime",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

async function writeMatrixReport(file, { matrix, metadata, scenarios }) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.matrix.v1",
    matrix,
    status: "passed",
    dryRun: false,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata,
    scenarios,
  }, null, 2)}\n`, "utf8")
}

function scenario(id, classification) {
  return {
    id,
    description: `${id} scenario`,
    requires: [],
    exitCriteria: [],
    status: "passed",
    expectedFailure: false,
    classification,
    durationMs: 10,
    reason: null,
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
  }
}
