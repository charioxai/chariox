import { createHash } from "node:crypto"
import { mkdir, readFile, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import { validateDrillAggregateNextAction } from "./drill-aggregate-actions.mjs"
import { validateKnownArtifactContents } from "./drill-artifact-content-validation.mjs"
import { validateDrillArtifactKind } from "./drill-artifact-kinds.mjs"
import {
  DRILL_GENERATED_EVIDENCE_PATH_METADATA_KEYS,
  metadataListFromMetadata,
  runtimeSignalOwnersFromRuntimeSignals,
  runtimeSignalsFromMetadata,
  validateDrillArtifactIndexEvidenceRepoMetadata,
  validateDrillArtifactIndexGeneratedEvidenceMetadata,
  validateDrillArtifactIndexKindMetadata,
  validateDrillArtifactIndexProviderAccountAliasMetadata,
  validateDrillArtifactIndexRuntimeSignalOwnerMetadata,
  validateGeneratedEvidencePathText,
  validateProviderAccountAliasEntry,
} from "./drill-artifact-metadata-validation.mjs"
import { validateDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import { validateDrillExitCriterionStatus } from "./drill-exit-criterion-statuses.mjs"
import { validateDrillFailureClassification } from "./drill-failure-taxonomy.mjs"
import { validateDrillGeneratedEvidenceKind } from "./drill-generated-evidence-metadata.mjs"
import {
  validateDrillGeneratedMatrixName,
  validateDrillGeneratedMatrixNameRepoCounts,
} from "./drill-generated-matrix-metadata.mjs"
import { validateDrillGeneratedMatrixLimitation } from "./drill-generated-matrix-limitations.mjs"
import { findDrillJsonArtifactPaths } from "./drill-json-discovery.mjs"
import {
  redactDrillSecretText,
  sanitizeDrillMetadata,
} from "./drill-secrets.mjs"
import { validateDrillArtifactValidationPreset } from "./drill-validation-gate-presets.mjs"
import { parseDrillIsoTimestamp } from "./drill-time.mjs"
import {
  drillRuntimeSignalOwnersFor,
  validateDrillRuntimeSignal,
} from "./drill-runtime-signals.mjs"
import { validateDrillRuntimeAuthorityInvariant } from "./drill-runtime-authority-invariants.mjs"
import { normalizeCloudRuntimeAuthorityInvariantId } from "./drill-runtime-authority-registry-parity.mjs"

export const DRILL_ARTIFACT_INDEX_SCHEMA = "arroba.drill.artifact_index.v1"
export const DRILL_ARTIFACT_INDEX_AGGREGATE_SCHEMA = "arroba.drill.artifact_index.aggregate.v1"
export const DRILL_ARTIFACT_DIAGNOSTIC_METADATA_KEYS = Object.freeze([
  "runtimeSignals",
  "runtimeSignalOwners",
  "requiredRuntimeSignals",
  "requiredRuntimeSignalOwners",
  "missingRuntimeSignals",
  "missingRuntimeSignalOwners",
  "runtimeAuthorityInvariants",
  "requiredRuntimeAuthorityInvariants",
  "missingRuntimeAuthorityInvariants",
  "coverageAreas",
  "validationPresets",
  "owners",
  "classifications",
  "requiredFailureClassifications",
  "missingFailureClassifications",
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
  "generatedEvidenceRepos",
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
  "requiredGeneratedValidationSuiteFailureRoots",
  "missingGeneratedValidationSuiteFailureRoots",
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
  runtimeAuthorityInvariants: "runtime_authority_invariants",
  requiredRuntimeAuthorityInvariants: "required_runtime_authority_invariants",
  missingRuntimeAuthorityInvariants: "missing_runtime_authority_invariants",
  coverageAreas: "coverage_areas",
  validationPresets: "validation_presets",
  owners: "owners",
  classifications: "classifications",
  requiredFailureClassifications: "required_failure_classifications",
  missingFailureClassifications: "missing_failure_classifications",
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
  generatedEvidenceRepos: "generated_evidence_repos",
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
  requiredGeneratedValidationSuiteFailureRoots: "required_generated_validation_suite_failure_roots",
  missingGeneratedValidationSuiteFailureRoots: "missing_generated_validation_suite_failure_roots",
  providerAccountAliases: "provider_account_aliases",
  evidenceRepos: "evidence_repos",
  artifactCoverageInputSources: "artifact_coverage_input_sources",
})

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
  preserveOnSuccess = false,
  failure = null,
  metadata = {},
}) {
  const resolvedRootDir = resolvedDrillRootDir(rootDir)
  if (passed && preserveOnSuccess) {
    if (log) {
      log("preserved-successful-run", { rootDir: resolvedRootDir })
    }
    return { preserved: true, rootDir: resolvedRootDir }
  }

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
  const runtimeAuthorityInvariants = new Map()
  const requiredRuntimeAuthorityInvariants = new Map()
  const missingRuntimeAuthorityInvariants = new Map()
  const coverageAreas = new Map()
  const validationPresets = new Map()
  const owners = new Map()
  const classifications = new Map()
  const requiredFailureClassifications = new Map()
  const missingFailureClassifications = new Map()
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
  const generatedEvidenceRepos = new Map()
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
  const requiredGeneratedValidationSuiteFailureRoots = new Map()
  const missingGeneratedValidationSuiteFailureRoots = new Map()
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
    const indexRuntimeAuthorityInvariants = runtimeAuthorityInvariantsForEvidence(index.metadata, "runtimeAuthorityInvariants")
    const indexRequiredRuntimeAuthorityInvariants = runtimeAuthorityInvariantsForEvidence(index.metadata, "requiredRuntimeAuthorityInvariants")
    const indexMissingRuntimeAuthorityInvariants = runtimeAuthorityInvariantsForEvidence(index.metadata, "missingRuntimeAuthorityInvariants")
    const indexCoverageAreas = metadataListFromMetadata(index.metadata, "coverageAreas")
    const indexValidationPresets = metadataListFromMetadata(index.metadata, "validationPresets")
    const indexOwners = metadataListFromMetadata(index.metadata, "owners")
    const indexClassifications = metadataListFromMetadata(index.metadata, "classifications")
    const indexRequiredFailureClassifications = metadataListFromMetadata(index.metadata, "requiredFailureClassifications")
    const indexMissingFailureClassifications = metadataListFromMetadata(index.metadata, "missingFailureClassifications")
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
    const indexGeneratedEvidenceRepos = metadataListFromMetadata(index.metadata, "generatedEvidenceRepos")
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
    const indexRequiredGeneratedValidationSuiteFailureRoots = metadataListFromMetadata(index.metadata, "requiredGeneratedValidationSuiteFailureRoots")
    const indexMissingGeneratedValidationSuiteFailureRoots = metadataListFromMetadata(index.metadata, "missingGeneratedValidationSuiteFailureRoots")
    const indexProviderAccountAliases = metadataListFromMetadata(index.metadata, "providerAccountAliases")
    const indexEvidenceRepos = metadataListFromMetadata(index.metadata, "evidenceRepos")
    const indexArtifactCoverageInputSources = metadataListFromMetadata(index.metadata, "artifactCoverageInputSources")
    countValues(indexRuntimeSignals, runtimeSignals)
    countValues(indexRuntimeSignalOwners, runtimeSignalOwners)
    countValues(indexRequiredRuntimeSignals, requiredRuntimeSignals)
    countValues(indexRequiredRuntimeSignalOwners, requiredRuntimeSignalOwners)
    countValues(indexMissingRuntimeSignals, missingRuntimeSignals)
    countValues(indexMissingRuntimeSignalOwners, missingRuntimeSignalOwners)
    countValues(indexRuntimeAuthorityInvariants, runtimeAuthorityInvariants)
    countValues(indexRequiredRuntimeAuthorityInvariants, requiredRuntimeAuthorityInvariants)
    countValues(indexMissingRuntimeAuthorityInvariants, missingRuntimeAuthorityInvariants)
    countValues(indexCoverageAreas, coverageAreas)
    countValues(indexValidationPresets, validationPresets)
    countValues(indexOwners, owners)
    countValues(indexClassifications, classifications)
    countValues(indexRequiredFailureClassifications, requiredFailureClassifications)
    countValues(indexMissingFailureClassifications, missingFailureClassifications)
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
    countValues(indexGeneratedEvidenceRepos, generatedEvidenceRepos)
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
    countValues(indexRequiredGeneratedValidationSuiteFailureRoots, requiredGeneratedValidationSuiteFailureRoots)
    countValues(indexMissingGeneratedValidationSuiteFailureRoots, missingGeneratedValidationSuiteFailureRoots)
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
      runtimeAuthorityInvariants: countValues(indexRuntimeAuthorityInvariants),
      requiredRuntimeAuthorityInvariants: countValues(indexRequiredRuntimeAuthorityInvariants),
      missingRuntimeAuthorityInvariants: countValues(indexMissingRuntimeAuthorityInvariants),
      coverageAreas: countValues(indexCoverageAreas),
      validationPresets: countValues(indexValidationPresets),
      owners: countValues(indexOwners),
      classifications: countValues(indexClassifications),
      requiredFailureClassifications: countValues(indexRequiredFailureClassifications),
      missingFailureClassifications: countValues(indexMissingFailureClassifications),
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
      generatedEvidenceRepos: countValues(indexGeneratedEvidenceRepos),
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
      requiredGeneratedValidationSuiteFailureRoots: countValues(indexRequiredGeneratedValidationSuiteFailureRoots),
      missingGeneratedValidationSuiteFailureRoots: countValues(indexMissingGeneratedValidationSuiteFailureRoots),
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
    runtimeAuthorityInvariants: sortedCountObject(runtimeAuthorityInvariants),
    requiredRuntimeAuthorityInvariants: sortedCountObject(requiredRuntimeAuthorityInvariants),
    missingRuntimeAuthorityInvariants: sortedCountObject(missingRuntimeAuthorityInvariants),
    coverageAreas: sortedCountObject(coverageAreas),
    validationPresets: sortedCountObject(validationPresets),
    owners: sortedCountObject(owners),
    classifications: sortedCountObject(classifications),
    requiredFailureClassifications: sortedCountObject(requiredFailureClassifications),
    missingFailureClassifications: sortedCountObject(missingFailureClassifications),
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
    generatedEvidenceRepos: sortedCountObject(generatedEvidenceRepos),
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
    requiredGeneratedValidationSuiteFailureRoots: sortedCountObject(requiredGeneratedValidationSuiteFailureRoots),
    missingGeneratedValidationSuiteFailureRoots: sortedCountObject(missingGeneratedValidationSuiteFailureRoots),
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
  if (["generatedMatrixRepos", "generatedEvidenceRepos", "requiredGeneratedMatrixRepos", "missingGeneratedMatrixRepos"].includes(key)) {
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
      validateDrillRuntimeSignal(signal, source)
    }
  }
  if (["runtimeAuthorityInvariants", "requiredRuntimeAuthorityInvariants", "missingRuntimeAuthorityInvariants"].includes(key)) {
    for (const invariant of Object.keys(value)) {
      validateDrillRuntimeAuthorityInvariant(invariant, source)
    }
  }
  if (key === "exitCriterionStatuses" || key === "incompleteExitCriterionStatuses") {
    for (const status of Object.keys(value)) {
      validateDrillExitCriterionStatus(status, source)
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
      validateDrillArtifactValidationPreset(preset, source)
    }
  }
  if (["requiredFailureClassifications", "missingFailureClassifications"].includes(key)) {
    for (const classification of Object.keys(value)) {
      validateDrillFailureClassification(classification, source, {
        label: "failure classification",
      })
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

function runtimeAuthorityInvariantsForEvidence(metadata, key) {
  const invariants = metadataListFromMetadata(metadata, key)
  const evidenceRepos = metadataListFromMetadata(metadata, "evidenceRepos")
  if (evidenceRepos.length !== 1 || evidenceRepos[0] !== "cloud") return invariants
  return invariants.map(normalizeCloudRuntimeAuthorityInvariantId)
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

function sha256(value) {
  return createHash("sha256").update(value).digest("hex")
}

function sortedCountObject(counts) {
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
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
