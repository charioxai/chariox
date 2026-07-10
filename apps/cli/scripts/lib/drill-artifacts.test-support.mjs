import assert from "node:assert/strict"
import { mkdir, mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import {
  DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA,
  DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS,
  DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS,
  DRILL_ARTIFACT_INDEX_SCHEMA,
  diagnosticMetadataForDrillArtifactIndexAggregate,
  findDrillArtifactIndexPaths,
  formatDrillArtifactIndexAggregateSummary,
  finalizeDrillArtifacts,
  prepareDrillArtifacts,
  readDrillArtifactIndex,
  summarizeDrillArtifactIndexes,
  validateDrillArtifactIndexAggregate,
  validateDrillArtifactDiagnosticDimensions,
  validateDrillArtifactIndex,
  verifyDrillArtifactIndex,
  writeDrillArtifactIndex,
  writeDrillJsonArtifactOutput,
} from "./drill-artifacts.mjs"
import { drillFailureTaxonomyManifest } from "./drill-failure-taxonomy.mjs"
import { drillRuntimeSignalsManifest } from "./drill-runtime-signals.mjs"
import { drillRuntimeAuthorityManifest } from "./drill-runtime-authority-invariants.mjs"

export function validationSuiteRunArtifact(overrides = {}) {
  const manifest = overrides.manifest ?? validationSuiteManifestArtifact()
  return {
    schema: "arroba.drill.validation_suite_run.v1",
    status: "passed",
    ok: true,
    startedAt: "2026-01-01T00:00:00.000Z",
    completedAt: "2026-01-01T00:00:01.250Z",
    durationMs: 1250,
    exitCode: 0,
    signal: null,
    error: null,
    command: manifest.command,
    testCount: manifest.testCount,
    testPaths: manifest.testPaths,
    manifest,
    ...overrides,
  }
}

export function validationSuiteManifestArtifact(overrides = {}) {
  return {
    schema: "arroba.drill.validation_suite.v1",
    command: "node --test apps/cli/scripts/lib/drill-artifacts.test.mjs",
    testCount: 1,
    testPaths: ["apps/cli/scripts/lib/drill-artifacts.test.mjs"],
    failureTaxonomyManifest: drillFailureTaxonomyManifest(),
    runtimeSignalsManifest: drillRuntimeSignalsManifest(),
    ...overrides,
  }
}

export function matrixReportArtifact(overrides = {}) {
  const scenarios = overrides.scenarios ?? [{
    id: "local",
    description: "local scenario",
    requires: [],
    exitCriteria: [],
    exitCriteriaEvidence: [],
    runtimeSignals: ["session-authority"],
    status: "passed",
    expectedFailure: false,
    classification: null,
    owner: null,
    nextAction: null,
    durationMs: 1,
    reason: null,
    command: "node",
    args: ["--version"],
    artifactHints: [],
  }]
  return {
    schema: "arroba.drill.matrix.v1",
    matrix: "test-matrix",
    status: "passed",
    dryRun: false,
    startedAt: "2026-01-01T00:00:00.000Z",
    completedAt: "2026-01-01T00:00:01.000Z",
    durationMs: 1000,
    metadata: {},
    scenarios,
    ...overrides,
  }
}

export function focusedRuntimeGateReportArtifact(overrides = {}) {
  const reports = overrides.reports ?? [
    focusedRuntimeGateReportEntry("runtime-authority"),
    focusedRuntimeGateReportEntry("distributed-state-health"),
  ]
  return {
    schema: "arroba.drill.focused_runtime_gate.v1",
    status: reports.some((entry) => entry.report.status === "failed") ? "failed" : "passed",
    presets: ["runtime-authority", "distributed-state-health"],
    reports,
    nextActions: reports.flatMap((entry) =>
      entry.report.nextActions.map((action) => ({ ...action, preset: entry.preset })),
    ),
    ...overrides,
  }
}

export function focusedRuntimeGateReportEntry(preset, overrides = {}) {
  return {
    preset,
    report: validationGateReportArtifact({ preset, ...overrides }),
  }
}

export function validationGateReportArtifact({ preset = "runtime-authority", status = "passed", nextActions = [] } = {}) {
  return {
    schema: "arroba.drill.validation_gate.v1",
    status,
    presets: [preset],
    checks: {
      configuration: { status: "passed" },
      platformBundle: {
        status: "passed",
        dir: "/tmp/platform",
        artifacts: [],
        validationSuite: {
          testCount: 1,
          coverageAreas: [{ id: "matrix-validation", testCount: 1 }],
          validationPresets: [],
        },
      },
      artifacts: {
        status: "skipped",
        roots: [],
        inputs: [],
        indexPaths: [],
      },
      matrices: {
        status: status === "passed" ? "passed" : "failed",
        roots: [],
        inputs: [],
        reportPaths: [],
        requireComplete: false,
        reports: [],
        ...(status === "failed" ? { error: "missing focused runtime evidence" } : {}),
      },
      failures: {
        status: "skipped",
        inputs: [],
        roots: [],
        manifestPaths: [],
        manifests: [],
      },
    },
    nextActions,
  }
}

export function emptyDrillArtifactDiagnosticDimensions(overrides = {}) {
  return {
    ...Object.fromEntries(DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS.map((key) => [key, {}])),
    ...overrides,
  }
}


export {
  assert,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
  os,
  path,
  test,
  DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA,
  DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS,
  DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS,
  DRILL_ARTIFACT_INDEX_SCHEMA,
  diagnosticMetadataForDrillArtifactIndexAggregate,
  findDrillArtifactIndexPaths,
  formatDrillArtifactIndexAggregateSummary,
  finalizeDrillArtifacts,
  prepareDrillArtifacts,
  readDrillArtifactIndex,
  summarizeDrillArtifactIndexes,
  validateDrillArtifactIndexAggregate,
  validateDrillArtifactDiagnosticDimensions,
  validateDrillArtifactIndex,
  verifyDrillArtifactIndex,
  writeDrillArtifactIndex,
  writeDrillJsonArtifactOutput,
  drillFailureTaxonomyManifest,
  drillRuntimeSignalsManifest,
  drillRuntimeAuthorityManifest,
}
