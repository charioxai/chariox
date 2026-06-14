import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex, writeDrillArtifactIndex } from "./lib/drill-artifacts.mjs"

const execFile = promisify(execFileWithCallback)
const scriptPath = fileURLToPath(new URL("./drill-distributed-runtime-gate.mjs", import.meta.url))

test("distributed runtime gate passes with complete OSS and Cloud matrix evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const outputPath = path.join(rootDir, "gate.json")
    const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), { evidenceRepo: "oss" })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--require-complete",
      "--require-runtime-signal",
      "slice-auth-state",
      "--require-matrix-runtime-signal",
      "slice-auth-state",
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
    assert.deepEqual(report.checks.artifacts.requiredArtifactCoverageAreas, ["distributed-observability"])
    assert.deepEqual(report.checks.artifacts.missingArtifactCoverageAreas, [])
    assert.equal(report.checks.artifacts.aggregate.coverageAreas["distributed-observability"], 2)
    assert.deepEqual(report.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
    assert.deepEqual(report.checks.artifacts.requiredArtifactKinds, ["validation-suite-run"])
    assert.deepEqual(report.checks.artifacts.requiredArtifactEvidenceRepos, ["cloud", "oss"])
    assert.deepEqual(report.checks.artifacts.missingArtifactEvidenceRepos, [])
    assert.deepEqual(report.presets, ["distributed-runtime"])
    assert.deepEqual(report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.deepEqual(report.checks.matrices.missingProviders, [])
    assert.deepEqual(report.checks.matrices.missingScenarios, [])
    assert.deepEqual(report.checks.platformBundle.missingRuntimeSignals, [])
    assert.ok(report.checks.platformBundle.requiredRuntimeSignals.includes("slice-auth-state"))
    assert.deepEqual(report.checks.matrices.missingMatrixRuntimeSignals, [])
    assert.ok(report.checks.matrices.requiredMatrixRuntimeSignals.includes("slice-auth-state"))
    assert.deepEqual(report.checks.matrices.aggregate.runtimeSignalScenarios["slice-auth-state"].map((entry) => entry.id), ["provider-auth"])
    assert.equal(report.checks.matrices.aggregate.matrixNames["cloud-slice-runtime-matrix"], 1)
    assert.deepEqual(
      report.checks.matrices.aggregate.reports.find((entry) => entry.matrix === "cloud-slice-runtime-matrix").providers,
      ["claude", "codex", "opencode"],
    )
    assert.equal(report.checks.matrices.aggregate.deploymentPresets["hosted-cloud"], 1)
    assert.equal(artifactIndex.metadata.drill, "distributed-runtime-gate")
    assert.equal(artifactIndex.metadata.preset, "distributed-runtime")
    assert.equal(artifactIndex.metadata.evidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.matrixEvidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.artifactEvidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.coverageAreas, "distributed-observability,suite-contract")
    const indexedRuntimeSignals = artifactIndex.metadata.runtimeSignals.split(",")
    assert.equal(indexedRuntimeSignals.includes("home-extension-manifest-sync"), true)
    assert.equal(indexedRuntimeSignals.includes("provider-run-lifecycle"), true)
    assert.equal(indexedRuntimeSignals.includes("slice-auth-state"), true)
    assert.equal(indexedRuntimeSignals.includes("workspace-live-sync-state"), true)
    const indexedRuntimeSignalOwners = artifactIndex.metadata.runtimeSignalOwners.split(",")
    assert.equal(indexedRuntimeSignalOwners.includes("kernel-authority"), true)
    assert.equal(indexedRuntimeSignalOwners.includes("provider-account"), true)
    assert.equal(indexedRuntimeSignalOwners.includes("provider-runtime"), true)
    assert.equal(indexedRuntimeSignalOwners.includes("runtime-state"), true)
    const indexedClassifications = artifactIndex.metadata.classifications.split(",")
    assert.equal(indexedClassifications.includes("kernel-authority"), true)
    assert.equal(indexedClassifications.includes("remote-extension-sync"), true)
    assert.equal(indexedClassifications.includes("workspace-live-sync-conflict"), true)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate requires default artifact indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      coverageAreas: ["suite-contract"],
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.artifacts.status, "failed")
        assert.deepEqual(report.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        assert.deepEqual(report.checks.artifacts.missingArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        return true
      },
    )

    const discovered = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--json",
    ])).stdout)
    assert.equal(discovered.status, "passed")
    assert.equal(discovered.checks.artifacts.status, "passed")
    assert.equal(discovered.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 2)
    assert.deepEqual(discovered.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
    assert.deepEqual(discovered.checks.artifacts.missingArtifactSchemas, [])
    assert.deepEqual(discovered.checks.artifacts.requiredArtifactEvidenceRepos, ["cloud", "oss"])
    assert.deepEqual(discovered.checks.artifacts.missingArtifactEvidenceRepos, [])
    assert.deepEqual(discovered.checks.artifacts.requiredArtifactCoverageAreas, ["distributed-observability"])
    assert.deepEqual(discovered.checks.artifacts.missingArtifactCoverageAreas, [])
    assert.deepEqual(discovered.checks.artifacts.aggregate.evidenceRepos, { cloud: 1, oss: 1 })
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate accepts explicit artifact evidence inputs", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const explicitArtifactRoot = path.join(rootDir, "validation-artifacts", "cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    const ossArtifactIndex = await writeValidationSuiteArtifact(path.join(rootDir, "validation-artifacts", "oss"), {
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(explicitArtifactRoot)

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--artifact-index",
      ossArtifactIndex,
      "--artifact-root",
      explicitArtifactRoot,
      "--json",
    ])).stdout)

    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.roots, [explicitArtifactRoot])
    assert.deepEqual(report.checks.artifacts.inputs, [ossArtifactIndex])
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.cloud, 1)
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.oss, 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can run validation suites as artifact evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const validationSuiteOutputRoot = path.join(rootDir, "generated-validation-suites")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeFakeValidationSuiteScript({
      classification: "validation-suite",
      evidenceRepo: "oss",
      file: path.join(ossRoot, "apps", "cli", "scripts", "drill-validation-suite.mjs"),
    })
    await writeFakeValidationSuiteScript({
      classification: "cloud-validation-suite",
      evidenceRepo: "cloud",
      file: path.join(cloudRoot, "scripts", "cloud-validation-suite.mjs"),
    })

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--run-validation-suites",
      "--validation-suite-output-root",
      validationSuiteOutputRoot,
      "--json",
    ])).stdout)

    const expectedArtifactIndexes = [
      path.join(validationSuiteOutputRoot, "oss", "arroba-drill-artifacts.json"),
      path.join(validationSuiteOutputRoot, "cloud", "arroba-drill-artifacts.json"),
    ]
    assert.equal(report.status, "passed")
    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.inputs, expectedArtifactIndexes)
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.cloud, 1)
    assert.equal(report.checks.artifacts.aggregate.evidenceRepos.oss, 1)
    assert.equal(report.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 2)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate requires executed Cloud validation suite artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteManifestArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.artifacts.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        assert.deepEqual(report.checks.artifacts.missingArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
        assert.equal(report.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite.v1"], 1)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate requires Cloud validation suite distributed observability coverage", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), {
      coverageAreas: ["suite-contract"],
      evidenceRepo: "oss",
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      coverageAreas: ["suite-contract"],
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.artifacts.status, "failed")
        assert.deepEqual(report.checks.artifacts.requiredArtifactCoverageAreas, ["distributed-observability"])
        assert.deepEqual(report.checks.artifacts.missingArtifactCoverageAreas, ["distributed-observability"])
        assert.match(report.checks.artifacts.error, /missing required artifact coverage areas: distributed-observability/)
        assert.equal(report.checks.artifacts.aggregate.coverageAreas["suite-contract"], 2)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate can include default failure manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), { evidenceRepo: "oss" })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    await writeFailureManifest(path.join(cloudRoot, ".artifacts", "failed-run", "arroba-drill-failure.json"), {
      drill: "cloud-slice-runtime-matrix",
      message: "slice auth stale projection",
    })

    const skipped = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--include-default-artifacts",
      "--json",
    ])).stdout)
    assert.equal(skipped.status, "passed")
    assert.equal(skipped.checks.failures.status, "skipped")

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
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

test("distributed runtime gate accepts explicit failure manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: true })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), { evidenceRepo: "oss" })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))
    const failureManifest = await writeFailureManifest(path.join(rootDir, "preserved", "arroba-drill-failure.json"), {
      drill: "remote-agent-runtime-matrix",
      message: "worker lease expired",
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--failure-manifest",
        failureManifest,
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.equal(report.checks.failures.status, "failed")
        assert.deepEqual(report.checks.failures.inputs, [failureManifest])
        assert.equal(report.checks.failures.aggregate.total, 1)
        assert.equal(report.checks.failures.aggregate.failures[0].drill, "remote-agent-runtime-matrix")
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate reports missing hosted Cloud evidence", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-distributed-runtime-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud: false })
    await writeValidationSuiteArtifact(path.join(ossRoot, ".artifacts", "validation-suite"), { evidenceRepo: "oss" })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"))

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--include-default-artifacts",
        "--json",
      ]),
      (error) => {
        const report = JSON.parse(error.stdout)
        assert.equal(error.code, 1)
        assert.equal(report.status, "failed")
        assert.deepEqual(report.checks.matrices.missingDeploymentPresets, ["hosted-cloud"])
        assert.deepEqual(report.checks.matrices.missingScenarios, ["ui-projection"])
        assert.deepEqual(report.nextActions.map(({ owner, classification }) => ({ owner, classification })), [
          { owner: "validation-harness", classification: "matrix-coverage" },
          { owner: "validation-harness", classification: "matrix-coverage" },
          { owner: "validation-harness", classification: "matrix-coverage" },
        ])
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("distributed runtime gate rejects output artifact index without output", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--output-artifact-index", "/tmp/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /requires --output/)
      return true
    },
  )
})

test("distributed runtime gate rejects requirement flags without values", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-runtime-signal", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-runtime-signal requires a value/)
      return true
    },
  )
})

async function writeDistributedRuntimeMatrices({ ossRoot, cloudRoot, includeCloud }) {
  const ossMatrixRoot = path.join(ossRoot, ".artifacts", "drill-matrices")
  await writeMatrixReport(path.join(ossMatrixRoot, "native-provider-tui.json"), {
    matrix: "native-provider-tui-matrix",
    metadata: {
      deploymentPresets: "hetzner,local,same-host-remote,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("local-native-tui", "kernel-authority", ["provider-run-lifecycle", "session-authority"]),
      scenario("permission-visibility", "ui-client-projection", ["permission-interaction"]),
      scenario("remote-native-tui", "relay-runtime", ["provider-run-lifecycle", "session-authority"]),
      scenario("slice-native-tui", "worker-execution", ["provider-run-lifecycle", "session-authority"]),
      scenario("transcript-parity", "provider-error", ["client-projection-health"]),
      scenario("provider-auth-health", "provider-auth", ["provider-run-lifecycle"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "remote-agent-runtime.json"), {
    matrix: "remote-agent-runtime-matrix",
    metadata: {
      deploymentPresets: "hetzner,same-host-remote,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("collab-remote-agent", "kernel-authority", ["lease-health", "session-authority"]),
      scenario("lease-reconnect", "relay-target-freshness", ["lease-health", "relay-target-freshness"]),
      scenario("provider-run-binding", "worker-execution", ["lease-health", "provider-run-lifecycle"]),
      scenario("remote-prompt-dispatch", "relay-runtime", ["agent-lifecycle", "provider-run-lifecycle"]),
      scenario("single-user-remote-agent", "ui-client-projection", ["agent-lifecycle", "client-projection-health", "session-authority"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "remote-home-extension.json"), {
    matrix: "remote-home-extension-matrix",
    metadata: {
      deploymentPresets: "hetzner,local,self-hosted-relay",
    },
    scenarios: [
      scenario("local-single", "remote-extension-sync", ["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"]),
      scenario("local-collab", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
      scenario("hetzner-single", "worker-execution", ["home-extension-manifest-sync", "lease-health", "provider-run-lifecycle", "session-authority"]),
      scenario("hetzner-collab", "kernel-authority", ["home-extension-manifest-sync", "lease-health", "session-authority"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "slice-runtime.json"), {
    matrix: "slice-runtime-matrix",
    metadata: {
      deploymentPresets: "local,self-hosted-relay",
      providers: "claude,codex,opencode",
    },
    scenarios: [
      scenario("agent-reuse", "worker-execution", ["agent-lifecycle", "slice-runtime-state"]),
      scenario("docker-browser-state", "docker-runtime", ["slice-runtime-state"]),
      scenario("provider-auth", "slice-auth", ["provider-run-lifecycle", "slice-auth-state"]),
      scenario("session-start", "kernel-authority", ["session-authority", "slice-runtime-state"]),
      scenario("slice-lifecycle", "slice-runtime", ["slice-runtime-state"]),
    ],
  })
  await writeMatrixReport(path.join(ossMatrixRoot, "workspace-live-sync.json"), {
    matrix: "workspace-live-sync-matrix",
    metadata: {
      deploymentPresets: "hetzner,local,same-host-remote,self-hosted-relay",
      providers: "codex,opencode",
    },
    scenarios: [
      scenario("local-managed-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("local-tracked-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("local-permission-codex", "kernel-authority", ["session-authority", "workspace-live-sync-state"]),
      scenario("remote-managed-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("remote-tracked-codex", "workspace-live-sync-conflict", ["session-authority", "workspace-live-sync-state"]),
      scenario("remote-tracked-restart-codex", "relay-target-freshness", ["relay-target-freshness", "session-authority", "workspace-live-sync-state"]),
    ],
  })

  if (includeCloud) {
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
  }
}

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

async function writeValidationSuiteArtifact(rootDir, {
  coverageAreas = ["distributed-observability", "suite-contract"],
  evidenceRepo = "cloud",
} = {}) {
  const artifactPath = path.join(rootDir, "cloud-validation-suite.json")
  await mkdir(rootDir, { recursive: true })
  await writeFile(artifactPath, `${JSON.stringify({
    schema: "arroba.drill.validation_suite_run.v1",
    status: "passed",
    ok: true,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    exitCode: 0,
    signal: null,
    error: null,
    testCount: 1,
    command: "node --test scripts/cloud-validation-suite.test.mjs",
    testPaths: ["scripts/cloud-validation-suite.test.mjs"],
    manifest: {
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
    },
  }, null, 2)}\n`, "utf8")
  await writeDrillArtifactIndex({
    rootDir,
    artifacts: ["cloud-validation-suite.json"],
    metadata: {
      drill: "cloud-validation-suite",
      tests: 1,
      coverageAreas: coverageAreas.join(","),
      runtimeSignals: DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS.join(","),
      owners: "validation-platform",
      classifications: evidenceRepo === "cloud" ? "cloud-validation-suite" : "validation-suite",
      artifactKinds: "validation-suite-run",
      evidenceRepos: evidenceRepo,
    },
  })
  return path.join(rootDir, "arroba-drill-artifacts.json")
}

const DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS = Object.freeze([
  "agent-lifecycle",
  "client-projection-health",
  "home-extension-manifest-sync",
  "lease-health",
  "permission-interaction",
  "provider-run-lifecycle",
  "relay-target-freshness",
  "session-authority",
  "slice-auth-state",
  "slice-runtime-state",
  "workspace-live-sync-state",
])

async function writeValidationSuiteManifestArtifact(rootDir) {
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
    metadata: {
      drill: "cloud-validation-suite",
      tests: 1,
      artifactKinds: "validation-suite",
      evidenceRepos: "cloud",
    },
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
  return file
}

async function writeFakeValidationSuiteScript({
  classification,
  evidenceRepo,
  file,
}) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `#!/usr/bin/env node
import { createHash } from "node:crypto"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import path from "node:path"

const args = process.argv.slice(2)
const outputPath = valueFor("--output")
const artifactIndexPath = valueFor("--output-artifact-index")
const outputDir = path.dirname(outputPath)
await mkdir(outputDir, { recursive: true })
const report = {
  schema: "arroba.drill.validation_suite_run.v1",
  status: "passed",
  ok: true,
  startedAt: "2026-06-13T00:00:00.000Z",
  completedAt: "2026-06-13T00:00:01.000Z",
  durationMs: 1000,
  exitCode: 0,
  signal: null,
  error: null,
  command: "node --test fake-validation-suite.test.mjs",
  testCount: 1,
  testPaths: ["fake-validation-suite.test.mjs"],
  manifest: {
    schema: "arroba.drill.validation_suite.v1",
    testCount: 1,
    command: "node --test fake-validation-suite.test.mjs",
    coverage: [
      {
        id: "distributed-observability",
        description: "Distributed observability evidence.",
        testCount: 1,
        testPaths: ["fake-validation-suite.test.mjs"],
      },
      {
        id: "suite-contract",
        description: "Suite contract evidence.",
        testCount: 1,
        testPaths: ["fake-validation-suite.test.mjs"],
      },
    ],
    testPaths: ["fake-validation-suite.test.mjs"],
  },
}
await writeFile(outputPath, \`\${JSON.stringify(report, null, 2)}\\n\`, "utf8")
const bytes = await readFile(outputPath)
const index = {
  schema: "arroba.drill.artifact_index.v1",
  rootDir: outputDir,
  createdAt: "2026-06-13T00:00:02.000Z",
  metadata: {
    drill: "validation-suite",
    tests: 1,
    coverageAreas: "distributed-observability,suite-contract",
    runtimeSignals: ${JSON.stringify(DISTRIBUTED_RUNTIME_ARTIFACT_SIGNALS.join(","))},
    runtimeSignalOwners: "kernel-authority,provider-account,provider-runtime,runtime-network,runtime-state,ui-client,worker-kernel",
    owners: "validation-platform",
    classifications: ${JSON.stringify(classification)},
    artifactKinds: "validation-suite-run",
    evidenceRepos: ${JSON.stringify(evidenceRepo)},
  },
  artifacts: [{
    path: path.basename(outputPath),
    schema: report.schema,
    sha256: createHash("sha256").update(bytes).digest("hex"),
    sizeBytes: bytes.byteLength,
  }],
}
await writeFile(artifactIndexPath, \`\${JSON.stringify(index, null, 2)}\\n\`, "utf8")

function valueFor(flag) {
  const index = args.indexOf(flag)
  if (index < 0 || !args[index + 1]) throw new Error(\`\${flag} requires a value\`)
  return args[index + 1]
}
`, "utf8")
}

function scenario(id, classification, runtimeSignals = [], overrides = {}) {
  return {
    id,
    description: `${id} scenario`,
    status: "passed",
    ok: true,
    expectedFailure: false,
    classification,
    durationMs: 10,
    reason: null,
    requires: [],
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
    exitCriteria: [`${id} exit criteria`],
    runtimeSignals,
    ...overrides,
  }
}
