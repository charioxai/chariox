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
import { drillRuntimeSignalOwnersFor } from "./lib/drill-runtime-signals.mjs"

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
    assert.deepEqual(stdoutAggregate.runtimeSignalOwners, {
      "kernel-authority": 2,
      "runtime-state": 1,
    })
    assert.deepEqual(stdoutAggregate.validationPresets, {
      "distributed-runtime": 1,
      "workspace-live-sync": 1,
    })
    assert.deepEqual(stdoutAggregate.artifactKinds, {
      "artifact-index": 1,
      "matrix-report": 1,
      "validation-gate": 1,
    })
    assert.deepEqual(stdoutAggregate.evidenceRepos, {
      cloud: 1,
      oss: 2,
    })
    assert.deepEqual(stdoutAggregate.providerAccountAliases, {
      "codex=work": 1,
      "opencode=zen": 1,
    })
    assert.deepEqual(stdoutAggregate.artifactCoverageInputSources, {
      "artifact metadata inputs": 1,
    })
    assert.deepEqual(stdoutAggregate.exitCriterionStatuses, {
      "dry-run": 1,
    })
    assert.deepEqual(stdoutAggregate.incompleteExitCriterionStatuses, {
      "dry-run": 1,
    })
    assert.deepEqual(stdoutAggregate.plannedOwners, {
      "validation-harness": 1,
    })
    assert.deepEqual(stdoutAggregate.plannedClassifications, {
      "matrix-coverage": 1,
    })
    assert.deepEqual(stdoutAggregate.generatedEvidenceKinds, {
      "matrix-report": 1,
      "validation-suite-run": 1,
    })
    assert.deepEqual(stdoutAggregate.generatedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(stdoutAggregate.generatedMatrixArtifactIndexes, {
      "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.generatedValidationSuiteArtifactIndexes, {
      "/tmp/generated-suite/arroba-drill-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.generatedValidationSuiteFailureRoots, {
      "/tmp/generated-suite/failed-run": 1,
    })
    assert.deepEqual(stdoutAggregate.requiredGeneratedEvidenceKinds, {
      "matrix-report": 2,
      "validation-suite-run": 1,
    })
    assert.deepEqual(stdoutAggregate.missingGeneratedEvidenceKinds, {
      "matrix-report": 1,
    })
    assert.deepEqual(stdoutAggregate.requiredGeneratedMatrixArtifactIndexes, {
      "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.missingGeneratedMatrixArtifactIndexes, {
      "/tmp/generated-matrix/missing-matrix-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.requiredGeneratedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(stdoutAggregate.missingGeneratedMatrixLimitations, {
      "dry-run-classification-coverage": 1,
    })
    assert.deepEqual(stdoutAggregate.requiredGeneratedValidationSuiteArtifactIndexes, {
      "/tmp/generated-suite/arroba-drill-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.missingGeneratedValidationSuiteArtifactIndexes, {
      "/tmp/generated-suite/missing-artifacts.json": 1,
    })
    assert.deepEqual(stdoutAggregate.indexes.map((index) => index.source), [
      firstIndexPath,
      secondIndexPath,
    ])
    assert.equal(artifactIndex.metadata.drill, "artifact-index-summary")
    assert.equal(artifactIndex.metadata.indexes, 2)
    assert.equal(artifactIndex.metadata.runtimeSignals, "lease-health,session-authority,workspace-live-sync-state")
    assert.equal(artifactIndex.metadata.runtimeSignalOwners, "kernel-authority,runtime-state")
    assert.equal(artifactIndex.metadata.validationPresets, "distributed-runtime,workspace-live-sync")
    assert.equal(artifactIndex.metadata.owners, "runtime-network,validation-harness")
    assert.equal(artifactIndex.metadata.classifications, "matrix-coverage,validation-gate")
    assert.equal(artifactIndex.metadata.plannedOwners, "validation-harness")
    assert.equal(artifactIndex.metadata.plannedClassifications, "matrix-coverage")
    assert.equal(artifactIndex.metadata.exitCriterionStatuses, "dry-run")
    assert.equal(artifactIndex.metadata.incompleteExitCriterionStatuses, "dry-run")
    assert.equal(artifactIndex.metadata.artifactKinds, "artifact-index,artifact-index-aggregate,matrix-report,validation-gate")
    assert.equal(artifactIndex.metadata.generatedEvidenceKinds, "matrix-report,validation-suite-run")
    assert.equal(artifactIndex.metadata.generatedMatrixArtifactIndexes, "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json")
    assert.equal(artifactIndex.metadata.generatedMatrixLimitations, "dry-run-classification-coverage")
    assert.equal(artifactIndex.metadata.generatedValidationSuiteArtifactIndexes, "/tmp/generated-suite/arroba-drill-artifacts.json")
    assert.equal(artifactIndex.metadata.generatedValidationSuiteFailureRoots, "/tmp/generated-suite/failed-run")
    assert.equal(artifactIndex.metadata.requiredGeneratedEvidenceKinds, "matrix-report,validation-suite-run")
    assert.equal(artifactIndex.metadata.missingGeneratedEvidenceKinds, "matrix-report")
    assert.equal(artifactIndex.metadata.requiredGeneratedMatrixArtifactIndexes, "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json")
    assert.equal(artifactIndex.metadata.missingGeneratedMatrixArtifactIndexes, "/tmp/generated-matrix/missing-matrix-artifacts.json")
    assert.equal(artifactIndex.metadata.requiredGeneratedMatrixLimitations, "dry-run-classification-coverage")
    assert.equal(artifactIndex.metadata.missingGeneratedMatrixLimitations, "dry-run-classification-coverage")
    assert.equal(artifactIndex.metadata.requiredGeneratedValidationSuiteArtifactIndexes, "/tmp/generated-suite/arroba-drill-artifacts.json")
    assert.equal(artifactIndex.metadata.missingGeneratedValidationSuiteArtifactIndexes, "/tmp/generated-suite/missing-artifacts.json")
    assert.equal(artifactIndex.metadata.evidenceRepos, "cloud,oss")
    assert.equal(artifactIndex.metadata.providerAccountAliases, "codex=work,opencode=zen")
    assert.equal(artifactIndex.metadata.artifactCoverageInputCount, "1")
    assert.equal(artifactIndex.metadata.artifactCoverageInputSources, "artifact metadata inputs")
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

test("drill artifact index summary prints artifact coverage input count", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  try {
    await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")
    await writeIndexedReport(rootDir, "two", "arroba.drill.matrix.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-root",
      rootDir,
    ])

    assert.match(stdout, /artifact_coverage_input_sources: artifact metadata inputs=1/)
    assert.match(stdout, /artifact_coverage_input_count=1/)
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates stale indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")
    await rewriteDrillArtifactIndexCreatedAt(indexPath, new Date(Date.now() - 500).toISOString())

    const fresh = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-artifact-max-age-ms",
      "3600000",
      "--json",
    ])).stdout)
    assert.equal(fresh.requiredArtifactMaxAgeMs, 3_600_000)
    assert.deepEqual(fresh.staleArtifactIndexes, [])

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-artifact-max-age-ms=100",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /artifact_required_max_age_ms=100 stale_indexes=1/)
        assert.match(error.stdout, /next: regenerate stale drill artifact indexes/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates stale matrix reports", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  try {
    const indexPath = await writeIndexedReport(rootDir, "two", "arroba.drill.matrix.v1")
    await rewriteDrillMatrixReportCompletedAt(indexPath, new Date(Date.now() - 500).toISOString())

    const fresh = JSON.parse((await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-matrix-max-age-ms",
      "3600000",
      "--json",
    ])).stdout)
    assert.equal(fresh.requiredMatrixMaxAgeMs, 3_600_000)
    assert.deepEqual(fresh.staleMatrixReports, [])

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-matrix-max-age-ms=100",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /matrix_required_max_age_ms=100 stale_reports=1/)
        assert.match(error.stdout, /next: regenerate stale drill matrix reports/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates generated validation-suite failure roots", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-generated-validation-suite-failure-root",
      "/tmp/generated-suite/failed-run",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredGeneratedValidationSuiteFailureRoots, ["/tmp/generated-suite/failed-run"])
    assert.deepEqual(aggregate.missingGeneratedValidationSuiteFailureRoots, [])
    assert.equal(artifactIndex.metadata.requiredGeneratedValidationSuiteFailureRoots, "/tmp/generated-suite/failed-run")
    assert.equal(artifactIndex.metadata.missingGeneratedValidationSuiteFailureRoots, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-validation-suite-failure-root=/tmp/generated-suite/missing-run",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /generated_validation_suite_failure_roots_required=\/tmp\/generated-suite\/missing-run missing=\/tmp\/generated-suite\/missing-run/)
        assert.match(error.stdout, /next: rerun generated validation suites with --preserve-failure-root .*\/tmp\/generated-suite\/missing-run/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-validation-suite-failure-root",
        "/tmp/generated-suite/Bearer abcdefghijklmnop",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-validation-suite-failure-root includes secret-looking diagnostic text/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates generated validation-suite artifact indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-generated-validation-suite-artifact-index",
      "/tmp/generated-suite/arroba-drill-artifacts.json",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredGeneratedValidationSuiteArtifactIndexPaths, ["/tmp/generated-suite/arroba-drill-artifacts.json"])
    assert.deepEqual(aggregate.missingGeneratedValidationSuiteArtifactIndexPaths, [])
    assert.equal(artifactIndex.metadata.requiredGeneratedValidationSuiteArtifactIndexes, "/tmp/generated-suite/arroba-drill-artifacts.json")
    assert.equal(artifactIndex.metadata.missingGeneratedValidationSuiteArtifactIndexes, "/tmp/generated-suite/missing-artifacts.json")

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-validation-suite-artifact-index=/tmp/generated-suite/missing-artifacts.json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /generated_validation_suite_artifact_indexes_required=\/tmp\/generated-suite\/missing-artifacts\.json missing=\/tmp\/generated-suite\/missing-artifacts\.json/)
        assert.match(error.stdout, /next: rerun generated validation suites with artifact indexes .*\/tmp\/generated-suite\/missing-artifacts\.json/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-validation-suite-artifact-index",
        "/tmp/generated-suite/Bearer abcdefghijklmnop.json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-validation-suite-artifact-index includes secret-looking diagnostic text/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates generated evidence kinds", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-generated-evidence-kind",
      "validation-suite-run",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredGeneratedEvidenceKindRequirements, ["validation-suite-run"])
    assert.deepEqual(aggregate.missingRequiredGeneratedEvidenceKinds, [])
    assert.equal(artifactIndex.metadata.requiredGeneratedEvidenceKindRequirements, "validation-suite-run")
    assert.equal(artifactIndex.metadata.missingRequiredGeneratedEvidenceKinds, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-evidence-kind=matrix-report",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /generated_evidence_kinds_required=matrix-report missing=matrix-report/)
        assert.match(error.stdout, /next: include drill artifact indexes that record generated evidence kinds: matrix-report/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-evidence-kind",
        "matrix-reprot",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-evidence-kind has unknown generated evidence kind: matrix-reprot/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates generated matrix limitations", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const matrixIndexPath = await writeIndexedReport(rootDir, "two", "arroba.drill.matrix.v1")
    const validationIndexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      matrixIndexPath,
      "--require-generated-matrix-limitation",
      "dry-run-classification-coverage",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredGeneratedMatrixLimitationRequirements, ["dry-run-classification-coverage"])
    assert.deepEqual(aggregate.missingRequiredGeneratedMatrixLimitations, [])
    assert.equal(artifactIndex.metadata.requiredGeneratedMatrixLimitationRequirements, "dry-run-classification-coverage")
    assert.equal(artifactIndex.metadata.missingRequiredGeneratedMatrixLimitations, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        validationIndexPath,
        "--require-generated-matrix-limitation=dry-run-classification-coverage",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /generated_matrix_limitations_required=dry-run-classification-coverage missing=dry-run-classification-coverage/)
        assert.match(error.stdout, /next: include drill artifact indexes that record generated matrix limitations: dry-run-classification-coverage/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        matrixIndexPath,
        "--require-generated-matrix-limitation",
        "dry-run-classification-covergae",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-matrix-limitation has unknown generated matrix limitation: dry-run-classification-covergae/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates generated matrix artifact indexes", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "two", "arroba.drill.matrix.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-generated-matrix-artifact-index",
      "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredGeneratedMatrixArtifactIndexPaths, ["/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"])
    assert.deepEqual(aggregate.missingGeneratedMatrixArtifactIndexPaths, [])
    assert.equal(artifactIndex.metadata.requiredGeneratedMatrixArtifactIndexes, "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json")
    assert.equal(artifactIndex.metadata.missingGeneratedMatrixArtifactIndexes, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-matrix-artifact-index=/tmp/generated-matrix/missing-artifacts.json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /generated_matrix_artifact_indexes_required=\/tmp\/generated-matrix\/missing-artifacts\.json missing=\/tmp\/generated-matrix\/missing-artifacts\.json/)
        assert.match(error.stdout, /next: rerun generated matrix drills with artifact indexes .*\/tmp\/generated-matrix\/missing-artifacts\.json/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-generated-matrix-artifact-index=/tmp/generated-matrix/Bearer abcdefghijklmnop.json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-generated-matrix-artifact-index includes secret-looking diagnostic text/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates provider account aliases", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-provider-account-alias",
      "codex=work",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredProviderAccountAliases, ["codex=work"])
    assert.deepEqual(aggregate.missingProviderAccountAliases, [])
    assert.equal(artifactIndex.metadata.requiredProviderAccountAliases, "codex=work")
    assert.equal(artifactIndex.metadata.missingProviderAccountAliases, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-provider-account-alias=opencode=zen",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /provider_account_aliases_required=opencode=zen missing=opencode=zen/)
        assert.match(error.stdout, /next: include drill artifact indexes that record provider account aliases: opencode=zen/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-provider-account-alias",
        "cdoex=work",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-provider-account-alias has invalid value/)
        assert.match(error.stderr, /unknown provider account alias provider: cdoex/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates planned dry-run diagnostics", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "two", "arroba.drill.matrix.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-planned-owner",
      "validation-harness",
      "--require-planned-classification",
      "matrix-coverage",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredPlannedOwners, ["validation-harness"])
    assert.deepEqual(aggregate.missingPlannedOwners, [])
    assert.deepEqual(aggregate.requiredPlannedClassifications, ["matrix-coverage"])
    assert.deepEqual(aggregate.missingPlannedClassifications, [])
    assert.deepEqual(aggregate.nextActions, [])
    assert.equal(artifactIndex.metadata.requiredPlannedOwners, "validation-harness")
    assert.equal(artifactIndex.metadata.requiredPlannedClassifications, "matrix-coverage")
    assert.equal(artifactIndex.metadata.missingPlannedOwners, undefined)
    assert.equal(artifactIndex.metadata.missingPlannedClassifications, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-planned-owner=kernel-authority",
        "--require-planned-classification=workspace-live-sync-conflict",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /planned_owners_required=kernel-authority missing=kernel-authority/)
        assert.match(error.stdout, /planned_classifications_required=workspace-live-sync-conflict missing=workspace-live-sync-conflict/)
        assert.match(error.stdout, /next: include dry-run drill matrix artifact indexes with planned owner coverage: kernel-authority/)
        assert.match(error.stdout, /next: include dry-run drill matrix artifact indexes with planned classification coverage: workspace-live-sync-conflict/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-planned-owner=kernel-authority",
        "--require-planned-classification=workspace-live-sync-conflict",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const missing = JSON.parse(error.stdout)
        assert.deepEqual(missing.nextActions.map(({ owner, classification, nextAction }) => ({ owner, classification, nextAction })), [
          {
            owner: "kernel-authority",
            classification: "artifact-coverage",
            nextAction: "include dry-run drill matrix artifact indexes with planned owner coverage: kernel-authority",
          },
          {
            owner: "runtime-state",
            classification: "workspace-live-sync-conflict",
            nextAction: "include dry-run drill matrix artifact indexes with planned classification coverage: workspace-live-sync-conflict",
          },
        ])
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-planned-owner",
        "Bearer abcdefghijklmnop",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-planned-owner includes secret-looking diagnostic text/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates validation presets", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-validation-preset",
      "distributed-runtime",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredValidationPresets, ["distributed-runtime"])
    assert.deepEqual(aggregate.missingValidationPresets, [])
    assert.equal(artifactIndex.metadata.requiredValidationPresets, "distributed-runtime")
    assert.equal(artifactIndex.metadata.missingValidationPresets, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-validation-preset=workspace-live-sync",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /validation_presets_required=workspace-live-sync missing=workspace-live-sync/)
        assert.match(error.stdout, /next: include drill artifact indexes that record validation presets: workspace-live-sync/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-validation-preset",
        "distributed-runtmie",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-validation-preset has unknown validation preset: distributed-runtmie/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
})

test("drill artifact index summary gates runtime signals with owner-routed next actions", async () => {
  const rootDir = await mkdtemp(path.join(os.tmpdir(), "arroba-artifact-index-summary-"))
  const outputPath = path.join(rootDir, "aggregate.json")
  const artifactIndexPath = path.join(rootDir, "arroba-drill-artifacts.json")
  try {
    const indexPath = await writeIndexedReport(rootDir, "one", "arroba.drill.validation_gate.v1")

    const { stdout } = await execFile(process.execPath, [
      scriptPath,
      "--artifact-index",
      indexPath,
      "--require-runtime-signal",
      "lease-health,session-authority",
      "--require-runtime-signal-owner",
      "kernel-authority",
      "--json",
      "--output",
      outputPath,
      "--output-artifact-index",
      artifactIndexPath,
    ])
    const aggregate = JSON.parse(stdout)
    const artifactIndex = await verifyDrillArtifactIndex(artifactIndexPath)

    assert.deepEqual(aggregate.requiredRuntimeSignalRequirements, ["lease-health", "session-authority"])
    assert.deepEqual(aggregate.missingRuntimeSignalRequirements, [])
    assert.deepEqual(aggregate.requiredRuntimeSignalOwnerRequirements, ["kernel-authority"])
    assert.deepEqual(aggregate.missingRuntimeSignalOwnerRequirements, [])
    assert.deepEqual(aggregate.nextActions, [])
    assert.equal(artifactIndex.metadata.requiredRuntimeSignals, "lease-health,session-authority")
    assert.equal(artifactIndex.metadata.requiredRuntimeSignalOwners, "kernel-authority")
    assert.equal(artifactIndex.metadata.missingRuntimeSignals, undefined)
    assert.equal(artifactIndex.metadata.missingRuntimeSignalOwners, undefined)

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-signal=relay-target-freshness",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /runtime_signals_required=relay-target-freshness missing=relay-target-freshness/)
        assert.match(error.stdout, /next: include drill artifact indexes proving relay-target-freshness owned by runtime-network/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-signal=relay-target-freshness",
        "--json",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        const missing = JSON.parse(error.stdout)
        assert.deepEqual(missing.nextActions.map(({ owner, classification, count }) => ({ owner, classification, count })), [{
          owner: "runtime-network",
          classification: "runtime-signal-coverage",
          count: 1,
        }])
        assert.match(missing.nextActions[0].nextAction, /relay-target-freshness owned by runtime-network/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-signal-owner=runtime-network",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stdout, /runtime_signal_owners_required=runtime-network missing=runtime-network/)
        assert.match(error.stdout, /next: include drill artifact indexes with runtime signal owner coverage: runtime-network/)
        return true
      },
    )

    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-signal",
        "workspace-live-synch-state",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-runtime-signal has unknown runtime signal: workspace-live-synch-state/)
        return true
      },
    )
    await assert.rejects(
      execFile(process.execPath, [
        scriptPath,
        "--artifact-index",
        indexPath,
        "--require-runtime-signal-owner",
        "kernel-authoritiy",
      ]),
      (error) => {
        assert.equal(error.code, 1)
        assert.match(error.stderr, /--require-runtime-signal-owner has unknown runtime signal owner: kernel-authoritiy/)
        return true
      },
    )
  } finally {
    await rm(rootDir, { recursive: true, force: true })
  }
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
  await writeFile(path.join(drillRoot, "reports", "report.json"), `${JSON.stringify(
    schema === "arroba.drill.matrix.v1" ? matrixReportArtifact() : { schema },
  )}\n`, "utf8")
  const runtimeSignals = name === "one"
    ? ["session-authority", "lease-health"]
    : ["session-authority", "workspace-live-sync-state"]
  await writeDrillArtifactIndex({
    rootDir: drillRoot,
    artifacts: ["reports/report.json"],
    metadata: {
      classifications: name === "one"
        ? "validation-gate"
        : "matrix-coverage",
      owners: name === "one"
        ? "validation-harness"
        : "runtime-network",
      plannedClassifications: name === "one"
        ? ""
        : "matrix-coverage",
      plannedOwners: name === "one"
        ? ""
        : "validation-harness",
      exitCriterionStatuses: name === "one"
        ? ""
        : "dry-run",
      incompleteExitCriterionStatuses: name === "one"
        ? ""
        : "dry-run",
      runtimeSignals: runtimeSignals.join(","),
      runtimeSignalOwners: drillRuntimeSignalOwnersFor(runtimeSignals).join(","),
      validationPresets: name === "one"
        ? "distributed-runtime"
        : "workspace-live-sync",
      artifactKinds: name === "one"
        ? "validation-gate,artifact-index"
        : "matrix-report",
      generatedEvidenceKinds: name === "one"
        ? "validation-suite-run"
        : "matrix-report",
      generatedMatrixArtifactIndexes: name === "one"
        ? ""
        : "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json",
      generatedMatrixLimitations: name === "one"
        ? ""
        : "dry-run-classification-coverage",
      generatedValidationSuiteArtifactIndexes: name === "one"
        ? "/tmp/generated-suite/arroba-drill-artifacts.json"
        : "",
      generatedValidationSuiteFailureRoots: name === "one"
        ? "/tmp/generated-suite/failed-run"
        : "",
      requiredGeneratedEvidenceKinds: name === "one"
        ? "validation-suite-run,matrix-report"
        : "matrix-report",
      missingGeneratedEvidenceKinds: name === "one"
        ? "matrix-report"
        : "",
      requiredGeneratedMatrixArtifactIndexes: name === "one"
        ? "/tmp/generated-matrix/workspace-live-sync-matrix-artifacts.json"
        : "",
      missingGeneratedMatrixArtifactIndexes: name === "one"
        ? "/tmp/generated-matrix/missing-matrix-artifacts.json"
        : "",
      requiredGeneratedMatrixLimitations: name === "one"
        ? "dry-run-classification-coverage"
        : "",
      missingGeneratedMatrixLimitations: name === "one"
        ? "dry-run-classification-coverage"
        : "",
      requiredGeneratedValidationSuiteArtifactIndexes: name === "one"
        ? "/tmp/generated-suite/arroba-drill-artifacts.json"
        : "",
      missingGeneratedValidationSuiteArtifactIndexes: name === "one"
        ? "/tmp/generated-suite/missing-artifacts.json"
        : "",
      evidenceRepos: name === "one"
        ? "oss"
        : "oss,cloud",
      providerAccountAliases: name === "one"
        ? "codex=work"
        : "opencode=zen",
      artifactCoverageInputSources: name === "one"
        ? ""
        : "artifact metadata inputs",
    },
  })
  return path.join(drillRoot, "arroba-drill-artifacts.json")
}

async function rewriteDrillArtifactIndexCreatedAt(indexPath, createdAt) {
  const index = JSON.parse(await readFile(indexPath, "utf8"))
  index.createdAt = createdAt
  await writeFile(indexPath, `${JSON.stringify(index, null, 2)}\n`, "utf8")
}

async function rewriteDrillMatrixReportCompletedAt(indexPath, completedAt) {
  const index = JSON.parse(await readFile(indexPath, "utf8"))
  const artifact = index.artifacts.find((entry) => entry.schema === "arroba.drill.matrix.v1")
  const artifactPath = path.join(index.rootDir, artifact.path)
  const report = JSON.parse(await readFile(artifactPath, "utf8"))
  report.completedAt = completedAt
  report.startedAt = new Date(Date.parse(completedAt) - 1000).toISOString()
  report.durationMs = 1000
  await writeFile(artifactPath, `${JSON.stringify(report, null, 2)}\n`, "utf8")
  await writeDrillArtifactIndex({
    rootDir: index.rootDir,
    artifacts: index.artifacts.map((entry) => entry.path),
    indexPath,
    metadata: index.metadata,
  })
}

function matrixReportArtifact() {
  return {
    schema: "arroba.drill.matrix.v1",
    matrix: "artifact-index-summary-matrix",
    status: "dry-run",
    dryRun: true,
    startedAt: "2026-01-01T00:00:00.000Z",
    completedAt: "2026-01-01T00:00:01.000Z",
    durationMs: 1000,
    metadata: {},
    scenarios: [{
      id: "summary",
      description: "summary scenario",
      requires: [],
      exitCriteria: ["summary aggregate records incomplete exit criterion status"],
      exitCriteriaEvidence: [{
        id: "summary:exit-01",
        criterion: "summary aggregate records incomplete exit criterion status",
        status: "dry-run",
        reason: "scenario command was selected but not executed",
      }],
      runtimeSignals: ["session-authority"],
      status: "dry-run",
      expectedFailure: false,
      classification: null,
      owner: null,
      plannedClassification: "matrix-coverage",
      plannedOwner: "validation-harness",
      plannedNextAction: "run the missing deployment preset scenario, then rerun the matrix",
      nextAction: null,
      durationMs: 0,
      reason: null,
      command: "node",
      args: ["--version"],
      artifactHints: [],
    }],
  }
}
