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

export {
  assert,
  execFile,
  mkdir,
  mkdtemp,
  os,
  path,
  readFile,
  rm,
  scriptPath,
  test,
  verifyDrillArtifactIndex,
  writeDrillArtifactIndex,
  writeFile,
}

export async function writeIndexedReport(rootDir, name, schema) {
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
      runtimeAuthorityInvariants: name === "one"
        ? "home-session-authority"
        : "client-render-request",
      validationPresets: name === "one"
        ? "distributed-runtime"
        : "workspace-live-sync",
      requiredFailureClassifications: name === "one"
        ? "kernel-authority"
        : "workspace-live-sync-conflict",
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
      generatedMatrixNames: name === "one"
        ? ""
        : "workspace-live-sync-matrix",
      generatedMatrixRepos: name === "one"
        ? ""
        : "oss",
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
      requiredGeneratedValidationSuiteFailureRoots: name === "one"
        ? "/tmp/generated-suite/failed-run"
        : "",
      missingGeneratedValidationSuiteFailureRoots: name === "one"
        ? "/tmp/generated-suite/missing-run"
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

export async function rewriteDrillArtifactIndexCreatedAt(indexPath, createdAt) {
  const index = JSON.parse(await readFile(indexPath, "utf8"))
  index.createdAt = createdAt
  await writeFile(indexPath, `${JSON.stringify(index, null, 2)}\n`, "utf8")
}

export async function rewriteDrillMatrixReportCompletedAt(indexPath, completedAt) {
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

export function matrixReportArtifact() {
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
