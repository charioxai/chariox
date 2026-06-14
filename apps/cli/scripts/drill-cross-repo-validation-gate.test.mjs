import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile, mkdir } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex, writeDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
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
        scenario("slice-lifecycle", "slice-runtime", ["slice-runtime-state"]),
        scenario("provider-auth", "slice-auth", ["provider-run-lifecycle", "slice-auth-state"]),
        scenario("session-start", "kernel-authority", ["session-authority"]),
        scenario("agent-reuse", "worker-execution", ["agent-lifecycle"]),
        scenario("docker-browser-state", "docker-runtime", ["slice-runtime-state"]),
      ],
    })
    await writeMatrixReport(path.join(cloudRoot, ".artifacts", "drill-matrices", "cloud-slice-runtime.json"), {
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
        providerCount: 3,
        providers: "claude,codex,opencode",
        defaultModel: "provider-default",
        providerModelOverrides: "",
      },
      scenarios: [
        scenario("ui-projection", "ui-client-projection", ["client-projection-health"], { providers: ["claude", "codex", "opencode"] }),
      ],
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--platform-bundle",
      bundleDir,
      "--preset",
      "slice-runtime",
      "--require-runtime-signal",
      "slice-auth-state",
      "--require-matrix-runtime-signal",
      "slice-auth-state",
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
    assert.equal(report.checks.artifacts.status, "passed")
    assert.equal(report.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite.v1"], 1)
    assert.deepEqual(report.checks.artifacts.aggregate.indexes.map((index) => path.relative(cloudRoot, index.rootDir)), [
      path.join(".artifacts", "validation-suite"),
    ])
    assert.deepEqual(report.checks.matrices.requiredMatrices, ["slice-runtime-matrix"])
    assert.deepEqual(report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.deepEqual(report.checks.matrices.missingMatrixClassifications, [])
    assert.deepEqual(report.checks.platformBundle.requiredRuntimeSignals, [
      "agent-lifecycle",
      "client-projection-health",
      "provider-run-lifecycle",
      "session-authority",
      "slice-auth-state",
      "slice-runtime-state",
    ])
    assert.deepEqual(report.checks.platformBundle.missingRuntimeSignals, [])
    assert.deepEqual(report.checks.matrices.requiredMatrixRuntimeSignals, [
      "agent-lifecycle",
      "client-projection-health",
      "provider-run-lifecycle",
      "session-authority",
      "slice-auth-state",
      "slice-runtime-state",
    ])
    assert.deepEqual(report.checks.matrices.missingMatrixRuntimeSignals, [])
    assert.deepEqual(report.checks.matrices.aggregate.runtimeSignalScenarios["slice-auth-state"].map((entry) => entry.id), ["provider-auth"])
    assert.equal(report.checks.matrices.aggregate.matrixNames["slice-runtime-matrix"], 1)
    assert.equal(report.checks.matrices.aggregate.matrixNames["cloud-slice-runtime-matrix"], 1)
    assert.deepEqual(
      report.checks.matrices.aggregate.reports.find((entry) => entry.matrix === "cloud-slice-runtime-matrix").providers,
      ["claude", "codex", "opencode"],
    )
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["hosted-cloud"], 1)
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["local"], 1)
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["self-hosted-relay"], 1)
    assert.equal(artifactIndex.metadata.drill, "cross-repo-validation-gate")
    assert.equal(artifactIndex.metadata.status, "passed")
    assert.equal(artifactIndex.metadata.runtimeSignals, "agent-lifecycle,client-projection-health,provider-run-lifecycle,session-authority,slice-auth-state,slice-runtime-state")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate keeps default artifact roots opt-in", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    const skipped = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--json",
    ])).stdout)
    assert.equal(skipped.checks.artifacts.status, "skipped")

    const discovered = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--include-default-artifacts",
      "--json",
    ])).stdout)
    assert.equal(discovered.checks.artifacts.status, "passed")
    assert.equal(discovered.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite.v1"], 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate keeps default failure roots opt-in", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeFailureManifest(path.join(cloudRoot, ".artifacts", "failed-run", "arroba-drill-failure.json"), {
      drill: "cloud-slice-runtime-matrix",
      message: "relay target stale",
    })

    const skipped = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--json",
    ])).stdout)
    assert.equal(skipped.checks.failures.status, "skipped")

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--no-default-roots",
        "--include-default-failures",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.failures.status, "failed")
        assert.deepEqual(report.checks.failures.roots, [
          path.join(cloudRoot, ".artifacts"),
          path.join(ossRoot, ".artifacts"),
        ].sort())
        assert.equal(report.checks.failures.aggregate.total, 1)
        assert.equal(report.checks.failures.aggregate.failures[0].drill, "cloud-slice-runtime-matrix")
        return true
      },
    )
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
        scenario("slice-lifecycle", "slice-runtime", ["slice-runtime-state"]),
        scenario("provider-auth", "slice-auth", ["provider-run-lifecycle", "slice-auth-state"]),
        scenario("session-start", "kernel-authority", ["session-authority"]),
        scenario("agent-reuse", "worker-execution", ["agent-lifecycle"]),
        scenario("ui-projection", "ui-client-projection", ["client-projection-health"]),
        scenario("docker-browser-state", "docker-runtime", ["slice-runtime-state"]),
      ],
    })
    await writeMatrixReport(path.join(cloudMatrixRoot, "cloud-slice-runtime.json"), {
      matrix: "cloud-slice-runtime-matrix",
      metadata: {
        deploymentPresets: "hosted-cloud",
        providerCount: 3,
        providers: "claude,codex,opencode",
        defaultModel: "provider-default",
        providerModelOverrides: "",
      },
      scenarios: [
        scenario("hosted-slice-browser-e2e", "ui-client-projection", ["client-projection-health"], { providers: ["claude", "codex", "opencode"] }),
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

async function writeValidationSuiteArtifact(rootDir) {
  const artifactPath = path.join(rootDir, "cloud-validation-suite.json")
  await mkdir(rootDir, { recursive: true })
  await writeFile(artifactPath, `${JSON.stringify({
    schema: "arroba.drill.validation_suite.v1",
    testCount: 1,
    command: "node --test scripts/cloud-validation-suite.test.mjs",
    coverage: [{
      id: "suite-contract",
      description: "Cloud validation-suite contract",
      testCount: 1,
      testPaths: ["scripts/cloud-validation-suite.test.mjs"],
    }],
    testPaths: ["scripts/cloud-validation-suite.test.mjs"],
  }, null, 2)}\n`, "utf8")
  await writeDrillArtifactIndex({
    rootDir,
    artifacts: ["cloud-validation-suite.json"],
    metadata: { drill: "cloud-validation-suite", tests: 1 },
  })
}

async function writeFailureManifest(file, {
  drill = "failed-drill",
  message = "Token refresh failed: 401",
} = {}) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.failure.v1",
    rootDir: path.dirname(file),
    failedAt: "2026-06-13T00:00:00.000Z",
    metadata: { drill },
    error: { name: "Error", message, stack: null },
  }, null, 2)}\n`, "utf8")
}

function scenario(id, classification, runtimeSignals = [], overrides = {}) {
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
    runtimeSignals,
    ...overrides,
  }
}
