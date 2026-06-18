import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { verifyDrillArtifactIndex } from "./drill-artifacts.mjs"
import {
  DRILL_PLATFORM_BUNDLE_ARTIFACTS,
  DRILL_PLATFORM_BUNDLE_SCHEMA,
  verifyDrillPlatformBundle,
  writeDrillPlatformBundle,
} from "./drill-platform-bundle.mjs"
import { DRILL_RUNTIME_SIGNAL_IDS } from "./drill-runtime-signals.mjs"

test("defines stable drill platform bundle artifacts", () => {
  assert.deepEqual(DRILL_PLATFORM_BUNDLE_ARTIFACTS, [
    {
      path: "failure-taxonomy-drill.json",
      schema: "arroba.drill.failure_taxonomy.v1",
    },
    {
      path: "failure-taxonomy-scenario.json",
      schema: "arroba.drill.failure_taxonomy.v1",
    },
    {
      path: "generated-matrix-limitations.json",
      schema: "arroba.drill.generated_matrix_limitations.v1",
    },
    {
      path: "generated-matrix-names.json",
      schema: "arroba.drill.generated_matrix_names.v1",
    },
    {
      path: "runtime-signals.json",
      schema: "arroba.drill.runtime_signals.v1",
    },
    {
      path: "validation-suite.json",
      schema: "arroba.drill.validation_suite.v1",
    },
  ])
})

test("writes and verifies drill platform bundle artifacts", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    const bundle = await writeDrillPlatformBundle(rootDir)
    const verified = await verifyDrillPlatformBundle(rootDir)
    const artifactIndex = await verifyDrillArtifactIndex(path.join(rootDir, "arroba-drill-artifacts.json"))
    const generatedMatrixLimitations = JSON.parse(await readFile(path.join(rootDir, "generated-matrix-limitations.json"), "utf8"))
    const generatedMatrixNames = JSON.parse(await readFile(path.join(rootDir, "generated-matrix-names.json"), "utf8"))
    const validationSuite = JSON.parse(await readFile(path.join(rootDir, "validation-suite.json"), "utf8"))

    assert.equal(bundle.schema, DRILL_PLATFORM_BUNDLE_SCHEMA)
    assert.deepEqual(verified, bundle)
    assert.equal(artifactIndex.metadata.drill, "platform-bundle")
    assert.equal(generatedMatrixLimitations.schema, "arroba.drill.generated_matrix_limitations.v1")
    assert.deepEqual(generatedMatrixLimitations.limitations.map((limitation) => limitation.kind), ["dry-run-classification-coverage"])
    assert.equal(generatedMatrixNames.schema, "arroba.drill.generated_matrix_names.v1")
    assert.deepEqual(generatedMatrixNames.matrices, [
      { name: "cloud-slice-runtime-matrix", repo: "cloud" },
      { name: "native-provider-tui-matrix", repo: "oss" },
      { name: "remote-agent-runtime-matrix", repo: "oss" },
      { name: "remote-home-extension-matrix", repo: "oss" },
      { name: "slice-runtime-matrix", repo: "oss" },
      { name: "workspace-live-sync-matrix", repo: "oss" },
    ])
    assert.equal(validationSuite.coverage.length, 6)
    assert.deepEqual(validationSuite.coverage.map((area) => area.id), [
      "distributed-observability",
      "artifact-contracts",
      "failure-diagnostics",
      "matrix-validation",
      "runtime-fixtures",
      "suite-contract",
    ])
    assert.deepEqual(validationSuite.validationPresets.map((preset) => preset.name), [
      "distributed-runtime",
      "native-provider-tui",
      "remote-agent-runtime",
      "remote-home-extension",
      "slice-runtime",
      "workspace-live-sync",
    ])
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "distributed-runtime").requiredMatrices,
      ["cloud-slice-runtime-matrix", "native-provider-tui-matrix", "remote-agent-runtime-matrix", "remote-home-extension-matrix", "slice-runtime-matrix", "workspace-live-sync-matrix"],
    )
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "distributed-runtime").requiredArtifactGeneratedEvidenceRepos,
      ["cloud", "oss"],
    )
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "distributed-runtime").requiredArtifactRuntimeSignals,
      DRILL_RUNTIME_SIGNAL_IDS,
    )
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "distributed-runtime").requiredArtifactRuntimeSignalOwners,
      ["kernel-authority", "provider-account", "provider-runtime", "runtime-network", "runtime-state", "ui-client", "worker-kernel"],
    )
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "distributed-runtime").requiredArtifactOwners,
      ["validation-platform"],
    )
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "distributed-runtime").requiredArtifactClassifications,
      ["cloud-validation-suite", "validation-suite"],
    )
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "distributed-runtime").requiredArtifactFailureClassifications,
      ["cloud-runtime", "docker-runtime", "kernel-authority", "projection-staleness", "provider-auth", "provider-error", "relay-runtime", "relay-target-freshness", "remote-extension-sync", "remote-host-capacity", "remote-worker-version", "runtime-projection-health", "slice-auth", "slice-runtime", "ui-client-projection", "worker-execution", "workspace-live-sync-conflict"],
    )
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "distributed-runtime").requiredArtifactExitCriterionStatuses,
      ["satisfied"],
    )
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "remote-home-extension").requiredMatrices,
      ["remote-home-extension-matrix"],
    )
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "native-provider-tui").requiredScenarios,
      ["local-native-tui", "permission-visibility", "remote-native-tui", "slice-native-tui", "transcript-parity"],
    )
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "remote-agent-runtime").requiredMatrices,
      ["remote-agent-runtime-matrix"],
    )
    assert.deepEqual(
      validationSuite.validationPresets.find((preset) => preset.name === "slice-runtime").requiredDeploymentPresets,
      ["hosted-cloud", "local", "self-hosted-relay"],
    )
    assert.deepEqual(artifactIndex.artifacts.map((artifact) => ({
      path: artifact.path,
      schema: artifact.schema,
    })), [
      {
        path: "failure-taxonomy-drill.json",
        schema: "arroba.drill.failure_taxonomy.v1",
      },
      {
        path: "failure-taxonomy-scenario.json",
        schema: "arroba.drill.failure_taxonomy.v1",
      },
      {
        path: "generated-matrix-limitations.json",
        schema: "arroba.drill.generated_matrix_limitations.v1",
      },
      {
        path: "generated-matrix-names.json",
        schema: "arroba.drill.generated_matrix_names.v1",
      },
      {
        path: "index.json",
        schema: "arroba.drill.platform_bundle.v1",
      },
      {
        path: "runtime-signals.json",
        schema: "arroba.drill.runtime_signals.v1",
      },
      {
        path: "validation-suite.json",
        schema: "arroba.drill.validation_suite.v1",
      },
    ])
    assert.equal(validationSuite.schema, "arroba.drill.validation_suite.v1")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects unsafe drill platform bundle artifact paths", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeFile(path.join(rootDir, "index.json"), `${JSON.stringify({
      schema: DRILL_PLATFORM_BUNDLE_SCHEMA,
      outputDir: rootDir,
      artifacts: [{ path: "../outside.json", schema: "arroba.drill.validation_suite.v1" }],
    })}\n`, "utf8")

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /unsafe artifact path/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects incomplete drill platform bundles", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeFile(path.join(rootDir, "index.json"), `${JSON.stringify({
      schema: DRILL_PLATFORM_BUNDLE_SCHEMA,
      outputDir: rootDir,
      artifacts: [{
        path: "validation-suite.json",
        schema: "arroba.drill.validation_suite.v1",
        sha256: "0".repeat(64),
        sizeBytes: 0,
      }],
    })}\n`, "utf8")

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /artifacts do not match required platform contracts/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects validation suite artifact count drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const suitePath = path.join(rootDir, "validation-suite.json")
    const suite = JSON.parse(await readFile(suitePath, "utf8"))
    await replaceBundleArtifact(rootDir, "validation-suite.json", { ...suite, testCount: suite.testCount + 1 })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /testCount does not match testPaths/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects validation suite coverage drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const suitePath = path.join(rootDir, "validation-suite.json")
    const suite = JSON.parse(await readFile(suitePath, "utf8"))
    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      coverage: suite.coverage.map((area) => area.id === "runtime-fixtures"
        ? { ...area, testPaths: area.testPaths.slice(1), testCount: area.testCount - 1 }
        : area),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /missing coverage areas/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects validation suite coverage count drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const suitePath = path.join(rootDir, "validation-suite.json")
    const suite = JSON.parse(await readFile(suitePath, "utf8"))
    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      coverage: suite.coverage.map((area) => area.id === "suite-contract"
        ? { ...area, testCount: area.testCount + 1 }
        : area),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /testCount does not match testPaths/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects validation suite preset contract drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const suitePath = path.join(rootDir, "validation-suite.json")
    const suite = JSON.parse(await readFile(suitePath, "utf8"))
    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      validationPresets: suite.validationPresets.map((preset) => preset.name === "workspace-live-sync"
        ? { ...preset, requiredMatrices: "workspace-live-sync-matrix" }
        : preset),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /requiredMatrices must be an array/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects validation suite preset artifact kind drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const suitePath = path.join(rootDir, "validation-suite.json")
    const suite = JSON.parse(await readFile(suitePath, "utf8"))
    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      validationPresets: suite.validationPresets.map((preset) => preset.name === "distributed-runtime"
        ? { ...preset, requiredArtifactKinds: ["validation-sutie"] }
        : preset),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /requiredArtifactKinds\[0\] has unknown artifact kind "validation-sutie"/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects validation suite preset artifact provenance drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const suitePath = path.join(rootDir, "validation-suite.json")
    const suite = JSON.parse(await readFile(suitePath, "utf8"))
    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      validationPresets: suite.validationPresets.map((preset) => preset.name === "distributed-runtime"
        ? { ...preset, requiredArtifactEvidenceRepos: ["clodu"] }
        : preset),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /requiredArtifactEvidenceRepos\[0\] has unknown artifact evidence repo "clodu"/,
    )

    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      validationPresets: suite.validationPresets.map((preset) => preset.name === "distributed-runtime"
        ? { ...preset, requiredArtifactGeneratedEvidenceRepos: ["clodu"] }
        : preset),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /requiredArtifactGeneratedEvidenceRepos\[0\] has unknown artifact evidence repo "clodu"/,
    )

    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      validationPresets: suite.validationPresets.map((preset) => preset.name === "distributed-runtime"
        ? { ...preset, requiredArtifactGeneratedEvidenceKinds: ["matrix-reprot"] }
        : preset),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /requiredArtifactGeneratedEvidenceKinds\[0\] has unknown generated evidence kind "matrix-reprot"/,
    )

    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      validationPresets: suite.validationPresets.map((preset) => preset.name === "distributed-runtime"
        ? { ...preset, requiredArtifactExitCriterionStatuses: ["satisifed"] }
        : preset),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /requiredArtifactExitCriterionStatuses\[0\] has unknown exit criterion status "satisifed"/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects validation suite preset environment drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const suitePath = path.join(rootDir, "validation-suite.json")
    const suite = JSON.parse(await readFile(suitePath, "utf8"))
    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      validationPresets: suite.validationPresets.map((preset) => preset.name === "distributed-runtime"
        ? { ...preset, requiredFailureClassifications: ["kernel-autohority"] }
        : preset),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /requiredFailureClassifications\[0\] has unknown failure classification "kernel-autohority"/,
    )

    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      validationPresets: suite.validationPresets.map((preset) => preset.name === "distributed-runtime"
        ? { ...preset, requiredMatrixClassifications: ["kernel-autohority"] }
        : preset),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /requiredMatrixClassifications\[0\] has unknown failure classification "kernel-autohority"/,
    )

    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      validationPresets: suite.validationPresets.map((preset) => preset.name === "distributed-runtime"
        ? { ...preset, requiredDeploymentPresets: ["self-hotsed-relay"] }
        : preset),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /requiredDeploymentPresets\[0\] has unknown deployment preset "self-hotsed-relay"/,
    )

    await replaceBundleArtifact(rootDir, "validation-suite.json", {
      ...suite,
      validationPresets: suite.validationPresets.map((preset) => preset.name === "distributed-runtime"
        ? { ...preset, requiredProviders: ["cdoex"] }
        : preset),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /requiredProviders\[0\] has unknown provider "cdoex"/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects failure taxonomy target drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const taxonomyPath = path.join(rootDir, "failure-taxonomy-drill.json")
    const taxonomy = JSON.parse(await readFile(taxonomyPath, "utf8"))
    await replaceBundleArtifact(rootDir, "failure-taxonomy-drill.json", { ...taxonomy, target: "scenario" })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /has invalid target/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects generated matrix limitation taxonomy drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const limitationsPath = path.join(rootDir, "generated-matrix-limitations.json")
    const limitations = JSON.parse(await readFile(limitationsPath, "utf8"))
    await replaceBundleArtifact(rootDir, "generated-matrix-limitations.json", {
      ...limitations,
      limitations: limitations.limitations.map((limitation) => ({
        ...limitation,
        kind: "dry-run-classification-covergae",
      })),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /limitations do not match generated matrix limitation taxonomy/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects failure taxonomy classification drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const taxonomyPath = path.join(rootDir, "failure-taxonomy-scenario.json")
    const taxonomy = JSON.parse(await readFile(taxonomyPath, "utf8"))
    await replaceBundleArtifact(rootDir, "failure-taxonomy-scenario.json", {
      ...taxonomy,
      classifications: taxonomy.classifications.slice(1),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /classifications do not match drill failure taxonomy/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("rejects failure taxonomy owner and next action drift", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-platform-bundle-lib-"))
  try {
    await writeDrillPlatformBundle(rootDir)
    const taxonomyPath = path.join(rootDir, "failure-taxonomy-scenario.json")
    const taxonomy = JSON.parse(await readFile(taxonomyPath, "utf8"))
    await replaceBundleArtifact(rootDir, "failure-taxonomy-scenario.json", {
      ...taxonomy,
      classifications: taxonomy.classifications.map((entry) => entry.kind === "provider-auth"
        ? { ...entry, owner: "runtime-network" }
        : entry),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /invalid owner/,
    )

    await replaceBundleArtifact(rootDir, "failure-taxonomy-scenario.json", {
      ...taxonomy,
      classifications: taxonomy.classifications.map((entry) => entry.kind === "provider-auth"
        ? { ...entry, nextAction: "try something else" }
        : entry),
    })

    await assert.rejects(
      verifyDrillPlatformBundle(rootDir),
      /invalid nextAction/,
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function replaceBundleArtifact(rootDir, artifactPath, contents) {
  const serialized = `${JSON.stringify(contents, null, 2)}\n`
  await writeFile(path.join(rootDir, artifactPath), serialized, "utf8")
  const indexPath = path.join(rootDir, "index.json")
  const index = JSON.parse(await readFile(indexPath, "utf8"))
  index.artifacts = index.artifacts.map((artifact) => artifact.path === artifactPath
    ? {
        ...artifact,
        sha256: createHash("sha256").update(serialized).digest("hex"),
        sizeBytes: Buffer.byteLength(serialized),
      }
    : artifact)
  await writeFile(indexPath, `${JSON.stringify(index, null, 2)}\n`, "utf8")
}
