import { validateDrillFailureTaxonomyManifest } from "./drill-failure-taxonomy.mjs"
import { validateDrillMatrixReport } from "./drill-matrix-report.mjs"
import { validateDrillFocusedRuntimeGateReport } from "./drill-focused-runtime-gate-report.mjs"
import { parseDrillIsoTimestamp } from "./drill-time.mjs"
import { validateDrillValidationResultStatus } from "./drill-validation-statuses.mjs"
import {
  metadataHasAnyList,
  metadataListFromMetadata,
} from "./drill-artifact-metadata-validation.mjs"
import {
  validateDrillRuntimeSignalsManifest,
} from "./drill-runtime-signals.mjs"
import { validateDrillRuntimeAuthorityManifest } from "./drill-runtime-authority-invariants.mjs"

export function validateKnownArtifactContents(contents, artifactPath, metadata = {}) {
  let parsed
  try {
    parsed = JSON.parse(contents.toString("utf8"))
  } catch {
    return
  }
  const requiresRuntimeSignalManifest = metadataHasAnyList(metadata, [
    "runtimeSignals",
    "requiredRuntimeSignals",
    "missingRuntimeSignals",
  ])
  const requiresFailureTaxonomyManifest = requiresRuntimeSignalManifest
    || metadataHasAnyList(metadata, [
      "classifications",
      "requiredFailureClassifications",
      "missingFailureClassifications",
    ])
  const requiresRuntimeAuthorityManifest = metadataHasAnyList(metadata, [
    "runtimeAuthorityInvariants",
    "requiredRuntimeAuthorityInvariants",
    "missingRuntimeAuthorityInvariants",
  ])
  if (parsed?.schema === "arroba.drill.validation_suite.v1") {
    validateValidationSuiteManifestArtifact(parsed, artifactPath)
    validateValidationSuiteArtifactMetadata({
      artifactPath,
      expectedKind: "validation-suite",
      metadata,
      testCount: parsed.testCount,
    })
    if (requiresRuntimeSignalManifest && parsed.runtimeSignalsManifest === undefined) {
      throw new Error(`drill artifact ${artifactPath} is missing runtimeSignalsManifest`)
    }
    if (requiresFailureTaxonomyManifest && parsed.failureTaxonomyManifest === undefined) {
      throw new Error(`drill artifact ${artifactPath} is missing failureTaxonomyManifest`)
    }
    if (requiresRuntimeAuthorityManifest && parsed.runtimeAuthorityManifest === undefined) {
      throw new Error(`drill artifact ${artifactPath} is missing runtimeAuthorityManifest`)
    }
    if (parsed.failureTaxonomyManifest !== undefined) {
      validateDrillFailureTaxonomyManifest(parsed.failureTaxonomyManifest, `${artifactPath}.failureTaxonomyManifest`)
    }
    if (parsed.runtimeSignalsManifest !== undefined) {
      validateDrillRuntimeSignalsManifest(parsed.runtimeSignalsManifest, `${artifactPath}.runtimeSignalsManifest`)
    }
    if (parsed.runtimeAuthorityManifest !== undefined) {
      validateDrillRuntimeAuthorityManifest(parsed.runtimeAuthorityManifest, `${artifactPath}.runtimeAuthorityManifest`)
    }
  }
  if (parsed?.schema === "arroba.drill.validation_suite_run.v1") {
    validateValidationSuiteRunArtifact(parsed, artifactPath)
    validateValidationSuiteArtifactMetadata({
      artifactPath,
      expectedKind: "validation-suite-run",
      metadata,
      status: parsed.status,
      testCount: parsed.manifest.testCount,
    })
    if (requiresRuntimeSignalManifest && parsed.manifest?.runtimeSignalsManifest === undefined) {
      throw new Error(`drill artifact ${artifactPath} is missing manifest.runtimeSignalsManifest`)
    }
    if (requiresFailureTaxonomyManifest && parsed.manifest?.failureTaxonomyManifest === undefined) {
      throw new Error(`drill artifact ${artifactPath} is missing manifest.failureTaxonomyManifest`)
    }
    if (requiresRuntimeAuthorityManifest && parsed.manifest?.runtimeAuthorityManifest === undefined) {
      throw new Error(`drill artifact ${artifactPath} is missing manifest.runtimeAuthorityManifest`)
    }
    if (parsed.manifest?.failureTaxonomyManifest !== undefined) {
      validateDrillFailureTaxonomyManifest(parsed.manifest.failureTaxonomyManifest, `${artifactPath}.manifest.failureTaxonomyManifest`)
    }
    if (parsed.manifest?.runtimeSignalsManifest !== undefined) {
      validateDrillRuntimeSignalsManifest(parsed.manifest.runtimeSignalsManifest, `${artifactPath}.manifest.runtimeSignalsManifest`)
    }
    if (parsed.manifest?.runtimeAuthorityManifest !== undefined) {
      validateDrillRuntimeAuthorityManifest(parsed.manifest.runtimeAuthorityManifest, `${artifactPath}.manifest.runtimeAuthorityManifest`)
    }
  }
  if (parsed?.schema === "arroba.drill.matrix.v1") {
    validateDrillMatrixReport(parsed, artifactPath)
    validateMatrixArtifactMetadata(parsed, artifactPath, metadata)
  }
  if (parsed?.schema === "arroba.drill.focused_runtime_gate.v1") {
    validateDrillFocusedRuntimeGateReport(parsed, artifactPath)
    validateFocusedRuntimeGateArtifactMetadata(parsed, artifactPath, metadata)
  }
}

function validateFocusedRuntimeGateArtifactMetadata(report, artifactPath, metadata) {
  const artifactKinds = metadataListFromMetadata(metadata, "artifactKinds")
  if (artifactKinds.length > 0 && !artifactKinds.includes("focused-runtime-gate")) {
    throw new Error(`drill artifact ${artifactPath} metadata.artifactKinds must include focused-runtime-gate`)
  }
  if (metadata?.status !== undefined && metadata.status !== report.status) {
    throw new Error(`drill artifact ${artifactPath} metadata.status must match artifact status`)
  }
}

function validateMatrixArtifactMetadata(report, artifactPath, metadata) {
  const artifactKinds = metadataListFromMetadata(metadata, "artifactKinds")
  if (artifactKinds.length > 0 && !artifactKinds.includes("matrix-report")) {
    throw new Error(`drill artifact ${artifactPath} metadata.artifactKinds must include matrix-report`)
  }
  if (metadata?.matrix !== undefined && metadata.matrix !== report.matrix) {
    throw new Error(`drill artifact ${artifactPath} metadata.matrix must match artifact matrix`)
  }
  if (metadata?.status !== undefined && metadata.status !== report.status) {
    throw new Error(`drill artifact ${artifactPath} metadata.status must match artifact status`)
  }
  if (metadata?.dryRun !== undefined && metadata.dryRun !== report.dryRun) {
    throw new Error(`drill artifact ${artifactPath} metadata.dryRun must match artifact dryRun`)
  }
  if (metadata?.scenarios !== undefined && metadata.scenarios !== report.scenarios.length) {
    throw new Error(`drill artifact ${artifactPath} metadata.scenarios must match artifact scenarios`)
  }
  validateMatrixPlannedMetadata(report, artifactPath, metadata)
}

function validateMatrixPlannedMetadata(report, artifactPath, metadata) {
  const expectedPlannedOwners = plannedMetadataForReport(report, "plannedOwner")
  const expectedPlannedClassifications = plannedMetadataForReport(report, "plannedClassification")
  validateOptionalMetadataListMatches({
    artifactPath,
    field: "plannedOwners",
    actual: metadataListFromMetadata(metadata, "plannedOwners"),
    expected: expectedPlannedOwners,
  })
  validateOptionalMetadataListMatches({
    artifactPath,
    field: "plannedClassifications",
    actual: metadataListFromMetadata(metadata, "plannedClassifications"),
    expected: expectedPlannedClassifications,
  })
}

function plannedMetadataForReport(report, key) {
  return [...new Set((report.scenarios ?? [])
    .map((scenario) => scenario?.[key])
    .filter(nonEmptyString))]
    .sort()
}

function validateOptionalMetadataListMatches({ artifactPath, field, actual, expected }) {
  if (actual.length === 0) return
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(`drill artifact ${artifactPath} metadata.${field} must match artifact planned diagnostics`)
  }
}

function validateValidationSuiteArtifactMetadata({
  artifactPath,
  expectedKind,
  metadata,
  status = null,
  testCount,
}) {
  const artifactKinds = metadataListFromMetadata(metadata, "artifactKinds")
  if (artifactKinds.length > 0 && !artifactKinds.includes(expectedKind)) {
    throw new Error(`drill artifact ${artifactPath} metadata.artifactKinds must include ${expectedKind}`)
  }
  if (metadata?.status !== undefined && metadata.status !== status) {
    throw new Error(`drill artifact ${artifactPath} metadata.status must match artifact status`)
  }
  if (metadata?.tests !== undefined && metadata.tests !== testCount) {
    throw new Error(`drill artifact ${artifactPath} metadata.tests must match artifact testCount`)
  }
}

function validateValidationSuiteRunArtifact(run, source) {
  validateDrillValidationResultStatus(run.status, `drill artifact ${source}`, {
    message: () => `drill artifact ${source} has invalid status`,
  })
  if (typeof run.ok !== "boolean") {
    throw new Error(`drill artifact ${source} is missing ok`)
  }
  if (run.ok !== (run.status === "passed")) {
    throw new Error(`drill artifact ${source} ok does not match status`)
  }
  const startedMs = parseDrillIsoTimestamp(run.startedAt, `drill artifact ${source}.startedAt`)
  const completedMs = parseDrillIsoTimestamp(run.completedAt, `drill artifact ${source}.completedAt`)
  if (completedMs < startedMs) {
    throw new Error(`drill artifact ${source}.completedAt must not be before startedAt`)
  }
  if (!Number.isSafeInteger(run.durationMs) || run.durationMs < 0) {
    throw new Error(`drill artifact ${source} has invalid durationMs`)
  }
  if (run.durationMs !== completedMs - startedMs) {
    throw new Error(`drill artifact ${source}.durationMs must match completedAt - startedAt`)
  }
  if (run.exitCode !== null && (!Number.isSafeInteger(run.exitCode) || run.exitCode < 0)) {
    throw new Error(`drill artifact ${source} has invalid exitCode`)
  }
  if (run.signal !== null && !nonEmptyString(run.signal)) {
    throw new Error(`drill artifact ${source} has invalid signal`)
  }
  if (run.error !== null && typeof run.error !== "string") {
    throw new Error(`drill artifact ${source} has invalid error`)
  }
  if (run.status === "passed" && (run.exitCode !== 0 || run.signal !== null || run.error !== null)) {
    throw new Error(`drill artifact ${source} passed run has failure fields`)
  }
  validateValidationSuiteManifestArtifact(run.manifest, `${source}.manifest`)
  if (!nonEmptyString(run.command) || run.command !== run.manifest.command) {
    throw new Error(`drill artifact ${source}.command must match manifest.command`)
  }
  if (run.testCount !== run.manifest.testCount) {
    throw new Error(`drill artifact ${source}.testCount must match manifest.testCount`)
  }
  if (!Array.isArray(run.testPaths) || JSON.stringify(run.testPaths) !== JSON.stringify(run.manifest.testPaths)) {
    throw new Error(`drill artifact ${source}.testPaths must match manifest.testPaths`)
  }
}

function validateValidationSuiteManifestArtifact(manifest, source) {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`drill artifact ${source} is not an object`)
  }
  if (manifest.schema !== "arroba.drill.validation_suite.v1") {
    throw new Error(`drill artifact ${source} has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  if (!nonEmptyString(manifest.command)) {
    throw new Error(`drill artifact ${source} is missing command`)
  }
  if (!Number.isSafeInteger(manifest.testCount) || manifest.testCount <= 0) {
    throw new Error(`drill artifact ${source} has invalid testCount`)
  }
  if (!Array.isArray(manifest.testPaths) || manifest.testPaths.length !== manifest.testCount) {
    throw new Error(`drill artifact ${source}.testPaths must match testCount`)
  }
  for (const [index, testPath] of manifest.testPaths.entries()) {
    if (!nonEmptyString(testPath)) {
      throw new Error(`drill artifact ${source}.testPaths[${index}] has invalid path`)
    }
  }
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}
