import { validateDrillGeneratedMatrixCommandMetadata } from "./drill-generated-matrix-command-metadata.mjs"
import { validateDrillGeneratedMatrixLimitation } from "./drill-generated-matrix-limitations.mjs"
import { validateDrillValidationCheckStatus, validateDrillValidationResultStatus } from "./drill-validation-statuses.mjs"
import {
  appendMatrixRuntimeSignalSources,
  countObjectValues,
  countStringValues,
  formatMatrixRuntimeSignalSources,
  formatValidationGateCoverageCounts,
  staleFailureManifestSourceLabels,
  staleMatrixReportSourceLabels,
} from "./drill-validation-gate-aggregate-coverage.mjs"
import {
  nonEmptyString,
  validateArtifactEvidenceRepoArray,
  validateArtifactEvidenceRepoCountObject,
  validateArtifactKindArray,
  validateArtifactKindCountObject,
  validateArtifactValidationPresetArray,
  validateArtifactValidationPresetCountObject,
  validateCountObject,
  validateDeploymentPresetArray,
  validateDeploymentPresetCountObject,
  validateExitCriterionStatusArray,
  validateExitCriterionStatusCountObject,
  validateFailureClassificationArray,
  validateFailureClassificationCountObject,
  validateGeneratedEvidenceKindArray,
  validateGeneratedEvidenceKindCountObject,
  validateGeneratedEvidencePathArray,
  validateGeneratedEvidencePathCountObject,
  validateGeneratedEvidencePathText,
  validateGeneratedMatrixLimitationArray,
  validateGeneratedMatrixLimitationCountObject,
  validateMatrixRuntimeSignalSources,
  validatePresetArray,
  validatePresetCountObject,
  validateProviderAccountAliasArray,
  validateProviderAccountAliasCountObject,
  validateProviderArray,
  validateProviderCountObject,
  validateRuntimeAuthorityInvariantArray,
  validateRuntimeAuthorityInvariantCountObject,
  validateRuntimeSignalArray,
  validateRuntimeSignalCountObject,
  validateRuntimeSignalOwnerArray,
  validateRuntimeSignalOwnerCountObject,
  validateRuntimeSignalOwnerCountsMatch,
  validateRuntimeSignalScenarioMap,
  validateStringArray,
} from "./drill-validation-gate-aggregate-primitives.mjs"

export function validateValidationGateCoverageAggregate(coverage, source) {
  if (!coverage || typeof coverage !== "object" || Array.isArray(coverage)) {
    throw new Error(`${source} is not an object`)
  }
  validatePresetCountObject(coverage.presets ?? {}, `${source}.presets`)
  validateCountObject(coverage.requiredPlatformCoverageAreas ?? {}, `${source}.requiredPlatformCoverageAreas`)
  validateCountObject(coverage.missingPlatformCoverageAreas ?? {}, `${source}.missingPlatformCoverageAreas`)
  validateCountObject(coverage.requiredArtifactCoverageAreas ?? {}, `${source}.requiredArtifactCoverageAreas`)
  validateCountObject(coverage.missingArtifactCoverageAreas ?? {}, `${source}.missingArtifactCoverageAreas`)
  validateCountObject(coverage.requiredArtifactSchemas ?? {}, `${source}.requiredArtifactSchemas`)
  validateCountObject(coverage.missingArtifactSchemas ?? {}, `${source}.missingArtifactSchemas`)
  validateArtifactKindCountObject(coverage.requiredArtifactKinds ?? {}, `${source}.requiredArtifactKinds`)
  validateArtifactKindCountObject(coverage.missingArtifactKinds ?? {}, `${source}.missingArtifactKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.requiredArtifactGeneratedEvidenceKinds ?? {}, `${source}.requiredArtifactGeneratedEvidenceKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.missingArtifactGeneratedEvidenceKinds ?? {}, `${source}.missingArtifactGeneratedEvidenceKinds`)
  validateArtifactEvidenceRepoCountObject(coverage.requiredArtifactGeneratedEvidenceRepos ?? {}, `${source}.requiredArtifactGeneratedEvidenceRepos`)
  validateArtifactEvidenceRepoCountObject(coverage.missingArtifactGeneratedEvidenceRepos ?? {}, `${source}.missingArtifactGeneratedEvidenceRepos`)
  validateGeneratedEvidencePathCountObject(coverage.requiredArtifactGeneratedMatrixArtifactIndexes ?? {}, `${source}.requiredArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedEvidencePathCountObject(coverage.missingArtifactGeneratedMatrixArtifactIndexes ?? {}, `${source}.missingArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationCountObject(coverage.requiredArtifactGeneratedMatrixLimitations ?? {}, `${source}.requiredArtifactGeneratedMatrixLimitations`)
  validateGeneratedMatrixLimitationCountObject(coverage.missingArtifactGeneratedMatrixLimitations ?? {}, `${source}.missingArtifactGeneratedMatrixLimitations`)
  validateCountObject(coverage.requiredArtifactGeneratedMatrixNames ?? {}, `${source}.requiredArtifactGeneratedMatrixNames`)
  validateCountObject(coverage.missingArtifactGeneratedMatrixNames ?? {}, `${source}.missingArtifactGeneratedMatrixNames`)
  validateArtifactEvidenceRepoCountObject(coverage.requiredArtifactGeneratedMatrixRepos ?? {}, `${source}.requiredArtifactGeneratedMatrixRepos`)
  validateArtifactEvidenceRepoCountObject(coverage.missingArtifactGeneratedMatrixRepos ?? {}, `${source}.missingArtifactGeneratedMatrixRepos`)
  validateGeneratedEvidencePathCountObject(coverage.requiredArtifactGeneratedValidationSuiteArtifactIndexes ?? {}, `${source}.requiredArtifactGeneratedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathCountObject(coverage.missingArtifactGeneratedValidationSuiteArtifactIndexes ?? {}, `${source}.missingArtifactGeneratedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathCountObject(coverage.requiredArtifactGeneratedValidationSuiteFailureRoots ?? {}, `${source}.requiredArtifactGeneratedValidationSuiteFailureRoots`)
  validateGeneratedEvidencePathCountObject(coverage.missingArtifactGeneratedValidationSuiteFailureRoots ?? {}, `${source}.missingArtifactGeneratedValidationSuiteFailureRoots`)
  validateArtifactEvidenceRepoCountObject(coverage.requiredArtifactEvidenceRepos ?? {}, `${source}.requiredArtifactEvidenceRepos`)
  validateArtifactEvidenceRepoCountObject(coverage.missingArtifactEvidenceRepos ?? {}, `${source}.missingArtifactEvidenceRepos`)
  validateProviderAccountAliasCountObject(coverage.requiredArtifactProviderAccountAliases ?? {}, `${source}.requiredArtifactProviderAccountAliases`)
  validateProviderAccountAliasCountObject(coverage.missingArtifactProviderAccountAliases ?? {}, `${source}.missingArtifactProviderAccountAliases`)
  validateArtifactValidationPresetCountObject(coverage.requiredArtifactValidationPresets ?? {}, `${source}.requiredArtifactValidationPresets`)
  validateArtifactValidationPresetCountObject(coverage.missingArtifactValidationPresets ?? {}, `${source}.missingArtifactValidationPresets`)
  validateRuntimeAuthorityInvariantCountObject(coverage.requiredArtifactRuntimeAuthorityInvariants ?? {}, `${source}.requiredArtifactRuntimeAuthorityInvariants`)
  validateRuntimeAuthorityInvariantCountObject(coverage.missingArtifactRuntimeAuthorityInvariants ?? {}, `${source}.missingArtifactRuntimeAuthorityInvariants`)
  validateRuntimeSignalCountObject(coverage.requiredArtifactRuntimeSignals ?? {}, `${source}.requiredArtifactRuntimeSignals`)
  validateRuntimeSignalCountObject(coverage.missingArtifactRuntimeSignals ?? {}, `${source}.missingArtifactRuntimeSignals`)
  validateRuntimeSignalOwnerCountObject(coverage.requiredArtifactRuntimeSignalOwners ?? {}, `${source}.requiredArtifactRuntimeSignalOwners`)
  validateRuntimeSignalOwnerCountObject(coverage.missingArtifactRuntimeSignalOwners ?? {}, `${source}.missingArtifactRuntimeSignalOwners`)
  validateCountObject(coverage.requiredArtifactOwners ?? {}, `${source}.requiredArtifactOwners`)
  validateCountObject(coverage.missingArtifactOwners ?? {}, `${source}.missingArtifactOwners`)
  validateCountObject(coverage.requiredArtifactClassifications ?? {}, `${source}.requiredArtifactClassifications`)
  validateCountObject(coverage.missingArtifactClassifications ?? {}, `${source}.missingArtifactClassifications`)
  validateFailureClassificationCountObject(coverage.requiredArtifactFailureClassifications ?? {}, `${source}.requiredArtifactFailureClassifications`)
  validateFailureClassificationCountObject(coverage.missingArtifactFailureClassifications ?? {}, `${source}.missingArtifactFailureClassifications`)
  validateCountObject(coverage.requiredArtifactPlannedOwners ?? {}, `${source}.requiredArtifactPlannedOwners`)
  validateCountObject(coverage.missingArtifactPlannedOwners ?? {}, `${source}.missingArtifactPlannedOwners`)
  validateCountObject(coverage.requiredArtifactPlannedClassifications ?? {}, `${source}.requiredArtifactPlannedClassifications`)
  validateCountObject(coverage.missingArtifactPlannedClassifications ?? {}, `${source}.missingArtifactPlannedClassifications`)
  validateExitCriterionStatusCountObject(coverage.requiredArtifactExitCriterionStatuses ?? {}, `${source}.requiredArtifactExitCriterionStatuses`)
  validateExitCriterionStatusCountObject(coverage.missingArtifactExitCriterionStatuses ?? {}, `${source}.missingArtifactExitCriterionStatuses`)
  validateExitCriterionStatusCountObject(coverage.requiredArtifactIncompleteExitCriterionStatuses ?? {}, `${source}.requiredArtifactIncompleteExitCriterionStatuses`)
  validateExitCriterionStatusCountObject(coverage.missingArtifactIncompleteExitCriterionStatuses ?? {}, `${source}.missingArtifactIncompleteExitCriterionStatuses`)
  validateCountObject(coverage.artifactSchemas ?? {}, `${source}.artifactSchemas`)
  validateCountObject(coverage.artifactCoverageAreas ?? {}, `${source}.artifactCoverageAreas`)
  validateRuntimeAuthorityInvariantCountObject(coverage.artifactRuntimeAuthorityInvariants ?? {}, `${source}.artifactRuntimeAuthorityInvariants`)
  validateRuntimeSignalCountObject(coverage.requiredRuntimeSignals ?? {}, `${source}.requiredRuntimeSignals`)
  validateRuntimeSignalCountObject(coverage.missingRuntimeSignals ?? {}, `${source}.missingRuntimeSignals`)
  validateRuntimeSignalOwnerCountObject(coverage.requiredRuntimeSignalOwners ?? {}, `${source}.requiredRuntimeSignalOwners`)
  validateRuntimeSignalOwnerCountObject(coverage.missingRuntimeSignalOwners ?? {}, `${source}.missingRuntimeSignalOwners`)
  validateFailureClassificationCountObject(coverage.requiredFailureClassifications ?? {}, `${source}.requiredFailureClassifications`)
  validateFailureClassificationCountObject(coverage.missingFailureClassifications ?? {}, `${source}.missingFailureClassifications`)
  validateRuntimeSignalCountObject(coverage.artifactRuntimeSignals ?? {}, `${source}.artifactRuntimeSignals`)
  validateRuntimeSignalOwnerCountObject(coverage.artifactRuntimeSignalOwners ?? {}, `${source}.artifactRuntimeSignalOwners`)
  validateCountObject(coverage.artifactOwners ?? {}, `${source}.artifactOwners`)
  validateCountObject(coverage.artifactClassifications ?? {}, `${source}.artifactClassifications`)
  validateFailureClassificationCountObject(coverage.artifactFailureClassifications ?? {}, `${source}.artifactFailureClassifications`)
  validateCountObject(coverage.artifactPlannedOwners ?? {}, `${source}.artifactPlannedOwners`)
  validateCountObject(coverage.artifactPlannedClassifications ?? {}, `${source}.artifactPlannedClassifications`)
  validateExitCriterionStatusCountObject(coverage.artifactExitCriterionStatuses ?? {}, `${source}.artifactExitCriterionStatuses`)
  validateExitCriterionStatusCountObject(coverage.artifactIncompleteExitCriterionStatuses ?? {}, `${source}.artifactIncompleteExitCriterionStatuses`)
  validateArtifactKindCountObject(coverage.artifactKinds ?? {}, `${source}.artifactKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.artifactGeneratedEvidenceKinds ?? {}, `${source}.artifactGeneratedEvidenceKinds`)
  validateArtifactEvidenceRepoCountObject(coverage.artifactGeneratedEvidenceRepos ?? {}, `${source}.artifactGeneratedEvidenceRepos`)
  validateGeneratedEvidencePathCountObject(coverage.artifactGeneratedMatrixArtifactIndexes ?? {}, `${source}.artifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationCountObject(coverage.artifactGeneratedMatrixLimitations ?? {}, `${source}.artifactGeneratedMatrixLimitations`)
  validateCountObject(coverage.artifactGeneratedMatrixNames ?? {}, `${source}.artifactGeneratedMatrixNames`)
  validateArtifactEvidenceRepoCountObject(coverage.artifactGeneratedMatrixRepos ?? {}, `${source}.artifactGeneratedMatrixRepos`)
  validateGeneratedEvidencePathCountObject(coverage.artifactGeneratedValidationSuiteArtifactIndexes ?? {}, `${source}.artifactGeneratedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathCountObject(coverage.artifactGeneratedValidationSuiteFailureRoots ?? {}, `${source}.artifactGeneratedValidationSuiteFailureRoots`)
  validateArtifactEvidenceRepoCountObject(coverage.artifactEvidenceRepos ?? {}, `${source}.artifactEvidenceRepos`)
  validateProviderAccountAliasCountObject(coverage.artifactProviderAccountAliases ?? {}, `${source}.artifactProviderAccountAliases`)
  validateArtifactValidationPresetCountObject(coverage.artifactValidationPresets ?? {}, `${source}.artifactValidationPresets`)
  validateCountObject(coverage.artifactCoverageInputSources ?? {}, `${source}.artifactCoverageInputSources`)
  validateRuntimeSignalCountObject(coverage.failureRuntimeSignals ?? {}, `${source}.failureRuntimeSignals`)
  validateRuntimeSignalOwnerCountsMatch(coverage.failureRuntimeSignals ?? {}, coverage.failureRuntimeSignalOwners ?? {}, `${source}.failureRuntimeSignalOwners`)
  validateCountObject(coverage.failureOwners ?? {}, `${source}.failureOwners`)
  validateFailureClassificationCountObject(coverage.failureClassifications ?? {}, `${source}.failureClassifications`)
  validateCountObject(coverage.failureStaleManifests ?? {}, `${source}.failureStaleManifests`)
  validateRuntimeSignalCountObject(coverage.matrixRuntimeSignals ?? {}, `${source}.matrixRuntimeSignals`)
  validateRuntimeSignalOwnerCountsMatch(coverage.matrixRuntimeSignals ?? {}, coverage.matrixRuntimeSignalOwners ?? {}, `${source}.matrixRuntimeSignalOwners`)
  validateCountObject(coverage.matrixOwners ?? {}, `${source}.matrixOwners`)
  validateFailureClassificationCountObject(coverage.matrixClassifications ?? {}, `${source}.matrixClassifications`)
  validateCountObject(coverage.matrixStaleReports ?? {}, `${source}.matrixStaleReports`)
  validateCountObject(coverage.requiredMatrices ?? {}, `${source}.requiredMatrices`)
  validateCountObject(coverage.missingMatrices ?? {}, `${source}.missingMatrices`)
  validateFailureClassificationCountObject(coverage.requiredMatrixClassifications ?? {}, `${source}.requiredMatrixClassifications`)
  validateFailureClassificationCountObject(coverage.missingMatrixClassifications ?? {}, `${source}.missingMatrixClassifications`)
  validateRuntimeSignalCountObject(coverage.requiredMatrixRuntimeSignals ?? {}, `${source}.requiredMatrixRuntimeSignals`)
  validateRuntimeSignalCountObject(coverage.missingMatrixRuntimeSignals ?? {}, `${source}.missingMatrixRuntimeSignals`)
  validateDeploymentPresetCountObject(coverage.requiredDeploymentPresets ?? {}, `${source}.requiredDeploymentPresets`)
  validateDeploymentPresetCountObject(coverage.missingDeploymentPresets ?? {}, `${source}.missingDeploymentPresets`)
  validateProviderCountObject(coverage.requiredProviders ?? {}, `${source}.requiredProviders`)
  validateProviderCountObject(coverage.missingProviders ?? {}, `${source}.missingProviders`)
  validateCountObject(coverage.requiredScenarios ?? {}, `${source}.requiredScenarios`)
  validateCountObject(coverage.missingScenarios ?? {}, `${source}.missingScenarios`)
  validateGeneratedEvidenceKindCountObject(coverage.generatedEvidenceKinds ?? {}, `${source}.generatedEvidenceKinds`)
  validateGeneratedEvidencePathCountObject(coverage.generatedMatrixArtifactIndexes ?? {}, `${source}.generatedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationCountObject(coverage.generatedMatrixLimitations ?? {}, `${source}.generatedMatrixLimitations`)
  validateGeneratedEvidencePathCountObject(coverage.generatedValidationSuiteArtifactIndexes ?? {}, `${source}.generatedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathCountObject(coverage.generatedValidationSuiteFailureRoots ?? {}, `${source}.generatedValidationSuiteFailureRoots`)
  validateGeneratedEvidenceKindCountObject(coverage.requiredGeneratedEvidenceKinds ?? {}, `${source}.requiredGeneratedEvidenceKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.missingGeneratedEvidenceKinds ?? {}, `${source}.missingGeneratedEvidenceKinds`)
  validateGeneratedEvidencePathCountObject(coverage.requiredGeneratedMatrixArtifactIndexes ?? {}, `${source}.requiredGeneratedMatrixArtifactIndexes`)
  validateGeneratedEvidencePathCountObject(coverage.missingGeneratedMatrixArtifactIndexes ?? {}, `${source}.missingGeneratedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationCountObject(coverage.requiredGeneratedMatrixLimitations ?? {}, `${source}.requiredGeneratedMatrixLimitations`)
  validateGeneratedMatrixLimitationCountObject(coverage.missingGeneratedMatrixLimitations ?? {}, `${source}.missingGeneratedMatrixLimitations`)
  validateGeneratedEvidencePathCountObject(coverage.requiredGeneratedValidationSuiteArtifactIndexes ?? {}, `${source}.requiredGeneratedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathCountObject(coverage.missingGeneratedValidationSuiteArtifactIndexes ?? {}, `${source}.missingGeneratedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathCountObject(coverage.requiredGeneratedValidationSuiteFailureRoots ?? {}, `${source}.requiredGeneratedValidationSuiteFailureRoots`)
  validateGeneratedEvidencePathCountObject(coverage.missingGeneratedValidationSuiteFailureRoots ?? {}, `${source}.missingGeneratedValidationSuiteFailureRoots`)
}

export function validateValidationGateMatrixCoverage(coverage, source) {
  if (!coverage || typeof coverage !== "object" || Array.isArray(coverage)) {
    throw new Error(`${source} is not an object`)
  }
  validateRuntimeSignalCountObject(coverage.runtimeSignals ?? {}, `${source}.runtimeSignals`)
  validateRuntimeSignalOwnerCountsMatch(coverage.runtimeSignals ?? {}, coverage.runtimeSignalOwners ?? {}, `${source}.runtimeSignalOwners`)
  validateCountObject(coverage.owners ?? {}, `${source}.owners`)
  validateCountObject(coverage.classifications ?? {}, `${source}.classifications`)
  validateStaleMatrixReportSummaries(coverage.staleMatrixReports ?? [], `${source}.staleMatrixReports`)
  validateStringArray(coverage.requiredMatrices ?? [], `${source}.requiredMatrices`)
  validateStringArray(coverage.missingMatrices ?? [], `${source}.missingMatrices`)
  validateFailureClassificationArray(coverage.requiredMatrixClassifications ?? [], `${source}.requiredMatrixClassifications`)
  validateFailureClassificationArray(coverage.missingMatrixClassifications ?? [], `${source}.missingMatrixClassifications`)
  validateRuntimeSignalArray(coverage.requiredMatrixRuntimeSignals ?? [], `${source}.requiredMatrixRuntimeSignals`)
  validateRuntimeSignalArray(coverage.missingMatrixRuntimeSignals ?? [], `${source}.missingMatrixRuntimeSignals`)
  validateDeploymentPresetArray(coverage.requiredDeploymentPresets ?? [], `${source}.requiredDeploymentPresets`)
  validateDeploymentPresetArray(coverage.missingDeploymentPresets ?? [], `${source}.missingDeploymentPresets`)
  validateProviderArray(coverage.requiredProviders ?? [], `${source}.requiredProviders`)
  validateProviderArray(coverage.missingProviders ?? [], `${source}.missingProviders`)
  validateStringArray(coverage.requiredScenarios ?? [], `${source}.requiredScenarios`)
  validateStringArray(coverage.missingScenarios ?? [], `${source}.missingScenarios`)
  if (coverage.runtimeSignalScenarios !== undefined) {
    validateRuntimeSignalScenarioMap(coverage.runtimeSignalScenarios, `${source}.runtimeSignalScenarios`, { reportSource: false })
  }
}

export function validateValidationGateFailureCoverage(coverage, source) {
  if (!coverage || typeof coverage !== "object" || Array.isArray(coverage)) {
    throw new Error(`${source} is not an object`)
  }
  validateRuntimeSignalCountObject(coverage.runtimeSignals ?? {}, `${source}.runtimeSignals`)
  validateRuntimeSignalOwnerCountsMatch(coverage.runtimeSignals ?? {}, coverage.runtimeSignalOwners ?? {}, `${source}.runtimeSignalOwners`)
  validateCountObject(coverage.owners ?? {}, `${source}.owners`)
  validateFailureClassificationCountObject(coverage.classifications ?? {}, `${source}.classifications`)
  validateStaleFailureManifestSummaries(coverage.staleFailureManifests ?? [], `${source}.staleFailureManifests`)
}

export function validateStaleMatrixReportSummaries(reports, source) {
  if (!Array.isArray(reports)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, report] of reports.entries()) {
    const entrySource = `${source}[${index}]`
    if (!report || typeof report !== "object" || Array.isArray(report)) {
      throw new Error(`${entrySource} is not an object`)
    }
    if (report.source !== null && typeof report.source !== "string") {
      throw new Error(`${entrySource} has invalid source`)
    }
    if (typeof report.matrix !== "string" || report.matrix.length === 0) {
      throw new Error(`${entrySource} has invalid matrix`)
    }
    if (typeof report.completedAt !== "string" || report.completedAt.length === 0) {
      throw new Error(`${entrySource} has invalid completedAt`)
    }
    if (!Number.isSafeInteger(report.ageMs) || report.ageMs < 0) {
      throw new Error(`${entrySource} has invalid ageMs`)
    }
    if (!Number.isSafeInteger(report.maxAgeMs) || report.maxAgeMs < 0) {
      throw new Error(`${entrySource} has invalid maxAgeMs`)
    }
  }
}

export function validateStaleFailureManifestSummaries(manifests, source) {
  if (!Array.isArray(manifests)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, manifest] of manifests.entries()) {
    const entrySource = `${source}[${index}]`
    if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
      throw new Error(`${entrySource} is not an object`)
    }
    if (manifest.source !== null && typeof manifest.source !== "string") {
      throw new Error(`${entrySource} has invalid source`)
    }
    for (const key of ["drill", "failedAt"]) {
      if (typeof manifest[key] !== "string" || manifest[key].length === 0) {
        throw new Error(`${entrySource} has invalid ${key}`)
      }
    }
    if (!Number.isSafeInteger(manifest.ageMs) || manifest.ageMs < 0) {
      throw new Error(`${entrySource} has invalid ageMs`)
    }
    if (!Number.isSafeInteger(manifest.maxAgeMs) || manifest.maxAgeMs < 0) {
      throw new Error(`${entrySource} has invalid maxAgeMs`)
    }
  }
}

export function validateValidationGatePlatformCoverage(coverage, source) {
  if (!coverage || typeof coverage !== "object" || Array.isArray(coverage)) {
    throw new Error(`${source} is not an object`)
  }
  validateStringArray(coverage.requiredCoverageAreas ?? [], `${source}.requiredCoverageAreas`)
  validateStringArray(coverage.missingCoverageAreas ?? [], `${source}.missingCoverageAreas`)
  validateRuntimeSignalArray(coverage.requiredRuntimeSignals ?? [], `${source}.requiredRuntimeSignals`)
  validateRuntimeSignalArray(coverage.missingRuntimeSignals ?? [], `${source}.missingRuntimeSignals`)
  validateRuntimeSignalOwnerArray(coverage.requiredRuntimeSignalOwners ?? [], `${source}.requiredRuntimeSignalOwners`)
  validateRuntimeSignalOwnerArray(coverage.missingRuntimeSignalOwners ?? [], `${source}.missingRuntimeSignalOwners`)
  validateFailureClassificationArray(coverage.requiredFailureClassifications ?? [], `${source}.requiredFailureClassifications`)
  validateFailureClassificationArray(coverage.missingFailureClassifications ?? [], `${source}.missingFailureClassifications`)
}

export function assertValidationGateCoverageMatchesReports(aggregate, source) {
  const expected = {
    presets: new Map(),
    requiredPlatformCoverageAreas: new Map(),
    missingPlatformCoverageAreas: new Map(),
    requiredArtifactCoverageAreas: new Map(),
    missingArtifactCoverageAreas: new Map(),
    requiredArtifactSchemas: new Map(),
    missingArtifactSchemas: new Map(),
    requiredArtifactKinds: new Map(),
    missingArtifactKinds: new Map(),
    requiredArtifactGeneratedEvidenceKinds: new Map(),
    missingArtifactGeneratedEvidenceKinds: new Map(),
    requiredArtifactGeneratedEvidenceRepos: new Map(),
    missingArtifactGeneratedEvidenceRepos: new Map(),
    requiredArtifactGeneratedMatrixArtifactIndexes: new Map(),
    missingArtifactGeneratedMatrixArtifactIndexes: new Map(),
    requiredArtifactGeneratedMatrixLimitations: new Map(),
    missingArtifactGeneratedMatrixLimitations: new Map(),
    requiredArtifactGeneratedMatrixNames: new Map(),
    missingArtifactGeneratedMatrixNames: new Map(),
    requiredArtifactGeneratedMatrixRepos: new Map(),
    missingArtifactGeneratedMatrixRepos: new Map(),
    requiredArtifactGeneratedValidationSuiteArtifactIndexes: new Map(),
    missingArtifactGeneratedValidationSuiteArtifactIndexes: new Map(),
    requiredArtifactGeneratedValidationSuiteFailureRoots: new Map(),
    missingArtifactGeneratedValidationSuiteFailureRoots: new Map(),
    requiredArtifactEvidenceRepos: new Map(),
    missingArtifactEvidenceRepos: new Map(),
    requiredArtifactProviderAccountAliases: new Map(),
    missingArtifactProviderAccountAliases: new Map(),
    requiredArtifactValidationPresets: new Map(),
    missingArtifactValidationPresets: new Map(),
    requiredArtifactRuntimeAuthorityInvariants: new Map(),
    missingArtifactRuntimeAuthorityInvariants: new Map(),
    requiredArtifactRuntimeSignals: new Map(),
    missingArtifactRuntimeSignals: new Map(),
    requiredArtifactRuntimeSignalOwners: new Map(),
    missingArtifactRuntimeSignalOwners: new Map(),
    requiredArtifactOwners: new Map(),
    missingArtifactOwners: new Map(),
    requiredArtifactClassifications: new Map(),
    missingArtifactClassifications: new Map(),
    requiredArtifactFailureClassifications: new Map(),
    missingArtifactFailureClassifications: new Map(),
    requiredArtifactPlannedOwners: new Map(),
    missingArtifactPlannedOwners: new Map(),
    requiredArtifactPlannedClassifications: new Map(),
    missingArtifactPlannedClassifications: new Map(),
    requiredArtifactExitCriterionStatuses: new Map(),
    missingArtifactExitCriterionStatuses: new Map(),
    requiredArtifactIncompleteExitCriterionStatuses: new Map(),
    missingArtifactIncompleteExitCriterionStatuses: new Map(),
    artifactSchemas: new Map(),
    artifactCoverageAreas: new Map(),
    artifactRuntimeAuthorityInvariants: new Map(),
    requiredRuntimeSignals: new Map(),
    missingRuntimeSignals: new Map(),
    requiredRuntimeSignalOwners: new Map(),
    missingRuntimeSignalOwners: new Map(),
    requiredFailureClassifications: new Map(),
    missingFailureClassifications: new Map(),
    artifactRuntimeSignals: new Map(),
    artifactRuntimeSignalOwners: new Map(),
    artifactOwners: new Map(),
    artifactClassifications: new Map(),
    artifactFailureClassifications: new Map(),
    artifactPlannedOwners: new Map(),
    artifactPlannedClassifications: new Map(),
    artifactExitCriterionStatuses: new Map(),
    artifactIncompleteExitCriterionStatuses: new Map(),
    artifactKinds: new Map(),
    artifactGeneratedEvidenceKinds: new Map(),
    artifactGeneratedEvidenceRepos: new Map(),
    artifactGeneratedMatrixArtifactIndexes: new Map(),
    artifactGeneratedMatrixLimitations: new Map(),
    artifactGeneratedMatrixNames: new Map(),
    artifactGeneratedMatrixRepos: new Map(),
    artifactGeneratedValidationSuiteArtifactIndexes: new Map(),
    artifactGeneratedValidationSuiteFailureRoots: new Map(),
    artifactEvidenceRepos: new Map(),
    artifactProviderAccountAliases: new Map(),
    artifactValidationPresets: new Map(),
    artifactCoverageInputSources: new Map(),
    failureRuntimeSignals: new Map(),
    failureRuntimeSignalOwners: new Map(),
    failureOwners: new Map(),
    failureClassifications: new Map(),
    failureStaleManifests: new Map(),
    matrixRuntimeSignals: new Map(),
    matrixRuntimeSignalOwners: new Map(),
    matrixOwners: new Map(),
    matrixClassifications: new Map(),
    matrixStaleReports: new Map(),
    requiredMatrices: new Map(),
    missingMatrices: new Map(),
    requiredMatrixClassifications: new Map(),
    missingMatrixClassifications: new Map(),
    requiredMatrixRuntimeSignals: new Map(),
    missingMatrixRuntimeSignals: new Map(),
    requiredDeploymentPresets: new Map(),
    missingDeploymentPresets: new Map(),
    requiredProviders: new Map(),
    missingProviders: new Map(),
    requiredScenarios: new Map(),
    missingScenarios: new Map(),
    generatedEvidenceKinds: new Map(),
    generatedMatrixArtifactIndexes: new Map(),
    generatedMatrixLimitations: new Map(),
    generatedValidationSuiteArtifactIndexes: new Map(),
    generatedValidationSuiteFailureRoots: new Map(),
    requiredGeneratedEvidenceKinds: new Map(),
    missingGeneratedEvidenceKinds: new Map(),
    requiredGeneratedMatrixArtifactIndexes: new Map(),
    missingGeneratedMatrixArtifactIndexes: new Map(),
    requiredGeneratedMatrixLimitations: new Map(),
    missingGeneratedMatrixLimitations: new Map(),
    requiredGeneratedValidationSuiteArtifactIndexes: new Map(),
    missingGeneratedValidationSuiteArtifactIndexes: new Map(),
    requiredGeneratedValidationSuiteFailureRoots: new Map(),
    missingGeneratedValidationSuiteFailureRoots: new Map(),
  }
  for (const report of aggregate.reports) {
    countStringValues(expected.presets, report.presets ?? [])
    const platformCoverage = report.platformCoverage ?? {
      requiredCoverageAreas: [],
      missingCoverageAreas: [],
      requiredRuntimeSignals: [],
      missingRuntimeSignals: [],
      requiredRuntimeSignalOwners: [],
      missingRuntimeSignalOwners: [],
      requiredFailureClassifications: [],
      missingFailureClassifications: [],
    }
    countStringValues(expected.requiredPlatformCoverageAreas, platformCoverage.requiredCoverageAreas ?? [])
    countStringValues(expected.missingPlatformCoverageAreas, platformCoverage.missingCoverageAreas ?? [])
    countStringValues(expected.requiredRuntimeSignals, platformCoverage.requiredRuntimeSignals ?? [])
    countStringValues(expected.missingRuntimeSignals, platformCoverage.missingRuntimeSignals ?? [])
    countStringValues(expected.requiredRuntimeSignalOwners, platformCoverage.requiredRuntimeSignalOwners ?? [])
    countStringValues(expected.missingRuntimeSignalOwners, platformCoverage.missingRuntimeSignalOwners ?? [])
    countStringValues(expected.requiredFailureClassifications, platformCoverage.requiredFailureClassifications ?? [])
    countStringValues(expected.missingFailureClassifications, platformCoverage.missingFailureClassifications ?? [])
    countStringValues(expected.requiredArtifactCoverageAreas, report.artifactCoverage?.requiredArtifactCoverageAreas ?? [])
    countStringValues(expected.missingArtifactCoverageAreas, report.artifactCoverage?.missingArtifactCoverageAreas ?? [])
    countStringValues(expected.requiredArtifactSchemas, report.artifactCoverage?.requiredArtifactSchemas ?? [])
    countStringValues(expected.missingArtifactSchemas, report.artifactCoverage?.missingArtifactSchemas ?? [])
    countStringValues(expected.requiredArtifactKinds, report.artifactCoverage?.requiredArtifactKinds ?? [])
    countStringValues(expected.missingArtifactKinds, report.artifactCoverage?.missingArtifactKinds ?? [])
    countStringValues(expected.requiredArtifactGeneratedEvidenceKinds, report.artifactCoverage?.requiredArtifactGeneratedEvidenceKinds ?? [])
    countStringValues(expected.missingArtifactGeneratedEvidenceKinds, report.artifactCoverage?.missingArtifactGeneratedEvidenceKinds ?? [])
    countStringValues(expected.requiredArtifactGeneratedEvidenceRepos, report.artifactCoverage?.requiredArtifactGeneratedEvidenceRepos ?? [])
    countStringValues(expected.missingArtifactGeneratedEvidenceRepos, report.artifactCoverage?.missingArtifactGeneratedEvidenceRepos ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixArtifactIndexes, report.artifactCoverage?.requiredArtifactGeneratedMatrixArtifactIndexes ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixArtifactIndexes, report.artifactCoverage?.missingArtifactGeneratedMatrixArtifactIndexes ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixLimitations, report.artifactCoverage?.requiredArtifactGeneratedMatrixLimitations ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixLimitations, report.artifactCoverage?.missingArtifactGeneratedMatrixLimitations ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixNames, report.artifactCoverage?.requiredArtifactGeneratedMatrixNames ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixNames, report.artifactCoverage?.missingArtifactGeneratedMatrixNames ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixRepos, report.artifactCoverage?.requiredArtifactGeneratedMatrixRepos ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixRepos, report.artifactCoverage?.missingArtifactGeneratedMatrixRepos ?? [])
    countStringValues(expected.requiredArtifactGeneratedValidationSuiteArtifactIndexes, report.artifactCoverage?.requiredArtifactGeneratedValidationSuiteArtifactIndexes ?? [])
    countStringValues(expected.missingArtifactGeneratedValidationSuiteArtifactIndexes, report.artifactCoverage?.missingArtifactGeneratedValidationSuiteArtifactIndexes ?? [])
    countStringValues(expected.requiredArtifactGeneratedValidationSuiteFailureRoots, report.artifactCoverage?.requiredArtifactGeneratedValidationSuiteFailureRoots ?? [])
    countStringValues(expected.missingArtifactGeneratedValidationSuiteFailureRoots, report.artifactCoverage?.missingArtifactGeneratedValidationSuiteFailureRoots ?? [])
    countStringValues(expected.requiredArtifactEvidenceRepos, report.artifactCoverage?.requiredArtifactEvidenceRepos ?? [])
    countStringValues(expected.missingArtifactEvidenceRepos, report.artifactCoverage?.missingArtifactEvidenceRepos ?? [])
    countStringValues(expected.requiredArtifactProviderAccountAliases, report.artifactCoverage?.requiredArtifactProviderAccountAliases ?? [])
    countStringValues(expected.missingArtifactProviderAccountAliases, report.artifactCoverage?.missingArtifactProviderAccountAliases ?? [])
    countStringValues(expected.requiredArtifactValidationPresets, report.artifactCoverage?.requiredArtifactValidationPresets ?? [])
    countStringValues(expected.missingArtifactValidationPresets, report.artifactCoverage?.missingArtifactValidationPresets ?? [])
    countStringValues(expected.requiredArtifactRuntimeAuthorityInvariants, report.artifactCoverage?.requiredArtifactRuntimeAuthorityInvariants ?? [])
    countStringValues(expected.missingArtifactRuntimeAuthorityInvariants, report.artifactCoverage?.missingArtifactRuntimeAuthorityInvariants ?? [])
    countStringValues(expected.requiredArtifactRuntimeSignals, report.artifactCoverage?.requiredArtifactRuntimeSignals ?? [])
    countStringValues(expected.missingArtifactRuntimeSignals, report.artifactCoverage?.missingArtifactRuntimeSignals ?? [])
    countStringValues(expected.requiredArtifactRuntimeSignalOwners, report.artifactCoverage?.requiredArtifactRuntimeSignalOwners ?? [])
    countStringValues(expected.missingArtifactRuntimeSignalOwners, report.artifactCoverage?.missingArtifactRuntimeSignalOwners ?? [])
    countStringValues(expected.requiredArtifactOwners, report.artifactCoverage?.requiredArtifactOwners ?? [])
    countStringValues(expected.missingArtifactOwners, report.artifactCoverage?.missingArtifactOwners ?? [])
    countStringValues(expected.requiredArtifactClassifications, report.artifactCoverage?.requiredArtifactClassifications ?? [])
    countStringValues(expected.missingArtifactClassifications, report.artifactCoverage?.missingArtifactClassifications ?? [])
    countStringValues(expected.requiredArtifactFailureClassifications, report.artifactCoverage?.requiredArtifactFailureClassifications ?? [])
    countStringValues(expected.missingArtifactFailureClassifications, report.artifactCoverage?.missingArtifactFailureClassifications ?? [])
    countStringValues(expected.requiredArtifactPlannedOwners, report.artifactCoverage?.requiredArtifactPlannedOwners ?? [])
    countStringValues(expected.missingArtifactPlannedOwners, report.artifactCoverage?.missingArtifactPlannedOwners ?? [])
    countStringValues(expected.requiredArtifactPlannedClassifications, report.artifactCoverage?.requiredArtifactPlannedClassifications ?? [])
    countStringValues(expected.missingArtifactPlannedClassifications, report.artifactCoverage?.missingArtifactPlannedClassifications ?? [])
    countStringValues(expected.requiredArtifactExitCriterionStatuses, report.artifactCoverage?.requiredArtifactExitCriterionStatuses ?? [])
    countStringValues(expected.missingArtifactExitCriterionStatuses, report.artifactCoverage?.missingArtifactExitCriterionStatuses ?? [])
    countStringValues(expected.requiredArtifactIncompleteExitCriterionStatuses, report.artifactCoverage?.requiredArtifactIncompleteExitCriterionStatuses ?? [])
    countStringValues(expected.missingArtifactIncompleteExitCriterionStatuses, report.artifactCoverage?.missingArtifactIncompleteExitCriterionStatuses ?? [])
    countObjectValues(expected.artifactSchemas, report.artifactCoverage?.schemas)
    countObjectValues(expected.artifactCoverageAreas, report.artifactCoverage?.coverageAreas)
    countObjectValues(expected.artifactRuntimeAuthorityInvariants, report.artifactCoverage?.runtimeAuthorityInvariants)
    countObjectValues(expected.artifactRuntimeSignals, report.artifactCoverage?.runtimeSignals)
    countObjectValues(expected.artifactRuntimeSignalOwners, report.artifactCoverage?.runtimeSignalOwners)
    countObjectValues(expected.artifactOwners, report.artifactCoverage?.owners)
    countObjectValues(expected.artifactClassifications, report.artifactCoverage?.classifications)
    countObjectValues(expected.artifactFailureClassifications, report.artifactCoverage?.failureClassifications)
    countObjectValues(expected.artifactPlannedOwners, report.artifactCoverage?.plannedOwners)
    countObjectValues(expected.artifactPlannedClassifications, report.artifactCoverage?.plannedClassifications)
    countObjectValues(expected.artifactExitCriterionStatuses, report.artifactCoverage?.exitCriterionStatuses)
    countObjectValues(expected.artifactIncompleteExitCriterionStatuses, report.artifactCoverage?.incompleteExitCriterionStatuses)
    countObjectValues(expected.artifactKinds, report.artifactCoverage?.artifactKinds)
    countObjectValues(expected.artifactGeneratedEvidenceKinds, report.artifactCoverage?.generatedEvidenceKinds)
    countObjectValues(expected.artifactGeneratedEvidenceRepos, report.artifactCoverage?.generatedEvidenceRepos)
    countObjectValues(expected.artifactGeneratedMatrixArtifactIndexes, report.artifactCoverage?.generatedMatrixArtifactIndexes)
    countObjectValues(expected.artifactGeneratedMatrixLimitations, report.artifactCoverage?.generatedMatrixLimitations)
    countObjectValues(expected.artifactGeneratedMatrixNames, report.artifactCoverage?.generatedMatrixNames)
    countObjectValues(expected.artifactGeneratedMatrixRepos, report.artifactCoverage?.generatedMatrixRepos)
    countObjectValues(expected.artifactGeneratedValidationSuiteArtifactIndexes, report.artifactCoverage?.generatedValidationSuiteArtifactIndexes)
    countObjectValues(expected.artifactGeneratedValidationSuiteFailureRoots, report.artifactCoverage?.generatedValidationSuiteFailureRoots)
    countObjectValues(expected.artifactEvidenceRepos, report.artifactCoverage?.evidenceRepos)
    countObjectValues(expected.artifactProviderAccountAliases, report.artifactCoverage?.providerAccountAliases)
    countObjectValues(expected.artifactValidationPresets, report.artifactCoverage?.validationPresets)
    countObjectValues(expected.artifactCoverageInputSources, report.artifactCoverage?.artifactCoverageInputSources)
    countObjectValues(expected.failureRuntimeSignals, report.failureCoverage?.runtimeSignals)
    countObjectValues(expected.failureRuntimeSignalOwners, report.failureCoverage?.runtimeSignalOwners)
    countObjectValues(expected.failureOwners, report.failureCoverage?.owners)
    countObjectValues(expected.failureClassifications, report.failureCoverage?.classifications)
    countStringValues(expected.failureStaleManifests, staleFailureManifestSourceLabels(report.failureCoverage?.staleFailureManifests))
    const coverage = report.matrixCoverage ?? {
      runtimeSignals: {},
      runtimeSignalOwners: {},
      owners: {},
      classifications: {},
      requiredMatrices: [],
      missingMatrices: [],
      requiredMatrixClassifications: [],
      missingMatrixClassifications: [],
      requiredMatrixRuntimeSignals: [],
      missingMatrixRuntimeSignals: [],
      requiredDeploymentPresets: [],
      missingDeploymentPresets: [],
      requiredProviders: [],
      missingProviders: [],
      requiredScenarios: [],
      missingScenarios: [],
    }
    countObjectValues(expected.matrixRuntimeSignals, coverage.runtimeSignals)
    countObjectValues(expected.matrixRuntimeSignalOwners, coverage.runtimeSignalOwners)
    countObjectValues(expected.matrixOwners, coverage.owners)
    countObjectValues(expected.matrixClassifications, coverage.classifications)
    countStringValues(expected.matrixStaleReports, staleMatrixReportSourceLabels(coverage.staleMatrixReports))
    countStringValues(expected.requiredMatrices, coverage.requiredMatrices ?? [])
    countStringValues(expected.missingMatrices, coverage.missingMatrices ?? [])
    countStringValues(expected.requiredMatrixClassifications, coverage.requiredMatrixClassifications ?? [])
    countStringValues(expected.missingMatrixClassifications, coverage.missingMatrixClassifications ?? [])
    countStringValues(expected.requiredMatrixRuntimeSignals, coverage.requiredMatrixRuntimeSignals ?? [])
    countStringValues(expected.missingMatrixRuntimeSignals, coverage.missingMatrixRuntimeSignals ?? [])
    countStringValues(expected.requiredDeploymentPresets, coverage.requiredDeploymentPresets ?? [])
    countStringValues(expected.missingDeploymentPresets, coverage.missingDeploymentPresets ?? [])
    countStringValues(expected.requiredProviders, coverage.requiredProviders ?? [])
    countStringValues(expected.missingProviders, coverage.missingProviders ?? [])
    countStringValues(expected.requiredScenarios, coverage.requiredScenarios ?? [])
    countStringValues(expected.missingScenarios, coverage.missingScenarios ?? [])
    countStringValues(expected.generatedEvidenceKinds, report.generatedEvidence?.kinds ?? [])
    countStringValues(
      expected.generatedMatrixLimitations,
      (report.generatedEvidence?.matrixReports?.limitations ?? []).map((limitation) => limitation.kind),
    )
    countStringValues(expected.generatedMatrixArtifactIndexes, report.generatedEvidence?.matrixReports?.artifactIndexes ?? [])
    countStringValues(expected.generatedValidationSuiteArtifactIndexes, report.generatedEvidence?.validationSuites?.artifactIndexes ?? [])
    countStringValues(expected.generatedValidationSuiteFailureRoots, report.generatedEvidence?.validationSuites?.failureRoots ?? [])
  }
  for (const input of aggregate.artifactCoverageInputs ?? []) {
    countStringValues(expected.requiredArtifactCoverageAreas, input.artifactCoverage?.requiredArtifactCoverageAreas ?? [])
    countStringValues(expected.missingArtifactCoverageAreas, input.artifactCoverage?.missingArtifactCoverageAreas ?? [])
    countStringValues(expected.requiredArtifactSchemas, input.artifactCoverage?.requiredArtifactSchemas ?? [])
    countStringValues(expected.missingArtifactSchemas, input.artifactCoverage?.missingArtifactSchemas ?? [])
    countStringValues(expected.requiredArtifactKinds, input.artifactCoverage?.requiredArtifactKinds ?? [])
    countStringValues(expected.missingArtifactKinds, input.artifactCoverage?.missingArtifactKinds ?? [])
    countStringValues(expected.requiredArtifactGeneratedEvidenceKinds, input.artifactCoverage?.requiredArtifactGeneratedEvidenceKinds ?? [])
    countStringValues(expected.missingArtifactGeneratedEvidenceKinds, input.artifactCoverage?.missingArtifactGeneratedEvidenceKinds ?? [])
    countStringValues(expected.requiredArtifactGeneratedEvidenceRepos, input.artifactCoverage?.requiredArtifactGeneratedEvidenceRepos ?? [])
    countStringValues(expected.missingArtifactGeneratedEvidenceRepos, input.artifactCoverage?.missingArtifactGeneratedEvidenceRepos ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixArtifactIndexes, input.artifactCoverage?.requiredArtifactGeneratedMatrixArtifactIndexes ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixArtifactIndexes, input.artifactCoverage?.missingArtifactGeneratedMatrixArtifactIndexes ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixLimitations, input.artifactCoverage?.requiredArtifactGeneratedMatrixLimitations ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixLimitations, input.artifactCoverage?.missingArtifactGeneratedMatrixLimitations ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixNames, input.artifactCoverage?.requiredArtifactGeneratedMatrixNames ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixNames, input.artifactCoverage?.missingArtifactGeneratedMatrixNames ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixRepos, input.artifactCoverage?.requiredArtifactGeneratedMatrixRepos ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixRepos, input.artifactCoverage?.missingArtifactGeneratedMatrixRepos ?? [])
    countStringValues(expected.requiredArtifactGeneratedValidationSuiteArtifactIndexes, input.artifactCoverage?.requiredArtifactGeneratedValidationSuiteArtifactIndexes ?? [])
    countStringValues(expected.missingArtifactGeneratedValidationSuiteArtifactIndexes, input.artifactCoverage?.missingArtifactGeneratedValidationSuiteArtifactIndexes ?? [])
    countStringValues(expected.requiredArtifactGeneratedValidationSuiteFailureRoots, input.artifactCoverage?.requiredArtifactGeneratedValidationSuiteFailureRoots ?? [])
    countStringValues(expected.missingArtifactGeneratedValidationSuiteFailureRoots, input.artifactCoverage?.missingArtifactGeneratedValidationSuiteFailureRoots ?? [])
    countStringValues(expected.requiredArtifactEvidenceRepos, input.artifactCoverage?.requiredArtifactEvidenceRepos ?? [])
    countStringValues(expected.missingArtifactEvidenceRepos, input.artifactCoverage?.missingArtifactEvidenceRepos ?? [])
    countStringValues(expected.requiredArtifactProviderAccountAliases, input.artifactCoverage?.requiredArtifactProviderAccountAliases ?? [])
    countStringValues(expected.missingArtifactProviderAccountAliases, input.artifactCoverage?.missingArtifactProviderAccountAliases ?? [])
    countStringValues(expected.requiredArtifactValidationPresets, input.artifactCoverage?.requiredArtifactValidationPresets ?? [])
    countStringValues(expected.missingArtifactValidationPresets, input.artifactCoverage?.missingArtifactValidationPresets ?? [])
    countStringValues(expected.requiredArtifactRuntimeAuthorityInvariants, input.artifactCoverage?.requiredArtifactRuntimeAuthorityInvariants ?? [])
    countStringValues(expected.missingArtifactRuntimeAuthorityInvariants, input.artifactCoverage?.missingArtifactRuntimeAuthorityInvariants ?? [])
    countStringValues(expected.requiredArtifactRuntimeSignals, input.artifactCoverage?.requiredArtifactRuntimeSignals ?? [])
    countStringValues(expected.missingArtifactRuntimeSignals, input.artifactCoverage?.missingArtifactRuntimeSignals ?? [])
    countStringValues(expected.requiredArtifactRuntimeSignalOwners, input.artifactCoverage?.requiredArtifactRuntimeSignalOwners ?? [])
    countStringValues(expected.missingArtifactRuntimeSignalOwners, input.artifactCoverage?.missingArtifactRuntimeSignalOwners ?? [])
    countStringValues(expected.requiredArtifactOwners, input.artifactCoverage?.requiredArtifactOwners ?? [])
    countStringValues(expected.missingArtifactOwners, input.artifactCoverage?.missingArtifactOwners ?? [])
    countStringValues(expected.requiredArtifactClassifications, input.artifactCoverage?.requiredArtifactClassifications ?? [])
    countStringValues(expected.missingArtifactClassifications, input.artifactCoverage?.missingArtifactClassifications ?? [])
    countStringValues(expected.requiredArtifactFailureClassifications, input.artifactCoverage?.requiredArtifactFailureClassifications ?? [])
    countStringValues(expected.missingArtifactFailureClassifications, input.artifactCoverage?.missingArtifactFailureClassifications ?? [])
    countStringValues(expected.requiredArtifactPlannedOwners, input.artifactCoverage?.requiredArtifactPlannedOwners ?? [])
    countStringValues(expected.missingArtifactPlannedOwners, input.artifactCoverage?.missingArtifactPlannedOwners ?? [])
    countStringValues(expected.requiredArtifactPlannedClassifications, input.artifactCoverage?.requiredArtifactPlannedClassifications ?? [])
    countStringValues(expected.missingArtifactPlannedClassifications, input.artifactCoverage?.missingArtifactPlannedClassifications ?? [])
    countStringValues(expected.requiredArtifactExitCriterionStatuses, input.artifactCoverage?.requiredArtifactExitCriterionStatuses ?? [])
    countStringValues(expected.missingArtifactExitCriterionStatuses, input.artifactCoverage?.missingArtifactExitCriterionStatuses ?? [])
    countStringValues(expected.requiredArtifactIncompleteExitCriterionStatuses, input.artifactCoverage?.requiredArtifactIncompleteExitCriterionStatuses ?? [])
    countStringValues(expected.missingArtifactIncompleteExitCriterionStatuses, input.artifactCoverage?.missingArtifactIncompleteExitCriterionStatuses ?? [])
    countObjectValues(expected.artifactSchemas, input.artifactCoverage?.schemas)
    countObjectValues(expected.artifactCoverageAreas, input.artifactCoverage?.coverageAreas)
    countObjectValues(expected.artifactRuntimeAuthorityInvariants, input.artifactCoverage?.runtimeAuthorityInvariants)
    countObjectValues(expected.artifactRuntimeSignals, input.artifactCoverage?.runtimeSignals)
    countObjectValues(expected.artifactRuntimeSignalOwners, input.artifactCoverage?.runtimeSignalOwners)
    countObjectValues(expected.artifactOwners, input.artifactCoverage?.owners)
    countObjectValues(expected.artifactClassifications, input.artifactCoverage?.classifications)
    countObjectValues(expected.artifactFailureClassifications, input.artifactCoverage?.failureClassifications)
    countObjectValues(expected.artifactPlannedOwners, input.artifactCoverage?.plannedOwners)
    countObjectValues(expected.artifactPlannedClassifications, input.artifactCoverage?.plannedClassifications)
    countObjectValues(expected.artifactExitCriterionStatuses, input.artifactCoverage?.exitCriterionStatuses)
    countObjectValues(expected.artifactIncompleteExitCriterionStatuses, input.artifactCoverage?.incompleteExitCriterionStatuses)
    countObjectValues(expected.artifactKinds, input.artifactCoverage?.artifactKinds)
    countObjectValues(expected.artifactGeneratedEvidenceKinds, input.artifactCoverage?.generatedEvidenceKinds)
    countObjectValues(expected.artifactGeneratedEvidenceRepos, input.artifactCoverage?.generatedEvidenceRepos)
    countObjectValues(expected.artifactGeneratedMatrixArtifactIndexes, input.artifactCoverage?.generatedMatrixArtifactIndexes)
    countObjectValues(expected.artifactGeneratedMatrixLimitations, input.artifactCoverage?.generatedMatrixLimitations)
    countObjectValues(expected.artifactGeneratedMatrixNames, input.artifactCoverage?.generatedMatrixNames)
    countObjectValues(expected.artifactGeneratedMatrixRepos, input.artifactCoverage?.generatedMatrixRepos)
    countObjectValues(expected.artifactGeneratedValidationSuiteArtifactIndexes, input.artifactCoverage?.generatedValidationSuiteArtifactIndexes)
    countObjectValues(expected.artifactGeneratedValidationSuiteFailureRoots, input.artifactCoverage?.generatedValidationSuiteFailureRoots)
    countObjectValues(expected.artifactEvidenceRepos, input.artifactCoverage?.evidenceRepos)
    countObjectValues(expected.artifactProviderAccountAliases, input.artifactCoverage?.providerAccountAliases)
    countObjectValues(expected.artifactValidationPresets, input.artifactCoverage?.validationPresets)
    countObjectValues(expected.artifactCoverageInputSources, input.artifactCoverage?.artifactCoverageInputSources)
  }
  countStringValues(expected.requiredGeneratedEvidenceKinds, aggregate.requiredGeneratedEvidenceKinds ?? [])
  countStringValues(expected.missingGeneratedEvidenceKinds, aggregate.missingGeneratedEvidenceKinds ?? [])
  countStringValues(expected.requiredGeneratedMatrixArtifactIndexes, aggregate.requiredGeneratedMatrixArtifactIndexes ?? [])
  countStringValues(expected.missingGeneratedMatrixArtifactIndexes, aggregate.missingGeneratedMatrixArtifactIndexes ?? [])
  countStringValues(expected.requiredGeneratedMatrixLimitations, aggregate.requiredGeneratedMatrixLimitations ?? [])
  countStringValues(expected.missingGeneratedMatrixLimitations, aggregate.missingGeneratedMatrixLimitations ?? [])
  countStringValues(expected.requiredGeneratedValidationSuiteArtifactIndexes, aggregate.requiredGeneratedValidationSuiteArtifactIndexes ?? [])
  countStringValues(expected.missingGeneratedValidationSuiteArtifactIndexes, aggregate.missingGeneratedValidationSuiteArtifactIndexes ?? [])
  countStringValues(expected.requiredGeneratedValidationSuiteFailureRoots, aggregate.requiredGeneratedValidationSuiteFailureRoots ?? [])
  countStringValues(expected.missingGeneratedValidationSuiteFailureRoots, aggregate.missingGeneratedValidationSuiteFailureRoots ?? [])
  const expectedCoverage = formatValidationGateCoverageCounts(expected)
  if (JSON.stringify(aggregate.coverage) !== JSON.stringify(expectedCoverage)) {
    throw new Error(`${source} coverage does not match reports`)
  }
}

export function assertMatrixRuntimeSignalSourcesMatchReports(aggregate, source) {
  const expected = new Map()
  for (const report of aggregate.reports) {
    appendMatrixRuntimeSignalSources(expected, {
      reportSource: report.source ?? null,
      runtimeSignalScenarios: report.matrixCoverage?.runtimeSignalScenarios,
    })
  }
  const expectedSources = formatMatrixRuntimeSignalSources(expected)
  if (JSON.stringify(aggregate.matrixRuntimeSignalSources ?? {}) !== JSON.stringify(expectedSources)) {
    throw new Error(`${source} matrixRuntimeSignalSources does not match reports`)
  }
}

export function validateGateAggregateReportSummary(report, source) {
  if (!report || typeof report !== "object" || Array.isArray(report)) {
    throw new Error(`${source} is not an object`)
  }
  if (report.source !== null && typeof report.source !== "string") {
    throw new Error(`${source} has invalid source`)
  }
  validateDrillValidationResultStatus(report.status, source)
  validatePresetArray(report.presets ?? [], `${source}.presets`)
  if (!report.checks || typeof report.checks !== "object" || Array.isArray(report.checks)) {
    throw new Error(`${source} has invalid checks`)
  }
  for (const name of ["configuration", "platformBundle", "artifacts", "matrices", "failures"]) {
    validateDrillValidationCheckStatus(report.checks[name], `${source}.checks.${name}`, {
      message: () => `${source}.checks has invalid ${name}`,
    })
  }
  if (report.matrixCoverage !== undefined) {
    validateValidationGateMatrixCoverage(report.matrixCoverage, `${source}.matrixCoverage`)
  }
  if (report.platformCoverage !== undefined) {
    validateValidationGatePlatformCoverage(report.platformCoverage, `${source}.platformCoverage`)
  }
  if (report.artifactCoverage !== undefined) {
    validateValidationGateArtifactCoverage(report.artifactCoverage, `${source}.artifactCoverage`)
  }
  if (report.failureCoverage !== undefined) {
    validateValidationGateFailureCoverage(report.failureCoverage, `${source}.failureCoverage`)
  }
  if (report.generatedEvidence !== undefined) {
    validateValidationGateGeneratedEvidenceSummary(report.generatedEvidence, `${source}.generatedEvidence`)
  }
}

export function validateGateAggregateArtifactCoverageInput(input, source) {
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new Error(`${source} is not an object`)
  }
  if (input.source !== null && typeof input.source !== "string") {
    throw new Error(`${source} has invalid source`)
  }
  validateDrillValidationResultStatus(input.status, source)
  if (!input.checks || typeof input.checks !== "object" || Array.isArray(input.checks)) {
    throw new Error(`${source} has invalid checks`)
  }
  for (const name of ["configuration", "platformBundle", "artifacts", "matrices", "failures"]) {
    validateDrillValidationCheckStatus(input.checks[name], `${source}.checks.${name}`, {
      message: () => `${source}.checks has invalid ${name}`,
    })
  }
  validateValidationGateArtifactCoverage(input.artifactCoverage, `${source}.artifactCoverage`)
}

export function validateValidationGateGeneratedEvidenceSummary(generatedEvidence, source) {
  if (!generatedEvidence || typeof generatedEvidence !== "object" || Array.isArray(generatedEvidence)) {
    throw new Error(`${source} is not an object`)
  }
  validateGeneratedEvidenceKindArray(generatedEvidence.kinds ?? [], `${source}.kinds`)
  validateGeneratedValidationSuitesSummary(generatedEvidence.validationSuites, `${source}.validationSuites`)
  validateGeneratedMatrixReportsSummary(generatedEvidence.matrixReports, `${source}.matrixReports`)
}

export function validateGeneratedValidationSuitesSummary(validationSuites, source) {
  if (!validationSuites || typeof validationSuites !== "object" || Array.isArray(validationSuites)) {
    throw new Error(`${source} is not an object`)
  }
  if (typeof validationSuites.enabled !== "boolean") {
    throw new Error(`${source} has invalid enabled`)
  }
  validateGeneratedEvidencePathArray(validationSuites.artifactIndexes ?? [], `${source}.artifactIndexes`)
  validateGeneratedEvidencePathArray(validationSuites.failureRoots ?? [], `${source}.failureRoots`)
  if (!Array.isArray(validationSuites.commands)) {
    throw new Error(`${source}.commands is not an array`)
  }
  for (const [index, command] of validationSuites.commands.entries()) {
    validateGeneratedValidationSuiteCommandSummary(command, `${source}.commands[${index}]`)
  }
  validateGeneratedEvidencePathArray(validationSuites.outputRoots ?? [], `${source}.outputRoots`)
  if (validationSuites.enabled && ((validationSuites.artifactIndexes ?? []).length === 0 || (validationSuites.failureRoots ?? []).length === 0 || validationSuites.commands.length === 0 || (validationSuites.outputRoots ?? []).length === 0)) {
    throw new Error(`${source} enabled evidence is missing paths`)
  }
  if (!validationSuites.enabled && ((validationSuites.artifactIndexes ?? []).length > 0 || (validationSuites.failureRoots ?? []).length > 0 || validationSuites.commands.length > 0 || (validationSuites.outputRoots ?? []).length > 0)) {
    throw new Error(`${source} disabled evidence has paths`)
  }
}

export function validateGeneratedValidationSuiteCommandSummary(command, source) {
  if (!command || typeof command !== "object" || Array.isArray(command)) {
    throw new Error(`${source} is not an object`)
  }
  for (const key of ["artifactIndexPath", "cwd", "failureRoot", "reportPath", "scriptPath"]) {
    if (!nonEmptyString(command[key])) {
      throw new Error(`${source} has invalid ${key}`)
    }
    validateGeneratedEvidencePathText(command[key], `${source}.${key}`)
  }
  validateGeneratedEvidencePathArray(command.args ?? [], `${source}.args`)
  validateGeneratedEvidencePathArray(command.nodeArgs, `${source}.nodeArgs`)
}

export function validateGeneratedMatrixReportsSummary(matrixReports, source) {
  if (!matrixReports || typeof matrixReports !== "object" || Array.isArray(matrixReports)) {
    throw new Error(`${source} is not an object`)
  }
  if (typeof matrixReports.enabled !== "boolean") {
    throw new Error(`${source} has invalid enabled`)
  }
  if (typeof matrixReports.dryRun !== "boolean") {
    throw new Error(`${source} has invalid dryRun`)
  }
  if (typeof matrixReports.continueOnFailure !== "boolean") {
    throw new Error(`${source} has invalid continueOnFailure`)
  }
  validateGeneratedMatrixLimitations(matrixReports.limitations ?? [], `${source}.limitations`)
  validateGeneratedEvidencePathArray(matrixReports.artifactIndexes ?? [], `${source}.artifactIndexes`)
  validateGeneratedEvidencePathArray(matrixReports.roots ?? [], `${source}.roots`)
  if (!Array.isArray(matrixReports.commands)) {
    throw new Error(`${source}.commands is not an array`)
  }
  for (const [index, command] of matrixReports.commands.entries()) {
    validateGeneratedMatrixCommandSummary(command, `${source}.commands[${index}]`)
  }
  if (matrixReports.enabled && ((matrixReports.artifactIndexes ?? []).length === 0 || (matrixReports.roots ?? []).length === 0 || matrixReports.commands.length === 0)) {
    throw new Error(`${source} enabled evidence is missing paths`)
  }
  if (matrixReports.enabled && matrixReports.dryRun && (matrixReports.limitations ?? []).length === 0) {
    throw new Error(`${source} dry-run evidence is missing limitations`)
  }
  if (!matrixReports.enabled && ((matrixReports.artifactIndexes ?? []).length > 0 || (matrixReports.roots ?? []).length > 0 || matrixReports.commands.length > 0 || (matrixReports.limitations ?? []).length > 0)) {
    throw new Error(`${source} disabled evidence has generated data`)
  }
}

export function validateGeneratedMatrixLimitations(limitations, source) {
  if (!Array.isArray(limitations)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, limitation] of limitations.entries()) {
    const limitationSource = `${source}[${index}]`
    if (!limitation || typeof limitation !== "object" || Array.isArray(limitation)) {
      throw new Error(`${limitationSource} is not an object`)
    }
    for (const key of ["kind", "owner", "nextAction"]) {
      if (!nonEmptyString(limitation[key])) {
        throw new Error(`${limitationSource} has invalid ${key}`)
      }
    }
    validateDrillGeneratedMatrixLimitation(limitation.kind, limitationSource)
  }
}

export function validateGeneratedMatrixCommandSummary(command, source) {
  if (!command || typeof command !== "object" || Array.isArray(command)) {
    throw new Error(`${source} is not an object`)
  }
  validateDrillGeneratedMatrixCommandMetadata(command, source)
  for (const key of ["artifactIndexFlag", "artifactIndexPath", "cwd", "reportPath", "scriptPath"]) {
    if (!nonEmptyString(command[key])) {
      throw new Error(`${source} has invalid ${key}`)
    }
    if (key !== "artifactIndexFlag") {
      validateGeneratedEvidencePathText(command[key], `${source}.${key}`)
    }
  }
  validateGeneratedEvidencePathArray(command.args ?? [], `${source}.args`)
  validateGeneratedEvidencePathArray(command.nodeArgs, `${source}.nodeArgs`)
}

export function validateValidationGateArtifactCoverage(coverage, source) {
  if (!coverage || typeof coverage !== "object" || Array.isArray(coverage)) {
    throw new Error(`${source} is not an object`)
  }
  validateStringArray(coverage.requiredArtifactCoverageAreas ?? [], `${source}.requiredArtifactCoverageAreas`)
  validateStringArray(coverage.missingArtifactCoverageAreas ?? [], `${source}.missingArtifactCoverageAreas`)
  validateStringArray(coverage.requiredArtifactSchemas ?? [], `${source}.requiredArtifactSchemas`)
  validateStringArray(coverage.missingArtifactSchemas ?? [], `${source}.missingArtifactSchemas`)
  validateArtifactKindArray(coverage.requiredArtifactKinds ?? [], `${source}.requiredArtifactKinds`)
  validateArtifactKindArray(coverage.missingArtifactKinds ?? [], `${source}.missingArtifactKinds`)
  validateGeneratedEvidenceKindArray(coverage.requiredArtifactGeneratedEvidenceKinds ?? [], `${source}.requiredArtifactGeneratedEvidenceKinds`)
  validateGeneratedEvidenceKindArray(coverage.missingArtifactGeneratedEvidenceKinds ?? [], `${source}.missingArtifactGeneratedEvidenceKinds`)
  validateArtifactEvidenceRepoCountObject(coverage.generatedEvidenceRepos ?? {}, `${source}.generatedEvidenceRepos`)
  validateArtifactEvidenceRepoArray(coverage.requiredArtifactGeneratedEvidenceRepos ?? [], `${source}.requiredArtifactGeneratedEvidenceRepos`)
  validateArtifactEvidenceRepoArray(coverage.missingArtifactGeneratedEvidenceRepos ?? [], `${source}.missingArtifactGeneratedEvidenceRepos`)
  validateGeneratedEvidencePathArray(coverage.requiredArtifactGeneratedMatrixArtifactIndexes ?? [], `${source}.requiredArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedEvidencePathArray(coverage.missingArtifactGeneratedMatrixArtifactIndexes ?? [], `${source}.missingArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationArray(coverage.requiredArtifactGeneratedMatrixLimitations ?? [], `${source}.requiredArtifactGeneratedMatrixLimitations`)
  validateGeneratedMatrixLimitationArray(coverage.missingArtifactGeneratedMatrixLimitations ?? [], `${source}.missingArtifactGeneratedMatrixLimitations`)
  validateStringArray(coverage.requiredArtifactGeneratedMatrixNames ?? [], `${source}.requiredArtifactGeneratedMatrixNames`)
  validateStringArray(coverage.missingArtifactGeneratedMatrixNames ?? [], `${source}.missingArtifactGeneratedMatrixNames`)
  validateArtifactEvidenceRepoArray(coverage.requiredArtifactGeneratedMatrixRepos ?? [], `${source}.requiredArtifactGeneratedMatrixRepos`)
  validateArtifactEvidenceRepoArray(coverage.missingArtifactGeneratedMatrixRepos ?? [], `${source}.missingArtifactGeneratedMatrixRepos`)
  validateGeneratedEvidencePathArray(coverage.requiredArtifactGeneratedValidationSuiteArtifactIndexes ?? [], `${source}.requiredArtifactGeneratedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathArray(coverage.missingArtifactGeneratedValidationSuiteArtifactIndexes ?? [], `${source}.missingArtifactGeneratedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathArray(coverage.requiredArtifactGeneratedValidationSuiteFailureRoots ?? [], `${source}.requiredArtifactGeneratedValidationSuiteFailureRoots`)
  validateGeneratedEvidencePathArray(coverage.missingArtifactGeneratedValidationSuiteFailureRoots ?? [], `${source}.missingArtifactGeneratedValidationSuiteFailureRoots`)
  validateArtifactEvidenceRepoArray(coverage.requiredArtifactEvidenceRepos ?? [], `${source}.requiredArtifactEvidenceRepos`)
  validateArtifactEvidenceRepoArray(coverage.missingArtifactEvidenceRepos ?? [], `${source}.missingArtifactEvidenceRepos`)
  validateProviderAccountAliasArray(coverage.requiredArtifactProviderAccountAliases ?? [], `${source}.requiredArtifactProviderAccountAliases`)
  validateProviderAccountAliasArray(coverage.missingArtifactProviderAccountAliases ?? [], `${source}.missingArtifactProviderAccountAliases`)
  validateArtifactValidationPresetArray(coverage.requiredArtifactValidationPresets ?? [], `${source}.requiredArtifactValidationPresets`)
  validateArtifactValidationPresetArray(coverage.missingArtifactValidationPresets ?? [], `${source}.missingArtifactValidationPresets`)
  validateRuntimeAuthorityInvariantArray(coverage.requiredArtifactRuntimeAuthorityInvariants ?? [], `${source}.requiredArtifactRuntimeAuthorityInvariants`)
  validateRuntimeAuthorityInvariantArray(coverage.missingArtifactRuntimeAuthorityInvariants ?? [], `${source}.missingArtifactRuntimeAuthorityInvariants`)
  validateRuntimeSignalArray(coverage.requiredArtifactRuntimeSignals ?? [], `${source}.requiredArtifactRuntimeSignals`)
  validateRuntimeSignalArray(coverage.missingArtifactRuntimeSignals ?? [], `${source}.missingArtifactRuntimeSignals`)
  validateRuntimeSignalOwnerArray(coverage.requiredArtifactRuntimeSignalOwners ?? [], `${source}.requiredArtifactRuntimeSignalOwners`)
  validateRuntimeSignalOwnerArray(coverage.missingArtifactRuntimeSignalOwners ?? [], `${source}.missingArtifactRuntimeSignalOwners`)
  validateStringArray(coverage.requiredArtifactOwners ?? [], `${source}.requiredArtifactOwners`)
  validateStringArray(coverage.missingArtifactOwners ?? [], `${source}.missingArtifactOwners`)
  validateStringArray(coverage.requiredArtifactClassifications ?? [], `${source}.requiredArtifactClassifications`)
  validateStringArray(coverage.missingArtifactClassifications ?? [], `${source}.missingArtifactClassifications`)
  validateFailureClassificationArray(coverage.requiredArtifactFailureClassifications ?? [], `${source}.requiredArtifactFailureClassifications`)
  validateFailureClassificationArray(coverage.missingArtifactFailureClassifications ?? [], `${source}.missingArtifactFailureClassifications`)
  validateStringArray(coverage.requiredArtifactPlannedOwners ?? [], `${source}.requiredArtifactPlannedOwners`)
  validateStringArray(coverage.missingArtifactPlannedOwners ?? [], `${source}.missingArtifactPlannedOwners`)
  validateStringArray(coverage.requiredArtifactPlannedClassifications ?? [], `${source}.requiredArtifactPlannedClassifications`)
  validateStringArray(coverage.missingArtifactPlannedClassifications ?? [], `${source}.missingArtifactPlannedClassifications`)
  validateExitCriterionStatusArray(coverage.requiredArtifactExitCriterionStatuses ?? [], `${source}.requiredArtifactExitCriterionStatuses`)
  validateExitCriterionStatusArray(coverage.missingArtifactExitCriterionStatuses ?? [], `${source}.missingArtifactExitCriterionStatuses`)
  validateExitCriterionStatusArray(coverage.requiredArtifactIncompleteExitCriterionStatuses ?? [], `${source}.requiredArtifactIncompleteExitCriterionStatuses`)
  validateExitCriterionStatusArray(coverage.missingArtifactIncompleteExitCriterionStatuses ?? [], `${source}.missingArtifactIncompleteExitCriterionStatuses`)
  validateCountObject(coverage.schemas ?? {}, `${source}.schemas`)
  validateCountObject(coverage.coverageAreas ?? {}, `${source}.coverageAreas`)
  validateRuntimeAuthorityInvariantCountObject(coverage.runtimeAuthorityInvariants ?? {}, `${source}.runtimeAuthorityInvariants`)
  validateRuntimeSignalCountObject(coverage.runtimeSignals ?? {}, `${source}.runtimeSignals`)
  validateRuntimeSignalOwnerCountObject(coverage.runtimeSignalOwners ?? {}, `${source}.runtimeSignalOwners`)
  validateCountObject(coverage.owners ?? {}, `${source}.owners`)
  validateCountObject(coverage.classifications ?? {}, `${source}.classifications`)
  validateFailureClassificationCountObject(coverage.failureClassifications ?? {}, `${source}.failureClassifications`)
  validateCountObject(coverage.plannedOwners ?? {}, `${source}.plannedOwners`)
  validateCountObject(coverage.plannedClassifications ?? {}, `${source}.plannedClassifications`)
  validateExitCriterionStatusCountObject(coverage.exitCriterionStatuses ?? {}, `${source}.exitCriterionStatuses`)
  validateExitCriterionStatusCountObject(coverage.incompleteExitCriterionStatuses ?? {}, `${source}.incompleteExitCriterionStatuses`)
  validateArtifactKindCountObject(coverage.artifactKinds ?? {}, `${source}.artifactKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.generatedEvidenceKinds ?? {}, `${source}.generatedEvidenceKinds`)
  validateGeneratedEvidencePathCountObject(coverage.generatedMatrixArtifactIndexes ?? {}, `${source}.generatedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationCountObject(coverage.generatedMatrixLimitations ?? {}, `${source}.generatedMatrixLimitations`)
  validateGeneratedEvidencePathCountObject(coverage.generatedValidationSuiteArtifactIndexes ?? {}, `${source}.generatedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathCountObject(coverage.generatedValidationSuiteFailureRoots ?? {}, `${source}.generatedValidationSuiteFailureRoots`)
  validateArtifactEvidenceRepoCountObject(coverage.evidenceRepos ?? {}, `${source}.evidenceRepos`)
  validateProviderAccountAliasCountObject(coverage.providerAccountAliases ?? {}, `${source}.providerAccountAliases`)
  validateArtifactValidationPresetCountObject(coverage.validationPresets ?? {}, `${source}.validationPresets`)
  validateCountObject(coverage.artifactCoverageInputSources ?? {}, `${source}.artifactCoverageInputSources`)
}
