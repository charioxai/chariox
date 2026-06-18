import assert from "node:assert/strict"
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { writeDrillArtifactIndex } from "./drill-artifacts.mjs"
import { artifactValidationGateCheck } from "./drill-validation-gate-artifact-check.mjs"

test("skips artifact validation when no roots or indexes are configured", async () => {
  const check = await artifactValidationGateCheck({
    artifactIndexes: [],
    artifactRoots: [],
  }, { maxDepth: 8 })

  assert.deepEqual(check, {
    status: "skipped",
    roots: [],
    inputs: [],
    indexPaths: [],
    requiredArtifactMaxAgeMs: null,
    staleArtifactIndexes: [],
    requiredArtifactCoverageAreas: [],
    missingArtifactCoverageAreas: [],
    requiredArtifactSchemas: [],
    missingArtifactSchemas: [],
    requiredArtifactKinds: [],
    missingArtifactKinds: [],
    requiredArtifactGeneratedEvidenceKinds: [],
    missingArtifactGeneratedEvidenceKinds: [],
    requiredArtifactGeneratedMatrixArtifactIndexes: [],
    missingArtifactGeneratedMatrixArtifactIndexes: [],
    requiredArtifactGeneratedMatrixLimitations: [],
    missingArtifactGeneratedMatrixLimitations: [],
    requiredArtifactGeneratedMatrixNames: [],
    missingArtifactGeneratedMatrixNames: [],
    requiredArtifactGeneratedMatrixRepos: [],
    missingArtifactGeneratedMatrixRepos: [],
    requiredArtifactGeneratedValidationSuiteFailureRoots: [],
    missingArtifactGeneratedValidationSuiteFailureRoots: [],
    requiredArtifactEvidenceRepos: [],
    missingArtifactEvidenceRepos: [],
    requiredArtifactProviderAccountAliases: [],
    missingArtifactProviderAccountAliases: [],
    requiredArtifactValidationPresets: [],
    missingArtifactValidationPresets: [],
    requiredArtifactRuntimeSignals: [],
    missingArtifactRuntimeSignals: [],
    requiredArtifactRuntimeSignalOwners: [],
    missingArtifactRuntimeSignalOwners: [],
    requiredArtifactOwners: [],
    missingArtifactOwners: [],
    requiredArtifactClassifications: [],
    missingArtifactClassifications: [],
    requiredArtifactFailureClassifications: [],
    missingArtifactFailureClassifications: [],
    requiredArtifactPlannedOwners: [],
    missingArtifactPlannedOwners: [],
    requiredArtifactPlannedClassifications: [],
    missingArtifactPlannedClassifications: [],
    requiredArtifactExitCriterionStatuses: [],
    missingArtifactExitCriterionStatuses: [],
    requiredArtifactIncompleteExitCriterionStatuses: [],
    missingArtifactIncompleteExitCriterionStatuses: [],
  })
})

test("fails when required artifact schemas have no evidence", async () => {
  const check = await artifactValidationGateCheck({
    artifactIndexes: [],
    artifactRoots: [],
  }, {
    maxDepth: 8,
    requiredArtifactCoverageAreas: [],
    requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
  })

  assert.equal(check.status, "failed")
  assert.deepEqual(check.requiredArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
  assert.deepEqual(check.missingArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
  assert.match(check.error, /missing required artifact schemas/)
})

test("gates required artifact coverage areas from artifact index metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    await writeReportArtifact(rootDir, "reports/gate.json")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
      metadata: { coverageAreas: "distributed-observability,matrix-validation" },
    })

    const pass = await artifactValidationGateCheck({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactCoverageAreas: ["distributed-observability"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.requiredArtifactCoverageAreas, ["distributed-observability"])
    assert.deepEqual(pass.missingArtifactCoverageAreas, [])
    assert.deepEqual(pass.aggregate.coverageAreas, {
      "distributed-observability": 1,
      "matrix-validation": 1,
    })

    const fail = await artifactValidationGateCheck({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactCoverageAreas: ["runtime-fixtures"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.missingArtifactCoverageAreas, ["runtime-fixtures"])
    assert.match(fail.error, /missing required artifact coverage areas: runtime-fixtures/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates required artifact kinds and evidence repos from artifact index metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    await writeReportArtifact(rootDir, "reports/gate.json")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
      metadata: {
        artifactKinds: "validation-gate,artifact-index",
        generatedEvidenceKinds: "validation-suite-run",
        generatedMatrixArtifactIndexes: "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
        generatedMatrixLimitations: "dry-run-classification-coverage",
        generatedMatrixNames: "workspace-live-sync-matrix",
        generatedMatrixRepos: "oss",
        generatedValidationSuiteFailureRoots: "/tmp/generated-suite/failed-run",
        evidenceRepos: "oss",
      },
    })

    const pass = await artifactValidationGateCheck({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactKinds: ["validation-gate"],
      requiredArtifactGeneratedEvidenceKinds: ["validation-suite-run"],
      requiredArtifactGeneratedMatrixArtifactIndexes: ["/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"],
      requiredArtifactGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
      requiredArtifactGeneratedMatrixNames: ["workspace-live-sync-matrix"],
      requiredArtifactGeneratedMatrixRepos: ["oss"],
      requiredArtifactGeneratedValidationSuiteFailureRoots: ["/tmp/generated-suite/failed-run"],
      requiredArtifactEvidenceRepos: ["oss"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.requiredArtifactKinds, ["validation-gate"])
    assert.deepEqual(pass.missingArtifactKinds, [])
    assert.deepEqual(pass.requiredArtifactGeneratedEvidenceKinds, ["validation-suite-run"])
    assert.deepEqual(pass.missingArtifactGeneratedEvidenceKinds, [])
    assert.deepEqual(pass.requiredArtifactGeneratedMatrixArtifactIndexes, ["/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"])
    assert.deepEqual(pass.missingArtifactGeneratedMatrixArtifactIndexes, [])
    assert.deepEqual(pass.requiredArtifactGeneratedMatrixLimitations, ["dry-run-classification-coverage"])
    assert.deepEqual(pass.missingArtifactGeneratedMatrixLimitations, [])
    assert.deepEqual(pass.requiredArtifactGeneratedMatrixNames, ["workspace-live-sync-matrix"])
    assert.deepEqual(pass.missingArtifactGeneratedMatrixNames, [])
    assert.deepEqual(pass.requiredArtifactGeneratedMatrixRepos, ["oss"])
    assert.deepEqual(pass.missingArtifactGeneratedMatrixRepos, [])
    assert.deepEqual(pass.requiredArtifactGeneratedValidationSuiteFailureRoots, ["/tmp/generated-suite/failed-run"])
    assert.deepEqual(pass.missingArtifactGeneratedValidationSuiteFailureRoots, [])
    assert.deepEqual(pass.requiredArtifactEvidenceRepos, ["oss"])
    assert.deepEqual(pass.missingArtifactEvidenceRepos, [])
    assert.deepEqual(pass.aggregate.artifactKinds, {
      "artifact-index": 1,
      "validation-gate": 1,
    })
    assert.deepEqual(pass.aggregate.evidenceRepos, { oss: 1 })

    const fail = await artifactValidationGateCheck({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactKinds: ["validation-suite-run"],
      requiredArtifactGeneratedEvidenceKinds: ["matrix-report"],
      requiredArtifactGeneratedMatrixArtifactIndexes: ["/tmp/generated-matrix/missing-artifacts.json"],
      requiredArtifactGeneratedMatrixLimitations: ["dry-run-classification-coverage"],
      requiredArtifactGeneratedMatrixNames: ["missing-matrix"],
      requiredArtifactGeneratedMatrixRepos: ["cloud"],
      requiredArtifactGeneratedValidationSuiteFailureRoots: ["/tmp/generated-suite/missing-run"],
      requiredArtifactEvidenceRepos: ["cloud"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.missingArtifactKinds, ["validation-suite-run"])
    assert.deepEqual(fail.missingArtifactGeneratedEvidenceKinds, ["matrix-report"])
    assert.deepEqual(fail.missingArtifactGeneratedMatrixArtifactIndexes, ["/tmp/generated-matrix/missing-artifacts.json"])
    assert.deepEqual(fail.missingArtifactGeneratedMatrixLimitations, [])
    assert.deepEqual(fail.missingArtifactGeneratedMatrixNames, ["missing-matrix"])
    assert.deepEqual(fail.missingArtifactGeneratedMatrixRepos, ["cloud"])
    assert.deepEqual(fail.missingArtifactGeneratedValidationSuiteFailureRoots, ["/tmp/generated-suite/missing-run"])
    assert.deepEqual(fail.missingArtifactEvidenceRepos, ["cloud"])
    assert.match(fail.error, /missing required artifact kinds: validation-suite-run/)
    assert.match(fail.error, /missing required artifact generated evidence kinds: matrix-report/)
    assert.match(fail.error, /missing required artifact generated matrix artifact indexes: \/tmp\/generated-matrix\/missing-artifacts\.json/)
    assert.match(fail.error, /missing required artifact generated matrix names: missing-matrix/)
    assert.match(fail.error, /missing required artifact generated matrix repos: cloud/)
    assert.match(fail.error, /missing required artifact generated validation-suite failure roots: \/tmp\/generated-suite\/missing-run/)
    assert.match(fail.error, /missing required artifact evidence repos: cloud/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates required artifact diagnostic dimensions from artifact index metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    await writeReportArtifact(rootDir, "reports/gate.json")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
      metadata: {
        runtimeSignals: "session-authority,workspace-live-sync-state",
        runtimeSignalOwners: "kernel-authority,runtime-state",
        validationPresets: "distributed-runtime",
        owners: "validation-platform",
        classifications: "validation-gate",
        requiredFailureClassifications: "kernel-authority,workspace-live-sync-conflict",
        plannedOwners: "validation-harness",
        plannedClassifications: "matrix-coverage",
      },
    })
    const indexPath = path.join(rootDir, "arroba-drill-artifacts.json")

    const pass = await artifactValidationGateCheck({
      artifactIndexes: [indexPath],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactRuntimeSignals: ["workspace-live-sync-state"],
      requiredArtifactRuntimeSignalOwners: ["kernel-authority", "runtime-state"],
      requiredArtifactValidationPresets: ["distributed-runtime"],
      requiredArtifactOwners: ["validation-platform"],
      requiredArtifactClassifications: ["validation-gate"],
      requiredArtifactFailureClassifications: ["workspace-live-sync-conflict"],
      requiredArtifactPlannedOwners: ["validation-harness"],
      requiredArtifactPlannedClassifications: ["matrix-coverage"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.missingArtifactRuntimeSignals, [])
    assert.deepEqual(pass.missingArtifactRuntimeSignalOwners, [])
    assert.deepEqual(pass.missingArtifactValidationPresets, [])
    assert.deepEqual(pass.missingArtifactOwners, [])
    assert.deepEqual(pass.missingArtifactClassifications, [])
    assert.deepEqual(pass.missingArtifactFailureClassifications, [])
    assert.deepEqual(pass.missingArtifactPlannedOwners, [])
    assert.deepEqual(pass.missingArtifactPlannedClassifications, [])

    const fail = await artifactValidationGateCheck({
      artifactIndexes: [indexPath],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactRuntimeSignals: ["lease-health"],
      requiredArtifactRuntimeSignalOwners: ["worker-kernel"],
      requiredArtifactValidationPresets: ["workspace-live-sync"],
      requiredArtifactOwners: ["runtime-network"],
      requiredArtifactClassifications: ["relay-runtime"],
      requiredArtifactFailureClassifications: ["remote-extension-sync"],
      requiredArtifactPlannedOwners: ["kernel-authority"],
      requiredArtifactPlannedClassifications: ["workspace-live-sync-conflict"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.missingArtifactRuntimeSignals, ["lease-health"])
    assert.deepEqual(fail.missingArtifactRuntimeSignalOwners, ["worker-kernel"])
    assert.deepEqual(fail.missingArtifactValidationPresets, ["workspace-live-sync"])
    assert.deepEqual(fail.missingArtifactOwners, ["runtime-network"])
    assert.deepEqual(fail.missingArtifactClassifications, ["relay-runtime"])
    assert.deepEqual(fail.missingArtifactFailureClassifications, ["remote-extension-sync"])
    assert.deepEqual(fail.missingArtifactPlannedOwners, ["kernel-authority"])
    assert.deepEqual(fail.missingArtifactPlannedClassifications, ["workspace-live-sync-conflict"])
    assert.match(fail.error, /missing required artifact runtime signals: lease-health/)
    assert.match(fail.error, /missing required artifact runtime signal owners: worker-kernel/)
    assert.match(fail.error, /missing required artifact validation presets: workspace-live-sync/)
    assert.match(fail.error, /missing required artifact owners: runtime-network/)
    assert.match(fail.error, /missing required artifact classifications: relay-runtime/)
    assert.match(fail.error, /missing required artifact failure classifications: remote-extension-sync/)
    assert.match(fail.error, /missing required artifact planned owners: kernel-authority/)
    assert.match(fail.error, /missing required artifact planned classifications: workspace-live-sync-conflict/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates required artifact exit criterion statuses from artifact index metadata", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    await writeReportArtifact(rootDir, "reports/gate.json")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
      metadata: {
        exitCriterionStatuses: "satisfied,failed",
        incompleteExitCriterionStatuses: "dry-run,skipped",
      },
    })
    const indexPath = path.join(rootDir, "arroba-drill-artifacts.json")

    const pass = await artifactValidationGateCheck({
      artifactIndexes: [indexPath],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactExitCriterionStatuses: ["satisfied"],
      requiredArtifactIncompleteExitCriterionStatuses: ["dry-run"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.missingArtifactExitCriterionStatuses, [])
    assert.deepEqual(pass.missingArtifactIncompleteExitCriterionStatuses, [])

    const fail = await artifactValidationGateCheck({
      artifactIndexes: [indexPath],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactExitCriterionStatuses: ["skipped"],
      requiredArtifactIncompleteExitCriterionStatuses: ["satisfied"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.missingArtifactExitCriterionStatuses, ["skipped"])
    assert.deepEqual(fail.missingArtifactIncompleteExitCriterionStatuses, ["satisfied"])
    assert.match(fail.error, /missing required artifact exit criterion statuses: skipped/)
    assert.match(fail.error, /missing required artifact incomplete exit criterion statuses: satisfied/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when artifact roots contain no indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    const check = await artifactValidationGateCheck({
      artifactIndexes: [],
      artifactRoots: [rootDir],
    }, { maxDepth: 8 })

    assert.equal(check.status, "failed")
    assert.deepEqual(check.roots, [rootDir])
    assert.deepEqual(check.indexPaths, [])
    assert.equal(check.error, "no artifact indexes found")
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("passes with explicit and discovered artifact indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    const explicitRoot = path.join(rootDir, "explicit")
    const discoveredRoot = path.join(rootDir, "discovered")
    const explicitIndex = path.join(explicitRoot, "custom-index.json")
    const discoveredIndex = path.join(discoveredRoot, "arroba-drill-artifacts.json")
    await writeReportArtifact(explicitRoot, "reports/gate.json")
    await writeReportArtifact(discoveredRoot, "reports/matrix.json")
    await writeDrillArtifactIndex({
      rootDir: explicitRoot,
      artifacts: ["reports/gate.json"],
      indexPath: explicitIndex,
      metadata: { drill: "explicit" },
    })
    await writeDrillArtifactIndex({
      rootDir: discoveredRoot,
      artifacts: ["reports/matrix.json"],
      metadata: { drill: "discovered" },
    })

    const check = await artifactValidationGateCheck({
      artifactIndexes: [explicitIndex],
      artifactRoots: [rootDir],
    }, { maxDepth: 8 })

    assert.equal(check.status, "passed")
    assert.deepEqual(check.inputs, [explicitIndex])
    assert.deepEqual(check.indexPaths, [explicitIndex, discoveredIndex].sort())
    assert.equal(check.aggregate.totals.indexes, 2)
    assert.equal(check.aggregate.totals.artifacts, 2)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("gates required artifact schemas", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    await writeReportArtifact(rootDir, "reports/gate.json")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
      metadata: { drill: "schema-gate" },
    })

    const pass = await artifactValidationGateCheck({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactSchemas: ["arroba.drill.validation_gate.v1"],
    })
    assert.equal(pass.status, "passed")
    assert.deepEqual(pass.requiredArtifactSchemas, ["arroba.drill.validation_gate.v1"])
    assert.deepEqual(pass.missingArtifactSchemas, [])

    const fail = await artifactValidationGateCheck({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
      artifactRoots: [],
    }, {
      maxDepth: 8,
      requiredArtifactSchemas: ["arroba.drill.validation_suite_run.v1"],
    })
    assert.equal(fail.status, "failed")
    assert.deepEqual(fail.missingArtifactSchemas, ["arroba.drill.validation_suite_run.v1"])
    assert.match(fail.error, /missing required artifact schemas/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("fails when an indexed artifact is tampered", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-validation-gate-artifacts-"))
  try {
    const artifactPath = path.join(rootDir, "reports", "gate.json")
    await writeReportArtifact(rootDir, "reports/gate.json")
    await writeDrillArtifactIndex({
      rootDir,
      artifacts: ["reports/gate.json"],
    })
    await writeFile(artifactPath, "{\"schema\":\"tampered\"}\n", "utf8")

    const check = await artifactValidationGateCheck({
      artifactIndexes: [path.join(rootDir, "arroba-drill-artifacts.json")],
      artifactRoots: [],
    }, { maxDepth: 8 })

    assert.equal(check.status, "failed")
    assert.match(check.error, /drill artifact reports\/gate\.json sha256 mismatch/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

async function writeReportArtifact(rootDir, relativePath) {
  const file = path.join(rootDir, relativePath)
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.validation_gate.v1",
    status: "passed",
  })}\n`, "utf8")
}
