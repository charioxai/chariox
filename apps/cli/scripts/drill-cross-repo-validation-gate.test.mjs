import assert from "node:assert/strict"
import { execFile as execFileWithCallback } from "node:child_process"
import { mkdtemp, readFile, rm, writeFile, mkdir } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"
import { promisify } from "node:util"

import { verifyDrillArtifactIndex, writeDrillArtifactIndex } from "./lib/drill-artifacts.mjs"
import { drillFailureTaxonomyManifest } from "./lib/drill-failure-taxonomy.mjs"
import { writeDrillPlatformBundle } from "./lib/drill-platform-bundle.mjs"
import { drillRuntimeSignalsManifest } from "./lib/drill-runtime-signals.mjs"

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
        scenario("ui-projection", "ui-client-projection", ["client-projection-health", "runtime-projection-health"], { providers: ["claude", "codex", "opencode"] }),
      ],
    })
    await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      metadata: { providerAccountAliases: "codex=work" },
    })

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
      "--require-artifact-provider-account-alias",
      "codex=work",
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
    assert.equal(report.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 1)
    assert.deepEqual(report.checks.artifacts.requiredArtifactProviderAccountAliases, ["codex=work"])
    assert.deepEqual(report.checks.artifacts.missingArtifactProviderAccountAliases, [])
    assert.deepEqual(report.checks.artifacts.aggregate.providerAccountAliases, { "codex=work": 1 })
    assert.deepEqual(report.checks.artifacts.aggregate.indexes.map((index) => path.relative(cloudRoot, index.rootDir)), [
      path.join(".artifacts", "validation-suite"),
    ])
    assert.deepEqual(report.checks.matrices.requiredMatrices, ["cloud-slice-runtime-matrix", "slice-runtime-matrix"])
    assert.deepEqual(report.checks.matrices.missingMatrices, [])
    assert.deepEqual(report.checks.matrices.missingDeploymentPresets, [])
    assert.deepEqual(report.checks.matrices.missingMatrixClassifications, [])
    assert.deepEqual(report.checks.platformBundle.requiredRuntimeSignals, [
      "agent-lifecycle",
      "client-projection-health",
      "provider-run-lifecycle",
      "runtime-projection-health",
      "session-authority",
      "slice-auth-state",
      "slice-runtime-state",
    ])
    assert.deepEqual(report.checks.platformBundle.missingRuntimeSignals, [])
    assert.deepEqual(report.checks.matrices.requiredMatrixRuntimeSignals, [
      "agent-lifecycle",
      "client-projection-health",
      "provider-run-lifecycle",
      "runtime-projection-health",
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
    assert.equal(artifactIndex.metadata.evidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.providerAccountAliases, "codex=work")
    assert.equal(artifactIndex.metadata.matrixEvidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.artifactEvidenceRepos, "cloud")
    assertMetadataIncludes(artifactIndex.metadata.runtimeSignals, [
      "agent-lifecycle",
      "home-extension-manifest-sync",
      "provider-run-lifecycle",
      "session-authority",
      "slice-auth-state",
      "workspace-live-sync-state",
    ])
    assertMetadataIncludes(artifactIndex.metadata.runtimeSignalOwners, [
      "kernel-authority",
      "provider-account",
      "provider-runtime",
      "runtime-state",
      "ui-client",
      "worker-kernel",
    ])
    assertMetadataIncludes(artifactIndex.metadata.classifications, [
      "docker-runtime",
      "kernel-authority",
      "remote-extension-sync",
      "slice-auth",
      "slice-runtime",
      "ui-client-projection",
      "workspace-live-sync-conflict",
    ])
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

function assertMetadataIncludes(value, expected) {
  const actual = new Set(String(value ?? "").split(",").filter(Boolean))
  for (const entry of expected) {
    assert.equal(actual.has(entry), true, `expected metadata to include ${entry}`)
  }
}

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
    assert.equal(discovered.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate accepts explicit artifact evidence inputs", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    const artifactRoot = path.join(rootDir, "artifact-root")
    await writeDrillPlatformBundle(bundleDir)
    const artifactIndex = await writeValidationSuiteArtifact(artifactRoot)

    const report = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--artifact-index",
      artifactIndex,
      "--json",
    ])).stdout)

    assert.equal(report.checks.artifacts.status, "passed")
    assert.deepEqual(report.checks.artifacts.inputs, [artifactIndex])
    assert.deepEqual(report.checks.artifacts.roots, [])
    assert.equal(report.checks.artifacts.aggregate.schemas["arroba.drill.validation_suite_run.v1"], 1)
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

test("cross repo validation gate accepts explicit failure manifests", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    const failureManifest = await writeFailureManifest(path.join(rootDir, "preserved", "arroba-drill-failure.json"), {
      drill: "slice-runtime-matrix",
      message: "slice launch timed out",
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--no-default-roots",
        "--platform-bundle",
        bundleDir,
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
        assert.deepEqual(report.checks.failures.roots, [])
        assert.equal(report.checks.failures.aggregate.total, 1)
        assert.equal(report.checks.failures.aggregate.failures[0].drill, "slice-runtime-matrix")
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
        scenario("ui-projection", "ui-client-projection", ["client-projection-health", "runtime-projection-health"]),
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
        scenario("hosted-slice-browser-e2e", "ui-client-projection", ["client-projection-health", "runtime-projection-health"], { providers: ["claude", "codex", "opencode"] }),
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

test("cross repo validation gate checks generated matrix registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeCloudGeneratedMatrixRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--require-generated-matrix-registry-parity",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate rejects generated matrix registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    await writeCloudGeneratedMatrixRegistry(cloudRoot, {
      matrices: [
        { name: "cloud-slice-runtime-matrix", repo: "cloud" },
        { name: "slice-runtime-matrix", repo: "oss" },
      ],
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--no-default-roots",
        "--require-generated-matrix-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /generated matrix registry parity failed/)
        assert.match(error.stderr, /workspace-live-sync-matrix/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate checks runtime signal registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeCloudRuntimeSignalsRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--require-runtime-signal-registry-parity",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate checks failure taxonomy registry parity", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const bundleDir = path.join(rootDir, "bundle")
    await writeDrillPlatformBundle(bundleDir)
    await writeCloudFailureTaxonomyRegistry(cloudRoot)

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--cloud-root",
      cloudRoot,
      "--no-default-roots",
      "--platform-bundle",
      bundleDir,
      "--require-failure-taxonomy-registry-parity",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate rejects failure taxonomy registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillFailureTaxonomyManifest()
    await writeCloudFailureTaxonomyRegistry(cloudRoot, {
      classifications: [
        ...manifest.classifications.filter((classification) => classification.kind === "kernel-authority"),
        {
          kind: "future-cloud-only-classification",
          owner: "kernel-authority",
          nextAction: "inspect future diagnostics",
        },
      ],
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--no-default-roots",
        "--require-failure-taxonomy-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /failure taxonomy registry parity failed/)
        assert.match(error.stderr, /future-cloud-only-classification/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate rejects runtime signal registry drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const manifest = drillRuntimeSignalsManifest()
    await writeCloudRuntimeSignalsRegistry(cloudRoot, {
      signals: manifest.signals.map((signal) => signal.id === "workspace-live-sync-state"
        ? { ...signal, owner: "kernel-authority" }
        : signal),
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--cloud-root",
        cloudRoot,
        "--no-default-roots",
        "--require-runtime-signal-registry-parity",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /runtime signal registry parity failed/)
        assert.match(error.stderr, /workspace-live-sync-state/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate requires artifact generated matrix limitation metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const artifactIndexPath = await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      metadata: { generatedMatrixLimitations: "dry-run-classification-coverage" },
    })

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--artifact-index",
        artifactIndexPath,
        "--require-artifact-generated-matrix-limitation",
        "dry-run-classification-covergae",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /unknown required artifact generated matrix limitation: dry-run-classification-covergae/)
        return true
      },
    )

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--no-default-roots",
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--artifact-index",
      artifactIndexPath,
      "--require-artifact-generated-matrix-limitation",
      "dry-run-classification-coverage",
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.artifacts.requiredArtifactGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
    assert.deepEqual(report.checks.artifacts.missingArtifactGeneratedMatrixLimitations, [])
    assert.equal(report.checks.artifacts.aggregate.generatedMatrixLimitations["dry-run-classification-coverage"], 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate requires artifact generated matrix artifact-index metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const generatedMatrixArtifactIndex = path.join(rootDir, "generated-matrix", "workspace-live-sync-matrix-artifacts.json")
    const artifactIndexPath = await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      metadata: { generatedMatrixArtifactIndexes: generatedMatrixArtifactIndex },
    })

    const missingGeneratedMatrixArtifactIndex = path.join(rootDir, "generated-matrix", "missing-matrix-artifacts.json")
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--no-default-roots",
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--artifact-index",
        artifactIndexPath,
        "--require-artifact-generated-matrix-artifact-index",
        missingGeneratedMatrixArtifactIndex,
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const failed = JSON.parse(error.stdout)
        assert.equal(failed.status, "failed")
        assert.deepEqual(failed.checks.artifacts.missingArtifactGeneratedMatrixArtifactIndexes, [
          missingGeneratedMatrixArtifactIndex,
        ])
        return true
      },
    )

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--no-default-roots",
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--artifact-index",
      artifactIndexPath,
      "--require-artifact-generated-matrix-artifact-index",
      generatedMatrixArtifactIndex,
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.artifacts.requiredArtifactGeneratedMatrixArtifactIndexes, [generatedMatrixArtifactIndex])
    assert.deepEqual(report.checks.artifacts.missingArtifactGeneratedMatrixArtifactIndexes, [])
    assert.equal(report.checks.artifacts.aggregate.generatedMatrixArtifactIndexes[generatedMatrixArtifactIndex], 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate requires artifact generated validation-suite failure-root metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-cross-repo-gate-"))
  try {
    const ossRoot = path.join(rootDir, "arroba")
    const cloudRoot = path.join(rootDir, "arroba-cloud")
    const generatedFailureRoot = path.join(rootDir, "generated-suite", "failed-run")
    const artifactIndexPath = await writeValidationSuiteArtifact(path.join(cloudRoot, ".artifacts", "validation-suite"), {
      metadata: { generatedValidationSuiteFailureRoots: generatedFailureRoot },
    })

    const missingGeneratedFailureRoot = path.join(rootDir, "generated-suite", "missing-run")
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--no-default-roots",
        "--oss-root",
        ossRoot,
        "--cloud-root",
        cloudRoot,
        "--artifact-index",
        artifactIndexPath,
        "--require-artifact-generated-validation-suite-failure-root",
        missingGeneratedFailureRoot,
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const failed = JSON.parse(error.stdout)
        assert.equal(failed.status, "failed")
        assert.deepEqual(failed.checks.artifacts.missingArtifactGeneratedValidationSuiteFailureRoots, [
          missingGeneratedFailureRoot,
        ])
        return true
      },
    )

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--no-default-roots",
      "--oss-root",
      ossRoot,
      "--cloud-root",
      cloudRoot,
      "--artifact-index",
      artifactIndexPath,
      "--require-artifact-generated-validation-suite-failure-root",
      generatedFailureRoot,
      "--json",
    ])
    const report = JSON.parse(stdout)
    assert.equal(report.status, "passed")
    assert.deepEqual(report.checks.artifacts.requiredArtifactGeneratedValidationSuiteFailureRoots, [generatedFailureRoot])
    assert.deepEqual(report.checks.artifacts.missingArtifactGeneratedValidationSuiteFailureRoots, [])
    assert.equal(report.checks.artifacts.aggregate.generatedValidationSuiteFailureRoots[generatedFailureRoot], 1)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("cross repo validation gate rejects aggregate-only generated evidence requirements", async () => {
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-failure-max-age-ms", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-failure-max-age-ms requires a value/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-evidence-kind", "matrix-report", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-evidence-kind is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-matrix-artifact-index", "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-matrix-artifact-index is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-matrix-limitation", "dry-run-classification-coverage", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-matrix-limitation is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-validation-suite-artifact-index", "/tmp/generated-suite/arroba-drill-artifacts.json", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-validation-suite-artifact-index is supported by drill-validation-gate-summary\.mjs/)
      return true
    },
  )
  await assert.rejects(
    execFile(process.execPath, [scriptPath, "--require-generated-validation-suite-failure-root", "/tmp/generated-suite/failed-run", "--json"]),
    (error) => {
      assert.equal(error.code, 1)
      assert.match(error.stderr, /--require-generated-validation-suite-failure-root is supported by drill-validation-gate-summary\.mjs/)
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

async function writeCloudFailureTaxonomyRegistry(cloudRoot, {
  classifications = drillFailureTaxonomyManifest().classifications
    .filter((classification) => [
      "docker-runtime",
      "kernel-authority",
      "runtime-projection-health",
      "workspace-live-sync-conflict",
    ].includes(classification.kind))
    .map((classification) => classification.kind === "docker-runtime"
      ? { ...classification, owner: "worker-kernel" }
      : classification),
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-failure-taxonomy.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudFailureTaxonomyManifest() {",
    `  return { schema: "arroba.drill.failure_taxonomy.v1", target: "scenario", classifications: ${JSON.stringify(classifications)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

async function writeValidationSuiteArtifact(rootDir, { metadata = {} } = {}) {
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
      coverageAreas: "distributed-observability,suite-contract",
      ...metadata,
    },
  })
  return path.join(rootDir, "arroba-drill-artifacts.json")
}

async function writeCloudGeneratedMatrixRegistry(cloudRoot, {
  matrices = [
    { name: "cloud-slice-runtime-matrix", repo: "cloud" },
    { name: "native-provider-tui-matrix", repo: "oss" },
    { name: "remote-agent-runtime-matrix", repo: "oss" },
    { name: "remote-home-extension-matrix", repo: "oss" },
    { name: "slice-runtime-matrix", repo: "oss" },
    { name: "workspace-live-sync-matrix", repo: "oss" },
  ],
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-drill-generated-matrix-names.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudDrillGeneratedMatrixNamesManifest() {",
    `  return { schema: "arroba.cloud.drill.generated_matrix_names.v1", matrices: ${JSON.stringify(matrices)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
}

async function writeCloudRuntimeSignalsRegistry(cloudRoot, {
  signals = drillRuntimeSignalsManifest().signals,
} = {}) {
  const registryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-signals.mjs")
  await mkdir(path.dirname(registryPath), { recursive: true })
  await writeFile(registryPath, [
    "export function cloudRuntimeSignalsManifest() {",
    `  return { schema: "arroba.drill.runtime_signals.v1", signals: ${JSON.stringify(signals)} }`,
    "}",
    "",
  ].join("\n"), "utf8")
  return registryPath
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
