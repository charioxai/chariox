import assert from "node:assert/strict"
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  describeDrillValidationGatePresets,
  drillValidationGateExitCode,
  findDrillValidationGateAggregatePaths,
  findDrillValidationGateReportPaths,
  formatDrillValidationGateAggregateSummary,
  formatDrillValidationGateSummary,
  readDrillValidationGateAggregate,
  readDrillValidationGateReport,
  runDrillValidationGate,
  summarizeDrillValidationGateReports,
  validateDrillValidationGateAggregate,
  validateDrillValidationGateReport,
} from "./drill-validation-gate.mjs"
import { writeDrillArtifactIndex } from "./drill-artifacts.mjs"
import { writeDrillPlatformBundle } from "./drill-platform-bundle.mjs"
import { drillValidationSuiteManifest } from "./drill-validation-suite.mjs"
import {
  distributedStateHealthPartialMatrixReport,
  runtimeAuthorityMatrixReportFixtures,
} from "./focused-runtime-fixtures.mjs"
import {
  workspaceLiveSyncRequiredScenarioDescriptors,
  workspaceLiveSyncRequiredScenarioIds,
} from "./workspace-live-sync-fixtures.mjs"

export {
  assert,
  describeDrillValidationGatePresets,
  distributedStateHealthPartialMatrixReport,
  drillValidationGateExitCode,
  findDrillValidationGateAggregatePaths,
  findDrillValidationGateReportPaths,
  formatDrillValidationGateAggregateSummary,
  formatDrillValidationGateSummary,
  mkdir,
  mkdtemp,
  os,
  path,
  readDrillValidationGateAggregate,
  readDrillValidationGateReport,
  readFile,
  rm,
  runDrillValidationGate,
  runtimeAuthorityMatrixReportFixtures,
  summarizeDrillValidationGateReports,
  test,
  validateDrillValidationGateAggregate,
  validateDrillValidationGateReport,
  writeDrillArtifactIndex,
  writeDrillPlatformBundle,
  writeFile,
  workspaceLiveSyncRequiredScenarioIds,
}

export async function writeMatrixReport(file, report) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify(report, null, 2)}\n`, "utf8")
}

export async function writeFailureManifest(file, {
  drill = "failed-drill",
  failedAt = "2026-06-13T00:00:00.000Z",
} = {}) {
  await mkdir(path.dirname(file), { recursive: true })
  await writeFile(file, `${JSON.stringify({
    schema: "arroba.drill.failure.v1",
    rootDir: path.dirname(file),
    failedAt,
    metadata: { drill },
    error: { name: "Error", message: "Token refresh failed: 401", stack: null },
  }, null, 2)}\n`, "utf8")
}

export async function rewriteArtifactIndexCreatedAt(indexPath, createdAt) {
  const index = JSON.parse(await readFile(indexPath, "utf8"))
  index.createdAt = createdAt
  await writeFile(indexPath, `${JSON.stringify(index, null, 2)}\n`, "utf8")
}

export function matrixReport(overrides = {}) {
  const scenarios = overrides.scenarios ?? [scenario("local", "passed")]
  const status = overrides.status ?? (scenarios.some((entry) => entry.status === "failed") ? "failed" : "passed")
  const dryRun = overrides.dryRun ?? false
  return {
    schema: "arroba.drill.matrix.v1",
    matrix: "test-matrix",
    status,
    dryRun,
    startedAt: "2026-06-13T00:00:00.000Z",
    completedAt: "2026-06-13T00:00:01.000Z",
    durationMs: 1000,
    metadata: {},
    scenarios,
    ...overrides,
  }
}

export function workspaceLiveSyncRequiredScenarios() {
  return workspaceLiveSyncRequiredScenarioDescriptors().map(({ id, classification, runtimeSignals }) => {
    return scenario(id, "passed", { classification, runtimeSignals })
  })
}

export function platformValidationPresetSummaries() {
  return drillValidationSuiteManifest().validationPresets.map((preset) => ({
    name: preset.name,
    requiredArtifactCoverageAreas: preset.requiredArtifactCoverageAreas,
    requiredArtifactSchemas: preset.requiredArtifactSchemas,
    requiredArtifactKinds: preset.requiredArtifactKinds,
    requiredArtifactGeneratedEvidenceKinds: preset.requiredArtifactGeneratedEvidenceKinds,
    requiredArtifactGeneratedEvidenceRepos: preset.requiredArtifactGeneratedEvidenceRepos,
    requiredArtifactGeneratedMatrixArtifactIndexes: preset.requiredArtifactGeneratedMatrixArtifactIndexes,
    requiredArtifactGeneratedMatrixLimitations: preset.requiredArtifactGeneratedMatrixLimitations,
    requiredArtifactGeneratedMatrixNames: preset.requiredArtifactGeneratedMatrixNames,
    requiredArtifactGeneratedMatrixRepos: preset.requiredArtifactGeneratedMatrixRepos,
    requiredArtifactGeneratedValidationSuiteArtifactIndexes: preset.requiredArtifactGeneratedValidationSuiteArtifactIndexes,
    requiredArtifactEvidenceRepos: preset.requiredArtifactEvidenceRepos,
    requiredArtifactProviderAccountAliases: preset.requiredArtifactProviderAccountAliases,
    requiredArtifactExitCriterionStatuses: preset.requiredArtifactExitCriterionStatuses,
    requiredArtifactIncompleteExitCriterionStatuses: preset.requiredArtifactIncompleteExitCriterionStatuses,
    requiredMatrices: preset.requiredMatrices,
    requiredRuntimeSignals: preset.requiredRuntimeSignals,
    requiredFailureClassifications: preset.requiredFailureClassifications,
    requiredMatrixRuntimeSignals: preset.requiredMatrixRuntimeSignals,
    requiredDeploymentPresets: preset.requiredDeploymentPresets,
    requiredProviders: preset.requiredProviders,
    requiredScenarios: preset.requiredScenarios,
    requiredGeneratedEvidenceKinds: preset.requiredGeneratedEvidenceKinds,
    requiredGeneratedMatrixArtifactIndexes: preset.requiredGeneratedMatrixArtifactIndexes,
    requiredGeneratedMatrixLimitations: preset.requiredGeneratedMatrixLimitations,
    requiredGeneratedValidationSuiteArtifactIndexes: preset.requiredGeneratedValidationSuiteArtifactIndexes,
    requiredGeneratedValidationSuiteFailureRoots: preset.requiredGeneratedValidationSuiteFailureRoots,
  }))
}

export function emptyArtifactCoverageSummary() {
  return {
    requiredArtifactCoverageAreas: [],
    missingArtifactCoverageAreas: [],
    requiredArtifactSchemas: [],
    missingArtifactSchemas: [],
    requiredArtifactKinds: [],
    missingArtifactKinds: [],
    requiredArtifactGeneratedEvidenceKinds: [],
    requiredArtifactGeneratedEvidenceRepos: [],
    missingArtifactGeneratedEvidenceKinds: [],
    missingArtifactGeneratedEvidenceRepos: [],
    requiredArtifactGeneratedMatrixArtifactIndexes: [],
    missingArtifactGeneratedMatrixArtifactIndexes: [],
    requiredArtifactGeneratedMatrixLimitations: [],
    missingArtifactGeneratedMatrixLimitations: [],
    requiredArtifactGeneratedMatrixNames: [],
    missingArtifactGeneratedMatrixNames: [],
    requiredArtifactGeneratedMatrixRepos: [],
    missingArtifactGeneratedMatrixRepos: [],
    requiredArtifactGeneratedValidationSuiteArtifactIndexes: [],
    missingArtifactGeneratedValidationSuiteArtifactIndexes: [],
    requiredArtifactEvidenceRepos: [],
    missingArtifactEvidenceRepos: [],
    requiredArtifactProviderAccountAliases: [],
    missingArtifactProviderAccountAliases: [],
    requiredArtifactValidationPresets: [],
    missingArtifactValidationPresets: [],
    requiredArtifactRuntimeAuthorityInvariants: [],
    missingArtifactRuntimeAuthorityInvariants: [],
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
    schemas: {},
    coverageAreas: {},
    runtimeAuthorityInvariants: {},
    runtimeSignals: {},
    runtimeSignalOwners: {},
    owners: {},
    classifications: {},
    failureClassifications: {},
    plannedOwners: {},
    plannedClassifications: {},
    exitCriterionStatuses: {},
    incompleteExitCriterionStatuses: {},
    artifactKinds: {},
    generatedEvidenceKinds: {},
    generatedEvidenceRepos: {},
    generatedMatrixArtifactIndexes: {},
    generatedMatrixLimitations: {},
    generatedMatrixNames: {},
    generatedMatrixRepos: {},
    generatedValidationSuiteArtifactIndexes: {},
    generatedValidationSuiteFailureRoots: {},
    evidenceRepos: {},
    providerAccountAliases: {},
    validationPresets: {},
    artifactCoverageInputSources: {},
  }
}

export function scenario(id, status, overrides = {}) {
  return {
    id,
    description: `${id} scenario`,
    requires: [],
    exitCriteria: [],
    status,
    expectedFailure: false,
    classification: status === "failed" ? "child-process" : null,
    durationMs: status === "skipped" || status === "dry-run" ? 0 : 10,
    reason: status === "failed" ? "code=1" : status === "skipped" ? "not run" : null,
    command: "node",
    args: [`${id}.mjs`],
    artifactHints: [],
    ...overrides,
  }
}
