import { validateDrillArtifactKind } from "./drill-artifact-kinds.mjs"
import { validateDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import {
  validateDrillGeneratedEvidenceKind,
  validateDrillGeneratedEvidencePath,
} from "./drill-generated-evidence-metadata.mjs"
import {
  validateDrillGeneratedMatrixName,
  validateDrillGeneratedMatrixNameRepoMetadata,
} from "./drill-generated-matrix-metadata.mjs"
import { validateDrillGeneratedMatrixLimitation } from "./drill-generated-matrix-limitations.mjs"
import {
  parseProviderAccountAlias,
  validateDrillProvider,
} from "./drill-provider-profiles.mjs"
import {
  drillRuntimeSignalOwnersFor,
  validateDrillRuntimeSignals,
} from "./drill-runtime-signals.mjs"

export const DRILL_GENERATED_EVIDENCE_PATH_METADATA_KEYS = Object.freeze([
  "generatedMatrixArtifactIndexes",
  "generatedValidationSuiteArtifactIndexes",
  "generatedValidationSuiteFailureRoots",
  "requiredGeneratedMatrixArtifactIndexes",
  "missingGeneratedMatrixArtifactIndexes",
  "requiredGeneratedValidationSuiteArtifactIndexes",
  "missingGeneratedValidationSuiteArtifactIndexes",
  "requiredGeneratedValidationSuiteFailureRoots",
  "missingGeneratedValidationSuiteFailureRoots",
])

export function runtimeSignalsFromMetadata(metadata) {
  return metadataListFromMetadata(metadata, "runtimeSignals")
}

export function metadataHasAnyList(metadata, keys) {
  return keys.some((key) => metadataListFromMetadata(metadata, key).length > 0)
}

export function runtimeSignalOwnersFromRuntimeSignals(runtimeSignals) {
  return drillRuntimeSignalOwnersFor(runtimeSignals)
}

export function validateDrillArtifactIndexRuntimeSignalOwnerMetadata(metadata, source) {
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

export function validateDrillArtifactIndexEvidenceRepoMetadata(metadata, source) {
  for (const key of ["evidenceRepos", "generatedEvidenceRepos"]) {
    for (const repo of metadataListFromMetadata(metadata, key)) {
      validateDrillArtifactEvidenceRepo(repo, `${source}.${key}`)
    }
  }
}

export function validateDrillArtifactIndexProviderAccountAliasMetadata(metadata, source) {
  for (const accountAlias of metadataListFromMetadata(metadata, "providerAccountAliases")) {
    validateProviderAccountAliasEntry(accountAlias, `${source}.providerAccountAliases`)
  }
}

export function validateProviderAccountAliasEntry(accountAlias, source) {
  const { provider } = parseProviderAccountAlias(accountAlias)
  validateDrillProvider(provider, source, {
    label: "provider account alias provider",
  })
}

export function validateDrillArtifactIndexGeneratedEvidenceMetadata(metadata, source) {
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

export function validateGeneratedEvidencePathText(value, source) {
  validateDrillGeneratedEvidencePath(value, source)
}

export function validateDrillArtifactIndexKindMetadata(metadata, source) {
  for (const kind of metadataListFromMetadata(metadata, "artifactKinds")) {
    validateDrillArtifactKind(kind, `${source}.artifactKinds`)
  }
}

export function metadataListFromMetadata(metadata, key) {
  const value = metadata?.[key]
  if (typeof value !== "string") return []
  return [...new Set(value.split(",").map((item) => item.trim()).filter(nonEmptyString))].sort()
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}
