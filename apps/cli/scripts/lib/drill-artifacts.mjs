import { createHash } from "node:crypto"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import { validateDrillAggregateNextAction } from "./drill-aggregate-actions.mjs"
import { validateDrillArtifactKind } from "./drill-artifact-kinds.mjs"
import { validateDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import {
  validateDrillGeneratedEvidenceKind,
  validateDrillGeneratedEvidencePath,
} from "./drill-generated-evidence-metadata.mjs"
import {
  validateDrillGeneratedMatrixName,
  validateDrillGeneratedMatrixNameRepoCounts,
  validateDrillGeneratedMatrixNameRepoMetadata,
} from "./drill-generated-matrix-metadata.mjs"
import { validateDrillGeneratedMatrixLimitation } from "./drill-generated-matrix-limitations.mjs"
import { findDrillJsonArtifactPaths } from "./drill-json-discovery.mjs"
import {
  redactDrillSecretText,
  sanitizeDrillMetadata,
} from "./drill-secrets.mjs"
import { validateDrillMatrixReport } from "./drill-matrix-report.mjs"
import {
  isKnownDrillProvider,
  parseProviderAccountAlias,
} from "./drill-provider-profiles.mjs"
import { isKnownDrillArtifactValidationPreset } from "./drill-validation-gate-presets.mjs"
import { parseDrillIsoTimestamp } from "./drill-time.mjs"
import {
  drillRuntimeSignalOwnersFor,
  isKnownDrillRuntimeSignal,
  validateDrillRuntimeSignals,
  validateDrillRuntimeSignalsManifest,
} from "./drill-runtime-signals.mjs"

export const DRILL_ARTIFACT_INDEX_SCHEMA = "arroba.drill.artifact_index.v1"
export const DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA = "arroba.drill.artifact_index.aggregate.v1"
export const DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS = Object.freeze([
  "runtimeSignals",
  "runtimeSignalOwners",
  "requiredRuntimeSignals",
  "requiredRuntimeSignalOwners",
  "missingRuntimeSignals",
  "missingRuntimeSignalOwners",
  "coverageAreas",
  "validationPresets",
  "owners",
  "classifications",
  "plannedOwners",
  "plannedClassifications",
  "exitCriterionStatuses",
  "incompleteExitCriterionStatuses",
  "artifactKinds",
  "generatedEvidenceKinds",
  "generatedMatrixArtifactIndexes",
  "generatedMatrixLimitations",
  "generatedMatrixNames",
  "generatedMatrixRepos",
  "generatedValidationSuiteArtifactIndexes",
  "generatedValidationSuiteFailureRoots",
  "requiredGeneratedEvidenceKinds",
  "missingGeneratedEvidenceKinds",
  "requiredGeneratedMatrixArtifactIndexes",
  "missingGeneratedMatrixArtifactIndexes",
  "requiredGeneratedMatrixLimitations",
  "missingGeneratedMatrixLimitations",
  "requiredGeneratedMatrixNames",
  "missingGeneratedMatrixNames",
  "requiredGeneratedMatrixRepos",
  "missingGeneratedMatrixRepos",
  "requiredGeneratedValidationSuiteArtifactIndexes",
  "missingGeneratedValidationSuiteArtifactIndexes",
  "providerAccountAliases",
  "evidenceRepos",
  "artifactCoverageInputSources",
])
export const DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS = Object.freeze([
  "schemas",
  ...DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS,
])
const DRILL_ARTIFACT_INDEX_FILE = "arroba-drill-artifacts.json"
const DRILL_ARTIFACT_DIAGNOSTIC_LABELS = Object.freeze({
  runtimeSignals: "runtime_signals",
  runtimeSignalOwners: "runtime_signal_owners",
  requiredRuntimeSignals: "required_runtime_signals",
  requiredRuntimeSignalOwners: "required_runtime_signal_owners",
  missingRuntimeSignals: "missing_runtime_signals",
  missingRuntimeSignalOwners: "missing_runtime_signal_owners",
  coverageAreas: "coverage_areas",
  validationPresets: "validation_presets",
  owners: "owners",
  classifications: "classifications",
  plannedOwners: "planned_owners",
  plannedClassifications: "planned_classifications",
  exitCriterionStatuses: "exit_criterion_statuses",
  incompleteExitCriterionStatuses: "incomplete_exit_criterion_statuses",
  artifactKinds: "artifact_kinds",
  generatedEvidenceKinds: "generated_evidence_kinds",
  generatedMatrixArtifactIndexes: "generated_matrix_artifact_indexes",
  generatedMatrixLimitations: "generated_matrix_limitations",
  generatedMatrixNames: "generated_matrix_names",
  generatedMatrixRepos: "generated_matrix_repos",
  generatedValidationSuiteArtifactIndexes: "generated_validation_suite_artifact_indexes",
  generatedValidationSuiteFailureRoots: "generated_validation_suite_failure_roots",
  requiredGeneratedEvidenceKinds: "required_generated_evidence_kinds",
  missingGeneratedEvidenceKinds: "missing_generated_evidence_kinds",
  requiredGeneratedMatrixArtifactIndexes: "required_generated_matrix_artifact_indexes",
  missingGeneratedMatrixArtifactIndexes: "missing_generated_matrix_artifact_indexes",
  requiredGeneratedMatrixLimitations: "required_generated_matrix_limitations",
  missingGeneratedMatrixLimitations: "missing_generated_matrix_limitations",
  requiredGeneratedMatrixNames: "required_generated_matrix_names",
  missingGeneratedMatrixNames: "missing_generated_matrix_names",
  requiredGeneratedMatrixRepos: "required_generated_matrix_repos",
  missingGeneratedMatrixRepos: "missing_generated_matrix_repos",
  requiredGeneratedValidationSuiteArtifactIndexes: "required_generated_validation_suite_artifact_indexes",
  missingGeneratedValidationSuiteArtifactIndexes: "missing_generated_validation_suite_artifact_indexes",
  providerAccountAliases: "provider_account_aliases",
  evidenceRepos: "evidence_repos",
  artifactCoverageInputSources: "artifact_coverage_input_sources",
})
const DRILL_GENERATED_EVIDENCE_PATH_METADATA_KEYS = Object.freeze([
  "generatedMatrixArtifactIndexes",
  "generatedValidationSuiteArtifactIndexes",
  "generatedValidationSuiteFailureRoots",
  "requiredGeneratedMatrixArtifactIndexes",
  "missingGeneratedMatrixArtifactIndexes",
  "requiredGeneratedValidationSuiteArtifactIndexes",
  "missingGeneratedValidationSuiteArtifactIndexes",
])

export async function prepareDrillArtifacts(rootDir) {
  const resolvedRootDir = resolvedDrillRootDir(rootDir)
  await rm(resolvedRootDir, { recursive: true, force: true }).catch(() => {})
  await mkdir(resolvedRootDir, { recursive: true })
  return resolvedRootDir
}

export async function finalizeDrillArtifacts({
  rootDir,
  passed,
  log = null,
  preserveOnFailure = true,
  failure = null,
  metadata = {},
}) {
  const resolvedRootDir = resolvedDrillRootDir(rootDir)
  if (passed || !preserveOnFailure) {
    await rm(resolvedRootDir, { recursive: true, force: true }).catch(() => {})
    if (!passed && log) {
      log("discarded-failed-run", { rootDir: resolvedRootDir })
    }
    return { preserved: false, rootDir: resolvedRootDir }
  }

  await mkdir(resolvedRootDir, { recursive: true }).catch(() => {})
  const manifest = failureManifest({ rootDir: resolvedRootDir, failure, metadata })
  const manifestPath = path.join(resolvedRootDir, "arroba-drill-failure.json")
  await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8").catch(() => {})
  if (log) {
    log("preserved-failed-run", { rootDir: resolvedRootDir, manifestPath })
  }
  return { preserved: true, rootDir: resolvedRootDir, manifestPath }
}

export async function writeDrillArtifactIndex({
  rootDir,
  artifacts,
  indexPath = null,
  metadata = {},
}) {
  if (!Array.isArray(artifacts) || artifacts.length === 0) {
    throw new Error("drill artifact index requires artifacts")
  }
  if (!nonEmptyString(rootDir)) {
    throw new Error("drill artifact rootDir is required")
  }
  const resolvedRootDir = path.resolve(rootDir)
  const resolvedIndexPath = indexPath ?? path.join(resolvedRootDir, DRILL_ARTIFACT_INDEX_FILE)
  const records = []
  for (const artifact of artifacts) {
    records.push(await artifactRecord(resolvedRootDir, artifact))
  }
  const index = {
    schema: DRILL_ARTIFACT_INDEX_SCHEMA,
    rootDir: resolvedRootDir,
    createdAt: new Date().toISOString(),
    metadata: sanitizeDrillMetadata(metadata),
    artifacts: records.sort((left, right) => left.path.localeCompare(right.path)),
  }
  validateDrillArtifactIndex(index)
  await mkdir(path.dirname(resolvedIndexPath), { recursive: true })
  await writeFile(resolvedIndexPath, `${JSON.stringify(index, null, 2)}\n`, "utf8")
  return index
}

export async function writeDrillJsonArtifactOutput({
  outputPath,
  value,
  artifactIndexPath = null,
  metadata = {},
}) {
  await mkdir(path.dirname(outputPath), { recursive: true })
  await writeFile(outputPath, `${JSON.stringify(value, null, 2)}\n`, "utf8")
  if (!artifactIndexPath) return null
  return await writeDrillArtifactIndex({
    rootDir: path.dirname(path.resolve(outputPath)),
    artifacts: [path.basename(outputPath)],
    indexPath: artifactIndexPath,
    metadata,
  })
}

export async function findDrillArtifactIndexPaths(roots, { maxDepth = 8 } = {}) {
  return await findDrillJsonArtifactPaths(roots, {
    fileName: DRILL_ARTIFACT_INDEX_FILE,
    maxDepth,
    schema: DRILL_ARTIFACT_INDEX_SCHEMA,
  })
}

export async function readDrillArtifactIndex(indexPath) {
  const index = JSON.parse(await readFile(indexPath, "utf8"))
  validateDrillArtifactIndex(index, indexPath)
  return index
}

export async function verifyDrillArtifactIndex(indexPath) {
  const index = await readDrillArtifactIndex(indexPath)
  for (const artifact of index.artifacts) {
    const contents = await readFile(path.join(index.rootDir, artifact.path))
    if (sha256(contents) !== artifact.sha256) {
      throw new Error(`drill artifact ${artifact.path} sha256 mismatch`)
    }
    if (contents.byteLength !== artifact.sizeBytes) {
      throw new Error(`drill artifact ${artifact.path} sizeBytes mismatch`)
    }
    const schema = schemaForArtifact(contents)
    if (schema !== artifact.schema) {
      throw new Error(`drill artifact ${artifact.path} schema mismatch`)
    }
    validateKnownArtifactContents(contents, artifact.path, index.metadata)
  }
  return index
}

export function summarizeDrillArtifactIndexes(indexes, { sources = [] } = {}) {
  const totals = {
    indexes: indexes.length,
    artifacts: 0,
    sizeBytes: 0,
  }
  const schemas = new Map()
  const runtimeSignals = new Map()
  const runtimeSignalOwners = new Map()
  const requiredRuntimeSignals = new Map()
  const requiredRuntimeSignalOwners = new Map()
  const missingRuntimeSignals = new Map()
  const missingRuntimeSignalOwners = new Map()
  const coverageAreas = new Map()
  const validationPresets = new Map()
  const owners = new Map()
  const classifications = new Map()
  const plannedOwners = new Map()
  const plannedClassifications = new Map()
  const exitCriterionStatuses = new Map()
  const incompleteExitCriterionStatuses = new Map()
  const artifactKinds = new Map()
  const generatedEvidenceKinds = new Map()
  const generatedMatrixArtifactIndexes = new Map()
  const generatedMatrixLimitations = new Map()
  const generatedMatrixNames = new Map()
  const generatedMatrixRepos = new Map()
  const generatedValidationSuiteArtifactIndexes = new Map()
  const generatedValidationSuiteFailureRoots = new Map()
  const requiredGeneratedEvidenceKinds = new Map()
  const missingGeneratedEvidenceKinds = new Map()
  const requiredGeneratedMatrixArtifactIndexes = new Map()
  const missingGeneratedMatrixArtifactIndexes = new Map()
  const requiredGeneratedMatrixLimitations = new Map()
  const missingGeneratedMatrixLimitations = new Map()
  const requiredGeneratedMatrixNames = new Map()
  const missingGeneratedMatrixNames = new Map()
  const requiredGeneratedMatrixRepos = new Map()
  const missingGeneratedMatrixRepos = new Map()
  const requiredGeneratedValidationSuiteArtifactIndexes = new Map()
  const missingGeneratedValidationSuiteArtifactIndexes = new Map()
  const providerAccountAliases = new Map()
  const evidenceRepos = new Map()
  const artifactCoverageInputSources = new Map()
  const summaries = indexes.map((index, indexPosition) => {
    validateDrillArtifactIndex(index, sources[indexPosition] ?? "drill artifact index")
    const indexTotals = {
      artifacts: index.artifacts.length,
      sizeBytes: 0,
    }
    const indexSchemas = new Map()
    const indexRuntimeSignals = runtimeSignalsFromMetadata(index.metadata)
    const indexRuntimeSignalOwners = runtimeSignalOwnersFromRuntimeSignals(indexRuntimeSignals)
    const indexRequiredRuntimeSignals = metadataListFromMetadata(index.metadata, "requiredRuntimeSignals")
    const indexRequiredRuntimeSignalOwners = runtimeSignalOwnersFromRuntimeSignals(indexRequiredRuntimeSignals)
    const indexMissingRuntimeSignals = metadataListFromMetadata(index.metadata, "missingRuntimeSignals")
    const indexMissingRuntimeSignalOwners = runtimeSignalOwnersFromRuntimeSignals(indexMissingRuntimeSignals)
    const indexCoverageAreas = metadataListFromMetadata(index.metadata, "coverageAreas")
    const indexValidationPresets = metadataListFromMetadata(index.metadata, "validationPresets")
    const indexOwners = metadataListFromMetadata(index.metadata, "owners")
    const indexClassifications = metadataListFromMetadata(index.metadata, "classifications")
    const indexPlannedOwners = metadataListFromMetadata(index.metadata, "plannedOwners")
    const indexPlannedClassifications = metadataListFromMetadata(index.metadata, "plannedClassifications")
    const indexExitCriterionStatuses = metadataListFromMetadata(index.metadata, "exitCriterionStatuses")
    const indexIncompleteExitCriterionStatuses = metadataListFromMetadata(index.metadata, "incompleteExitCriterionStatuses")
    const indexArtifactKinds = metadataListFromMetadata(index.metadata, "artifactKinds")
    const indexGeneratedEvidenceKinds = metadataListFromMetadata(index.metadata, "generatedEvidenceKinds")
    const indexGeneratedMatrixArtifactIndexes = metadataListFromMetadata(index.metadata, "generatedMatrixArtifactIndexes")
    const indexGeneratedMatrixLimitations = metadataListFromMetadata(index.metadata, "generatedMatrixLimitations")
    const indexGeneratedMatrixNames = metadataListFromMetadata(index.metadata, "generatedMatrixNames")
    const indexGeneratedMatrixRepos = metadataListFromMetadata(index.metadata, "generatedMatrixRepos")
    const indexGeneratedValidationSuiteArtifactIndexes = metadataListFromMetadata(index.metadata, "generatedValidationSuiteArtifactIndexes")
    const indexGeneratedValidationSuiteFailureRoots = metadataListFromMetadata(index.metadata, "generatedValidationSuiteFailureRoots")
    const indexRequiredGeneratedEvidenceKinds = metadataListFromMetadata(index.metadata, "requiredGeneratedEvidenceKinds")
    const indexMissingGeneratedEvidenceKinds = metadataListFromMetadata(index.metadata, "missingGeneratedEvidenceKinds")
    const indexRequiredGeneratedMatrixArtifactIndexes = metadataListFromMetadata(index.metadata, "requiredGeneratedMatrixArtifactIndexes")
    const indexMissingGeneratedMatrixArtifactIndexes = metadataListFromMetadata(index.metadata, "missingGeneratedMatrixArtifactIndexes")
    const indexRequiredGeneratedMatrixLimitations = metadataListFromMetadata(index.metadata, "requiredGeneratedMatrixLimitations")
    const indexMissingGeneratedMatrixLimitations = metadataListFromMetadata(index.metadata, "missingGeneratedMatrixLimitations")
    const indexRequiredGeneratedMatrixNames = metadataListFromMetadata(index.metadata, "requiredGeneratedMatrixNames")
    const indexMissingGeneratedMatrixNames = metadataListFromMetadata(index.metadata, "missingGeneratedMatrixNames")
    const indexRequiredGeneratedMatrixRepos = metadataListFromMetadata(index.metadata, "requiredGeneratedMatrixRepos")
    const indexMissingGeneratedMatrixRepos = metadataListFromMetadata(index.metadata, "missingGeneratedMatrixRepos")
    const indexRequiredGeneratedValidationSuiteArtifactIndexes = metadataListFromMetadata(index.metadata, "requiredGeneratedValidationSuiteArtifactIndexes")
    const indexMissingGeneratedValidationSuiteArtifactIndexes = metadataListFromMetadata(index.metadata, "missingGeneratedValidationSuiteArtifactIndexes")
    const indexProviderAccountAliases = metadataListFromMetadata(index.metadata, "providerAccountAliases")
    const indexEvidenceRepos = metadataListFromMetadata(index.metadata, "evidenceRepos")
    const indexArtifactCoverageInputSources = metadataListFromMetadata(index.metadata, "artifactCoverageInputSources")
    countValues(indexRuntimeSignals, runtimeSignals)
    countValues(indexRuntimeSignalOwners, runtimeSignalOwners)
    countValues(indexRequiredRuntimeSignals, requiredRuntimeSignals)
    countValues(indexRequiredRuntimeSignalOwners, requiredRuntimeSignalOwners)
    countValues(indexMissingRuntimeSignals, missingRuntimeSignals)
    countValues(indexMissingRuntimeSignalOwners, missingRuntimeSignalOwners)
    countValues(indexCoverageAreas, coverageAreas)
    countValues(indexValidationPresets, validationPresets)
    countValues(indexOwners, owners)
    countValues(indexClassifications, classifications)
    countValues(indexPlannedOwners, plannedOwners)
    countValues(indexPlannedClassifications, plannedClassifications)
    countValues(indexExitCriterionStatuses, exitCriterionStatuses)
    countValues(indexIncompleteExitCriterionStatuses, incompleteExitCriterionStatuses)
    countValues(indexArtifactKinds, artifactKinds)
    countValues(indexGeneratedEvidenceKinds, generatedEvidenceKinds)
    countValues(indexGeneratedMatrixArtifactIndexes, generatedMatrixArtifactIndexes)
    countValues(indexGeneratedMatrixLimitations, generatedMatrixLimitations)
    countValues(indexGeneratedMatrixNames, generatedMatrixNames)
    countValues(indexGeneratedMatrixRepos, generatedMatrixRepos)
    countValues(indexGeneratedValidationSuiteArtifactIndexes, generatedValidationSuiteArtifactIndexes)
    countValues(indexGeneratedValidationSuiteFailureRoots, generatedValidationSuiteFailureRoots)
    countValues(indexRequiredGeneratedEvidenceKinds, requiredGeneratedEvidenceKinds)
    countValues(indexMissingGeneratedEvidenceKinds, missingGeneratedEvidenceKinds)
    countValues(indexRequiredGeneratedMatrixArtifactIndexes, requiredGeneratedMatrixArtifactIndexes)
    countValues(indexMissingGeneratedMatrixArtifactIndexes, missingGeneratedMatrixArtifactIndexes)
    countValues(indexRequiredGeneratedMatrixLimitations, requiredGeneratedMatrixLimitations)
    countValues(indexMissingGeneratedMatrixLimitations, missingGeneratedMatrixLimitations)
    countValues(indexRequiredGeneratedMatrixNames, requiredGeneratedMatrixNames)
    countValues(indexMissingGeneratedMatrixNames, missingGeneratedMatrixNames)
    countValues(indexRequiredGeneratedMatrixRepos, requiredGeneratedMatrixRepos)
    countValues(indexMissingGeneratedMatrixRepos, missingGeneratedMatrixRepos)
    countValues(indexRequiredGeneratedValidationSuiteArtifactIndexes, requiredGeneratedValidationSuiteArtifactIndexes)
    countValues(indexMissingGeneratedValidationSuiteArtifactIndexes, missingGeneratedValidationSuiteArtifactIndexes)
    countValues(indexProviderAccountAliases, providerAccountAliases)
    countValues(indexEvidenceRepos, evidenceRepos)
    countValues(indexArtifactCoverageInputSources, artifactCoverageInputSources)
    for (const artifact of index.artifacts) {
      totals.artifacts += 1
      totals.sizeBytes += artifact.sizeBytes
      indexTotals.sizeBytes += artifact.sizeBytes
      const schema = artifact.schema ?? "none"
      schemas.set(schema, (schemas.get(schema) ?? 0) + 1)
      indexSchemas.set(schema, (indexSchemas.get(schema) ?? 0) + 1)
    }
    return {
      source: sources[indexPosition] ?? null,
      rootDir: index.rootDir,
      artifacts: indexTotals.artifacts,
      sizeBytes: indexTotals.sizeBytes,
      schemas: sortedCountObject(indexSchemas),
      runtimeSignals: countValues(indexRuntimeSignals),
      runtimeSignalOwners: countValues(indexRuntimeSignalOwners),
      requiredRuntimeSignals: countValues(indexRequiredRuntimeSignals),
      requiredRuntimeSignalOwners: countValues(indexRequiredRuntimeSignalOwners),
      missingRuntimeSignals: countValues(indexMissingRuntimeSignals),
      missingRuntimeSignalOwners: countValues(indexMissingRuntimeSignalOwners),
      coverageAreas: countValues(indexCoverageAreas),
      validationPresets: countValues(indexValidationPresets),
      owners: countValues(indexOwners),
      classifications: countValues(indexClassifications),
      plannedOwners: countValues(indexPlannedOwners),
      plannedClassifications: countValues(indexPlannedClassifications),
      exitCriterionStatuses: countValues(indexExitCriterionStatuses),
      incompleteExitCriterionStatuses: countValues(indexIncompleteExitCriterionStatuses),
      artifactKinds: countValues(indexArtifactKinds),
      generatedEvidenceKinds: countValues(indexGeneratedEvidenceKinds),
      generatedMatrixArtifactIndexes: countValues(indexGeneratedMatrixArtifactIndexes),
      generatedMatrixLimitations: countValues(indexGeneratedMatrixLimitations),
      generatedMatrixNames: countValues(indexGeneratedMatrixNames),
      generatedMatrixRepos: countValues(indexGeneratedMatrixRepos),
      generatedValidationSuiteArtifactIndexes: countValues(indexGeneratedValidationSuiteArtifactIndexes),
      generatedValidationSuiteFailureRoots: countValues(indexGeneratedValidationSuiteFailureRoots),
      requiredGeneratedEvidenceKinds: countValues(indexRequiredGeneratedEvidenceKinds),
      missingGeneratedEvidenceKinds: countValues(indexMissingGeneratedEvidenceKinds),
      requiredGeneratedMatrixArtifactIndexes: countValues(indexRequiredGeneratedMatrixArtifactIndexes),
      missingGeneratedMatrixArtifactIndexes: countValues(indexMissingGeneratedMatrixArtifactIndexes),
      requiredGeneratedMatrixLimitations: countValues(indexRequiredGeneratedMatrixLimitations),
      missingGeneratedMatrixLimitations: countValues(indexMissingGeneratedMatrixLimitations),
      requiredGeneratedMatrixNames: countValues(indexRequiredGeneratedMatrixNames),
      missingGeneratedMatrixNames: countValues(indexMissingGeneratedMatrixNames),
      requiredGeneratedMatrixRepos: countValues(indexRequiredGeneratedMatrixRepos),
      missingGeneratedMatrixRepos: countValues(indexMissingGeneratedMatrixRepos),
      requiredGeneratedValidationSuiteArtifactIndexes: countValues(indexRequiredGeneratedValidationSuiteArtifactIndexes),
      missingGeneratedValidationSuiteArtifactIndexes: countValues(indexMissingGeneratedValidationSuiteArtifactIndexes),
      providerAccountAliases: countValues(indexProviderAccountAliases),
      evidenceRepos: countValues(indexEvidenceRepos),
      artifactCoverageInputSources: countValues(indexArtifactCoverageInputSources),
    }
  })
  const aggregate = {
    schema: DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA,
    totals,
    schemas: sortedCountObject(schemas),
    runtimeSignals: sortedCountObject(runtimeSignals),
    runtimeSignalOwners: sortedCountObject(runtimeSignalOwners),
    requiredRuntimeSignals: sortedCountObject(requiredRuntimeSignals),
    requiredRuntimeSignalOwners: sortedCountObject(requiredRuntimeSignalOwners),
    missingRuntimeSignals: sortedCountObject(missingRuntimeSignals),
    missingRuntimeSignalOwners: sortedCountObject(missingRuntimeSignalOwners),
    coverageAreas: sortedCountObject(coverageAreas),
    validationPresets: sortedCountObject(validationPresets),
    owners: sortedCountObject(owners),
    classifications: sortedCountObject(classifications),
    plannedOwners: sortedCountObject(plannedOwners),
    plannedClassifications: sortedCountObject(plannedClassifications),
    exitCriterionStatuses: sortedCountObject(exitCriterionStatuses),
    incompleteExitCriterionStatuses: sortedCountObject(incompleteExitCriterionStatuses),
    artifactKinds: sortedCountObject(artifactKinds),
    generatedEvidenceKinds: sortedCountObject(generatedEvidenceKinds),
    generatedMatrixArtifactIndexes: sortedCountObject(generatedMatrixArtifactIndexes),
    generatedMatrixLimitations: sortedCountObject(generatedMatrixLimitations),
    generatedMatrixNames: sortedCountObject(generatedMatrixNames),
    generatedMatrixRepos: sortedCountObject(generatedMatrixRepos),
    generatedValidationSuiteArtifactIndexes: sortedCountObject(generatedValidationSuiteArtifactIndexes),
    generatedValidationSuiteFailureRoots: sortedCountObject(generatedValidationSuiteFailureRoots),
    requiredGeneratedEvidenceKinds: sortedCountObject(requiredGeneratedEvidenceKinds),
    missingGeneratedEvidenceKinds: sortedCountObject(missingGeneratedEvidenceKinds),
    requiredGeneratedMatrixArtifactIndexes: sortedCountObject(requiredGeneratedMatrixArtifactIndexes),
    missingGeneratedMatrixArtifactIndexes: sortedCountObject(missingGeneratedMatrixArtifactIndexes),
    requiredGeneratedMatrixLimitations: sortedCountObject(requiredGeneratedMatrixLimitations),
    missingGeneratedMatrixLimitations: sortedCountObject(missingGeneratedMatrixLimitations),
    requiredGeneratedMatrixNames: sortedCountObject(requiredGeneratedMatrixNames),
    missingGeneratedMatrixNames: sortedCountObject(missingGeneratedMatrixNames),
    requiredGeneratedMatrixRepos: sortedCountObject(requiredGeneratedMatrixRepos),
    missingGeneratedMatrixRepos: sortedCountObject(missingGeneratedMatrixRepos),
    requiredGeneratedValidationSuiteArtifactIndexes: sortedCountObject(requiredGeneratedValidationSuiteArtifactIndexes),
    missingGeneratedValidationSuiteArtifactIndexes: sortedCountObject(missingGeneratedValidationSuiteArtifactIndexes),
    providerAccountAliases: sortedCountObject(providerAccountAliases),
    evidenceRepos: sortedCountObject(evidenceRepos),
    artifactCoverageInputSources: sortedCountObject(artifactCoverageInputSources),
    indexes: summaries,
  }
  validateDrillArtifactIndexAggregate(aggregate)
  return aggregate
}

export function diagnosticMetadataForDrillArtifactIndexAggregate(aggregate) {
  validateDrillArtifactDiagnosticDimensions(aggregate)
  const metadata = Object.fromEntries(DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS
    .map((key) => [key, Object.keys(aggregate[key] ?? {}).sort().join(",")])
    .filter(([, value]) => value.length > 0))
  const artifactCoverageInputCount = Object.values(aggregate.artifactCoverageInputSources ?? {})
    .reduce((sum, count) => sum + count, 0)
  return {
    ...metadata,
    ...(artifactCoverageInputCount > 0 ? { artifactCoverageInputCount: String(artifactCoverageInputCount) } : {}),
  }
}

export function formatDrillArtifactIndexAggregateSummary(aggregate) {
  validateDrillArtifactIndexAggregate(aggregate)
  const lines = [
    "drill artifact index aggregate:",
    `indexes=${aggregate.totals.indexes} artifacts=${aggregate.totals.artifacts} size_bytes=${aggregate.totals.sizeBytes}`,
  ]
  const schemas = Object.entries(aggregate.schemas)
  if (schemas.length > 0) {
    lines.push(`schemas: ${schemas.map(([schema, count]) => `${schema}=${count}`).join(" ")}`)
  }
  for (const key of DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS) {
    const entries = Object.entries(aggregate[key] ?? {})
    if (entries.length > 0) {
      lines.push(`${DRILL_ARTIFACT_DIAGNOSTIC_LABELS[key]}: ${entries.map(([name, count]) => `${name}=${count}`).join(" ")}`)
    }
  }
  const artifactCoverageInputCount = Object.values(aggregate.artifactCoverageInputSources ?? {})
    .reduce((sum, count) => sum + count, 0)
  if (artifactCoverageInputCount > 0) {
    lines.push(`artifact_coverage_input_count=${artifactCoverageInputCount}`)
  }
  lines.push("next: verify indexed artifacts before using them as validation evidence")
  return lines.join("\n")
}

export function validateDrillArtifactDiagnosticDimensions(value, source = "drill artifact diagnostics") {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is not an object`)
  }
  for (const key of DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS) {
    if (!Object.hasOwn(value, key)) {
      throw new Error(`${source} is missing ${key}`)
    }
    validateDiagnosticCountObject(value[key], `${source}.${key}`, key)
  }
  validateDrillGeneratedMatrixNameRepoCounts(value.generatedMatrixNames, value.generatedMatrixRepos, source)
  validateRuntimeSignalOwnerKeysMatch(value.runtimeSignals, value.runtimeSignalOwners, source)
  validateRuntimeSignalOwnerKeysMatch(
    value.requiredRuntimeSignals,
    value.requiredRuntimeSignalOwners,
    source,
    "requiredRuntimeSignalOwners",
    "requiredRuntimeSignals",
  )
  validateRuntimeSignalOwnerKeysMatch(
    value.missingRuntimeSignals,
    value.missingRuntimeSignalOwners,
    source,
    "missingRuntimeSignalOwners",
    "missingRuntimeSignals",
  )
}

export function validateDrillArtifactIndex(index, source = "drill artifact index") {
  if (!index || typeof index !== "object" || Array.isArray(index)) {
    throw new Error(`${source} is not an object`)
  }
  if (index.schema !== DRILL_ARTIFACT_INDEX_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(index.schema)}`)
  }
  if (!nonEmptyString(index.rootDir) || !path.isAbsolute(index.rootDir)) {
    throw new Error(`${source} has invalid rootDir`)
  }
  if (!nonEmptyString(index.createdAt)) {
    throw new Error(`${source} is missing createdAt`)
  }
  parseDrillIsoTimestamp(index.createdAt, `${source}.createdAt`)
  if (!index.metadata || typeof index.metadata !== "object" || Array.isArray(index.metadata)) {
    throw new Error(`${source} has invalid metadata`)
  }
  validateDrillArtifactIndexKindMetadata(index.metadata, `${source}.metadata`)
  validateDrillArtifactIndexGeneratedEvidenceMetadata(index.metadata, `${source}.metadata`)
  validateDrillArtifactIndexEvidenceRepoMetadata(index.metadata, `${source}.metadata`)
  validateDrillArtifactIndexProviderAccountAliasMetadata(index.metadata, `${source}.metadata`)
  validateDrillArtifactIndexRuntimeSignalOwnerMetadata(index.metadata, `${source}.metadata`)
  if (!Array.isArray(index.artifacts) || index.artifacts.length === 0) {
    throw new Error(`${source} has invalid artifacts`)
  }
  const seen = new Set()
  for (const [artifactIndex, artifact] of index.artifacts.entries()) {
    validateArtifactIndexRecord(artifact, `${source}.artifacts[${artifactIndex}]`)
    if (seen.has(artifact.path)) {
      throw new Error(`${source} has duplicate artifact ${artifact.path}`)
    }
    seen.add(artifact.path)
  }
}

export function validateDrillArtifactIndexAggregate(aggregate, source = "drill artifact index aggregate") {
  if (!aggregate || typeof aggregate !== "object" || Array.isArray(aggregate)) {
    throw new Error(`${source} is not an object`)
  }
  if (aggregate.schema !== DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(aggregate.schema)}`)
  }
  if (!aggregate.totals || typeof aggregate.totals !== "object" || Array.isArray(aggregate.totals)) {
    throw new Error(`${source} has invalid totals`)
  }
  for (const key of ["indexes", "artifacts", "sizeBytes"]) {
    if (!Number.isSafeInteger(aggregate.totals[key]) || aggregate.totals[key] < 0) {
      throw new Error(`${source}.totals has invalid ${key}`)
    }
  }
  for (const key of DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS) {
    if (!Object.hasOwn(aggregate, key)) {
      throw new Error(`${source} is missing ${key}`)
    }
    validateDiagnosticCountObject(aggregate[key], `${source}.${key}`, key)
  }
  validateDrillGeneratedMatrixNameRepoCounts(aggregate.generatedMatrixNames, aggregate.generatedMatrixRepos, source)
  if (!Array.isArray(aggregate.indexes)) {
    throw new Error(`${source} has invalid indexes`)
  }
  for (const [index, summary] of aggregate.indexes.entries()) {
    validateArtifactIndexSummary(summary, `${source}.indexes[${index}]`)
  }
  validateDrillArtifactIndexAggregateNextActions(aggregate.nextActions, `${source}.nextActions`)
  if (aggregate.totals.indexes !== aggregate.indexes.length) {
    throw new Error(`${source} totals.indexes does not match indexes`)
  }
  const expectedArtifacts = aggregate.indexes.reduce((sum, index) => sum + index.artifacts, 0)
  const expectedSizeBytes = aggregate.indexes.reduce((sum, index) => sum + index.sizeBytes, 0)
  const expectedAggregateCounts = Object.fromEntries(
    DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS.map((key) => [key, new Map()]),
  )
  for (const index of aggregate.indexes) {
    for (const [key, expectedCounts] of Object.entries(expectedAggregateCounts)) {
      for (const [value, count] of Object.entries(index[key] ?? {})) {
        expectedCounts.set(value, (expectedCounts.get(value) ?? 0) + count)
      }
    }
  }
  if (aggregate.totals.artifacts !== expectedArtifacts || aggregate.totals.sizeBytes !== expectedSizeBytes) {
    throw new Error(`${source} totals do not match indexes`)
  }
  for (const [key, expectedCounts] of Object.entries(expectedAggregateCounts)) {
    if (JSON.stringify(aggregate[key] ?? {}) !== JSON.stringify(sortedCountObject(expectedCounts))) {
      throw new Error(`${source} ${key} do not match indexes`)
    }
  }
}

function validateDrillArtifactIndexAggregateNextActions(nextActions, source) {
  if (nextActions === undefined) return
  if (!Array.isArray(nextActions)) {
    throw new Error(`${source} is invalid`)
  }
  for (const [index, action] of nextActions.entries()) {
    validateDrillAggregateNextAction(action, `${source}[${index}]`)
  }
}

function failureManifest({ rootDir, failure, metadata }) {
  return {
    schema: "arroba.drill.failure.v1",
    rootDir,
    failedAt: new Date().toISOString(),
    metadata: sanitizeDrillMetadata(metadata),
    error: failure
      ? {
          name: failure.name ?? "Error",
          message: redactDrillSecretText(failure.message ?? String(failure)),
          stack: typeof failure.stack === "string" ? redactDrillSecretText(failure.stack) : null,
        }
      : null,
  }
}

function resolvedDrillRootDir(rootDir) {
  if (!nonEmptyString(rootDir)) throw new Error("drill artifact rootDir is required")
  return path.resolve(rootDir)
}

async function artifactRecord(rootDir, artifact) {
  const input = typeof artifact === "string" ? { path: artifact } : artifact
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new Error("drill artifact entry is not an object")
  }
  const relativePath = relativeArtifactPath(rootDir, input.path)
  const contents = await readFile(path.join(rootDir, relativePath))
  return {
    path: relativePath,
    schema: schemaForArtifact(contents),
    sha256: sha256(contents),
    sizeBytes: contents.byteLength,
  }
}

function validateArtifactIndexRecord(artifact, source) {
  if (!artifact || typeof artifact !== "object" || Array.isArray(artifact)) {
    throw new Error(`${source} is not an object`)
  }
  if (!safeRelativePath(artifact.path)) {
    throw new Error(`${source} has unsafe path ${JSON.stringify(artifact.path)}`)
  }
  if (artifact.schema !== null && !nonEmptyString(artifact.schema)) {
    throw new Error(`${source} has invalid schema`)
  }
  if (typeof artifact.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(artifact.sha256)) {
    throw new Error(`${source} has invalid sha256`)
  }
  if (!Number.isSafeInteger(artifact.sizeBytes) || artifact.sizeBytes < 0) {
    throw new Error(`${source} has invalid sizeBytes`)
  }
}

function validateArtifactIndexSummary(summary, source) {
  if (!summary || typeof summary !== "object" || Array.isArray(summary)) {
    throw new Error(`${source} is not an object`)
  }
  if (summary.source !== null && typeof summary.source !== "string") {
    throw new Error(`${source} has invalid source`)
  }
  if (!nonEmptyString(summary.rootDir) || !path.isAbsolute(summary.rootDir)) {
    throw new Error(`${source} has invalid rootDir`)
  }
  for (const key of ["artifacts", "sizeBytes"]) {
    if (!Number.isSafeInteger(summary[key]) || summary[key] < 0) {
      throw new Error(`${source} has invalid ${key}`)
    }
  }
  for (const key of DRILL_ARTIFACT_AGGREGATE_COUNT_KEYS) {
    if (!Object.hasOwn(summary, key)) {
      throw new Error(`${source} is missing ${key}`)
    }
    validateDiagnosticCountObject(summary[key], `${source}.${key}`, key)
  }
  validateDrillGeneratedMatrixNameRepoCounts(summary.generatedMatrixNames, summary.generatedMatrixRepos, source)
  validateRuntimeSignalOwnerCountsMatch(summary.runtimeSignals, summary.runtimeSignalOwners, source)
  validateRuntimeSignalOwnerCountsMatch(
    summary.requiredRuntimeSignals,
    summary.requiredRuntimeSignalOwners,
    source,
    "requiredRuntimeSignalOwners",
    "requiredRuntimeSignals",
  )
  validateRuntimeSignalOwnerCountsMatch(
    summary.missingRuntimeSignals,
    summary.missingRuntimeSignalOwners,
    source,
    "missingRuntimeSignalOwners",
    "missingRuntimeSignals",
  )
}

function validateCountObject(value, source) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is not an object`)
  }
  for (const [key, count] of Object.entries(value)) {
    if (!nonEmptyString(key) || !Number.isSafeInteger(count) || count < 0) {
      throw new Error(`${source} has invalid count`)
    }
  }
}

function validateDiagnosticCountObject(value, source, key) {
  validateCountObject(value, source)
  if (DRILL_GENERATED_EVIDENCE_PATH_METADATA_KEYS.includes(key)) {
    for (const valueKey of Object.keys(value)) {
      validateGeneratedEvidencePathText(valueKey, `${source}.${valueKey}`)
    }
  }
  if (key === "artifactKinds") {
    for (const kind of Object.keys(value)) {
      validateDrillArtifactKind(kind, source)
    }
  }
  if ([
    "generatedEvidenceKinds",
    "requiredGeneratedEvidenceKinds",
    "missingGeneratedEvidenceKinds",
  ].includes(key)) {
    for (const kind of Object.keys(value)) {
      validateDrillGeneratedEvidenceKind(kind, source)
    }
  }
  if ([
    "generatedMatrixLimitations",
    "requiredGeneratedMatrixLimitations",
    "missingGeneratedMatrixLimitations",
  ].includes(key)) {
    for (const limitation of Object.keys(value)) {
      validateDrillGeneratedMatrixLimitation(limitation, source)
    }
  }
  if (["generatedMatrixRepos", "requiredGeneratedMatrixRepos", "missingGeneratedMatrixRepos"].includes(key)) {
    for (const repo of Object.keys(value)) {
      validateDrillArtifactEvidenceRepo(repo, source)
    }
  }
  if (["generatedMatrixNames", "requiredGeneratedMatrixNames", "missingGeneratedMatrixNames"].includes(key)) {
    for (const matrixName of Object.keys(value)) {
      validateDrillGeneratedMatrixName(matrixName, {
        secretSource: `${source}.${matrixName}`,
        unknownSource: source,
      })
    }
  }
  if (["runtimeSignals", "requiredRuntimeSignals", "missingRuntimeSignals"].includes(key)) {
    for (const signal of Object.keys(value)) {
      if (!isKnownDrillRuntimeSignal(signal)) {
        throw new Error(`${source} has unknown runtime signal ${JSON.stringify(signal)}`)
      }
    }
  }
  if (key === "exitCriterionStatuses" || key === "incompleteExitCriterionStatuses") {
    for (const status of Object.keys(value)) {
      if (!["satisfied", "failed", "skipped", "dry-run"].includes(status)) {
        throw new Error(`${source} has unknown exit criterion status ${JSON.stringify(status)}`)
      }
    }
  }
  if (key === "evidenceRepos") {
    for (const repo of Object.keys(value)) {
      validateDrillArtifactEvidenceRepo(repo, source)
    }
  }
  if (key === "providerAccountAliases") {
    for (const accountAlias of Object.keys(value)) {
      validateProviderAccountAliasEntry(accountAlias, source)
    }
  }
  if (key === "validationPresets") {
    for (const preset of Object.keys(value)) {
      if (!isKnownDrillArtifactValidationPreset(preset)) {
        throw new Error(`${source} has unknown validation preset ${JSON.stringify(preset)}`)
      }
    }
  }
}

function validateRuntimeSignalOwnerCountsMatch(
  runtimeSignals,
  runtimeSignalOwners,
  source,
  ownerKey = "runtimeSignalOwners",
  signalKey = "runtimeSignals",
) {
  const expectedOwners = Object.fromEntries(
    drillRuntimeSignalOwnersFor(Object.keys(runtimeSignals ?? {})).map((owner) => [owner, 1]),
  )
  if (JSON.stringify(runtimeSignalOwners ?? {}) !== JSON.stringify(expectedOwners)) {
    throw new Error(`${source}.${ownerKey} must match ${signalKey}`)
  }
}

function validateRuntimeSignalOwnerKeysMatch(
  runtimeSignals,
  runtimeSignalOwners,
  source,
  ownerKey = "runtimeSignalOwners",
  signalKey = "runtimeSignals",
) {
  validateDiagnosticCountObject(runtimeSignals ?? {}, `${source}.${signalKey}`, signalKey)
  validateCountObject(runtimeSignalOwners ?? {}, `${source}.${ownerKey}`)
  const actualOwners = Object.keys(runtimeSignalOwners ?? {}).sort()
  const expectedOwners = drillRuntimeSignalOwnersFor(Object.keys(runtimeSignals ?? {})).sort()
  if (JSON.stringify(actualOwners) !== JSON.stringify(expectedOwners)) {
    throw new Error(`${source}.${ownerKey} must match ${signalKey}`)
  }
}

function relativeArtifactPath(rootDir, artifactPath) {
  if (!nonEmptyString(artifactPath)) throw new Error("drill artifact path is required")
  const absoluteRoot = resolvedDrillRootDir(rootDir)
  const absoluteArtifact = path.resolve(absoluteRoot, artifactPath)
  const relativePath = path.relative(absoluteRoot, absoluteArtifact)
  if (!safeRelativePath(relativePath)) {
    throw new Error(`drill artifact path escapes root: ${JSON.stringify(artifactPath)}`)
  }
  return relativePath
}

function safeRelativePath(value) {
  return typeof value === "string"
    && value.length > 0
    && !path.isAbsolute(value)
    && !value.split(/[\\/]/).includes("..")
}

function schemaForArtifact(contents) {
  try {
    const parsed = JSON.parse(contents.toString("utf8"))
    return nonEmptyString(parsed?.schema) ? parsed.schema : null
  } catch {
    return null
  }
}

function validateKnownArtifactContents(contents, artifactPath, metadata = {}) {
  let parsed
  try {
    parsed = JSON.parse(contents.toString("utf8"))
  } catch {
    return
  }
  const requiresRuntimeSignalManifest = runtimeSignalsFromMetadata(metadata).length > 0
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
    if (parsed.runtimeSignalsManifest !== undefined) {
      validateDrillRuntimeSignalsManifest(parsed.runtimeSignalsManifest, `${artifactPath}.runtimeSignalsManifest`)
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
    if (parsed.manifest?.runtimeSignalsManifest !== undefined) {
      validateDrillRuntimeSignalsManifest(parsed.manifest.runtimeSignalsManifest, `${artifactPath}.manifest.runtimeSignalsManifest`)
    }
  }
  if (parsed?.schema === "arroba.drill.matrix.v1") {
    validateDrillMatrixReport(parsed, artifactPath)
    validateMatrixArtifactMetadata(parsed, artifactPath, metadata)
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
  if (!["passed", "failed"].includes(run.status)) {
    throw new Error(`drill artifact ${source} has invalid status`)
  }
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

function sha256(value) {
  return createHash("sha256").update(value).digest("hex")
}

function sortedCountObject(counts) {
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

function runtimeSignalsFromMetadata(metadata) {
  return metadataListFromMetadata(metadata, "runtimeSignals")
}

function runtimeSignalOwnersFromRuntimeSignals(runtimeSignals) {
  return drillRuntimeSignalOwnersFor(runtimeSignals)
}

function validateDrillArtifactIndexRuntimeSignalOwnerMetadata(metadata, source) {
  const runtimeSignals = runtimeSignalsFromMetadata(metadata)
  const runtimeSignalOwners = metadataListFromMetadata(metadata, "runtimeSignalOwners")
  const requiredRuntimeSignals = metadataListFromMetadata(metadata, "requiredRuntimeSignals")
  const requiredRuntimeSignalOwners = metadataListFromMetadata(metadata, "requiredRuntimeSignalOwners")
  const missingRuntimeSignals = metadataListFromMetadata(metadata, "missingRuntimeSignals")
  const missingRuntimeSignalOwners = metadataListFromMetadata(metadata, "missingRuntimeSignalOwners")
  validateDrillRuntimeSignals(requiredRuntimeSignals, `${source}.requiredRuntimeSignals`)
  validateDrillRuntimeSignals(missingRuntimeSignals, `${source}.missingRuntimeSignals`)
  if (runtimeSignals.length > 0 || runtimeSignalOwners.length > 0) {
    const expectedRuntimeSignalOwners = runtimeSignalOwnersFromRuntimeSignals(runtimeSignals)
    if (runtimeSignals.length === 0) {
      throw new Error(`${source}.runtimeSignalOwners requires runtimeSignals`)
    }
    if (JSON.stringify(runtimeSignalOwners) !== JSON.stringify(expectedRuntimeSignalOwners)) {
      throw new Error(`${source}.runtimeSignalOwners must match runtimeSignals`)
    }
  }
  validateOptionalRuntimeSignalOwners(
    requiredRuntimeSignals,
    requiredRuntimeSignalOwners,
    `${source}.requiredRuntimeSignalOwners`,
    "requiredRuntimeSignals",
  )
  validateOptionalRuntimeSignalOwners(
    missingRuntimeSignals,
    missingRuntimeSignalOwners,
    `${source}.missingRuntimeSignalOwners`,
    "missingRuntimeSignals",
  )
}

function validateOptionalRuntimeSignalOwners(runtimeSignals, runtimeSignalOwners, source, signalKey) {
  if (runtimeSignals.length === 0 && runtimeSignalOwners.length === 0) return
  if (runtimeSignals.length === 0) {
    throw new Error(`${source} requires ${signalKey}`)
  }
  const expectedRuntimeSignalOwners = runtimeSignalOwnersFromRuntimeSignals(runtimeSignals)
  if (JSON.stringify(runtimeSignalOwners) !== JSON.stringify(expectedRuntimeSignalOwners)) {
    throw new Error(`${source} must match ${signalKey}`)
  }
}

function validateDrillArtifactIndexEvidenceRepoMetadata(metadata, source) {
  for (const repo of metadataListFromMetadata(metadata, "evidenceRepos")) {
    validateDrillArtifactEvidenceRepo(repo, `${source}.evidenceRepos`)
  }
}

function validateDrillArtifactIndexProviderAccountAliasMetadata(metadata, source) {
  for (const accountAlias of metadataListFromMetadata(metadata, "providerAccountAliases")) {
    validateProviderAccountAliasEntry(accountAlias, `${source}.providerAccountAliases`)
  }
}

function validateProviderAccountAliasEntry(accountAlias, source) {
  const { provider } = parseProviderAccountAlias(accountAlias)
  if (!isKnownDrillProvider(provider)) {
    throw new Error(`${source} has unknown provider account alias provider ${JSON.stringify(provider)}`)
  }
}

function validateDrillArtifactIndexGeneratedEvidenceMetadata(metadata, source) {
  for (const key of [
    "generatedEvidenceKinds",
    "requiredGeneratedEvidenceKinds",
    "missingGeneratedEvidenceKinds",
  ]) {
    for (const kind of metadataListFromMetadata(metadata, key)) {
      validateDrillGeneratedEvidenceKind(kind, `${source}.${key}`)
    }
  }
  for (const limitation of metadataListFromMetadata(metadata, "generatedMatrixLimitations")) {
    validateDrillGeneratedMatrixLimitation(limitation, `${source}.generatedMatrixLimitations`)
  }
  for (const key of ["requiredGeneratedMatrixLimitations", "missingGeneratedMatrixLimitations"]) {
    for (const limitation of metadataListFromMetadata(metadata, key)) {
      validateDrillGeneratedMatrixLimitation(limitation, `${source}.${key}`)
    }
  }
  for (const key of DRILL_GENERATED_EVIDENCE_PATH_METADATA_KEYS) {
    for (const [index, value] of metadataListFromMetadata(metadata, key).entries()) {
      validateGeneratedEvidencePathText(value, `${source}.${key}[${index}]`)
    }
  }
  for (const key of ["generatedMatrixRepos", "requiredGeneratedMatrixRepos", "missingGeneratedMatrixRepos"]) {
    for (const repo of metadataListFromMetadata(metadata, key)) {
      validateDrillArtifactEvidenceRepo(repo, `${source}.${key}`)
    }
  }
  for (const key of ["generatedMatrixNames", "requiredGeneratedMatrixNames", "missingGeneratedMatrixNames"]) {
    for (const [index, matrixName] of metadataListFromMetadata(metadata, key).entries()) {
      validateDrillGeneratedMatrixName(matrixName, {
        secretSource: `${source}.${key}[${index}]`,
        unknownSource: `${source}.${key}`,
      })
    }
  }
  validateGeneratedMatrixNameRepoMetadata(metadata, source)
}

function validateGeneratedMatrixNameRepoMetadata(metadata, source) {
  const matrixNames = metadataListFromMetadata(metadata, "generatedMatrixNames")
  const matrixRepos = new Set(metadataListFromMetadata(metadata, "generatedMatrixRepos"))
  validateDrillGeneratedMatrixNameRepoMetadata(matrixNames, matrixRepos, `${source}.generatedMatrixNames`)
}

function validateGeneratedEvidencePathText(value, source) {
  validateDrillGeneratedEvidencePath(value, source)
}

function validateDrillArtifactIndexKindMetadata(metadata, source) {
  for (const kind of metadataListFromMetadata(metadata, "artifactKinds")) {
    validateDrillArtifactKind(kind, `${source}.artifactKinds`)
  }
}

function metadataListFromMetadata(metadata, key) {
  const value = metadata?.[key]
  if (typeof value !== "string") return []
  return [...new Set(value.split(",").map((item) => item.trim()).filter(nonEmptyString))].sort()
}

function countValues(values, target = new Map()) {
  for (const value of values) {
    target.set(value, (target.get(value) ?? 0) + 1)
  }
  return sortedCountObject(target)
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}
