import { countDrillAggregateNextAction } from "./drill-aggregate-actions.mjs"
import { drillRuntimeSignalOwnerCounts } from "./drill-runtime-signals.mjs"

export function validationGateReportPlatformCoverage(report) {
  const platform = report.checks.platformBundle
  return {
    requiredCoverageAreas: [...(platform.requiredCoverageAreas ?? [])],
    missingCoverageAreas: [...(platform.missingCoverageAreas ?? [])],
    requiredRuntimeSignals: [...(platform.requiredRuntimeSignals ?? [])],
    missingRuntimeSignals: [...(platform.missingRuntimeSignals ?? [])],
    requiredRuntimeSignalOwners: [...(platform.requiredRuntimeSignalOwners ?? [])],
    missingRuntimeSignalOwners: [...(platform.missingRuntimeSignalOwners ?? [])],
    requiredFailureClassifications: [...(platform.requiredFailureClassifications ?? [])],
    missingFailureClassifications: [...(platform.missingFailureClassifications ?? [])],
  }
}

export function validationGateReportArtifactCoverage(report) {
  return {
    requiredArtifactCoverageAreas: [...(report.checks.artifacts.requiredArtifactCoverageAreas ?? [])],
    missingArtifactCoverageAreas: [...(report.checks.artifacts.missingArtifactCoverageAreas ?? [])],
    requiredArtifactSchemas: [...(report.checks.artifacts.requiredArtifactSchemas ?? [])],
    missingArtifactSchemas: [...(report.checks.artifacts.missingArtifactSchemas ?? [])],
    requiredArtifactKinds: [...(report.checks.artifacts.requiredArtifactKinds ?? [])],
    missingArtifactKinds: [...(report.checks.artifacts.missingArtifactKinds ?? [])],
    requiredArtifactGeneratedEvidenceKinds: [...(report.checks.artifacts.requiredArtifactGeneratedEvidenceKinds ?? [])],
    missingArtifactGeneratedEvidenceKinds: [...(report.checks.artifacts.missingArtifactGeneratedEvidenceKinds ?? [])],
    requiredArtifactGeneratedEvidenceRepos: [...(report.checks.artifacts.requiredArtifactGeneratedEvidenceRepos ?? [])],
    missingArtifactGeneratedEvidenceRepos: [...(report.checks.artifacts.missingArtifactGeneratedEvidenceRepos ?? [])],
    requiredArtifactGeneratedMatrixArtifactIndexes: [...(report.checks.artifacts.requiredArtifactGeneratedMatrixArtifactIndexes ?? [])],
    missingArtifactGeneratedMatrixArtifactIndexes: [...(report.checks.artifacts.missingArtifactGeneratedMatrixArtifactIndexes ?? [])],
    requiredArtifactGeneratedMatrixLimitations: [...(report.checks.artifacts.requiredArtifactGeneratedMatrixLimitations ?? [])],
    missingArtifactGeneratedMatrixLimitations: [...(report.checks.artifacts.missingArtifactGeneratedMatrixLimitations ?? [])],
    requiredArtifactGeneratedMatrixNames: [...(report.checks.artifacts.requiredArtifactGeneratedMatrixNames ?? [])],
    missingArtifactGeneratedMatrixNames: [...(report.checks.artifacts.missingArtifactGeneratedMatrixNames ?? [])],
    requiredArtifactGeneratedMatrixRepos: [...(report.checks.artifacts.requiredArtifactGeneratedMatrixRepos ?? [])],
    missingArtifactGeneratedMatrixRepos: [...(report.checks.artifacts.missingArtifactGeneratedMatrixRepos ?? [])],
    requiredArtifactGeneratedValidationSuiteArtifactIndexes: [...(report.checks.artifacts.requiredArtifactGeneratedValidationSuiteArtifactIndexes ?? [])],
    missingArtifactGeneratedValidationSuiteArtifactIndexes: [...(report.checks.artifacts.missingArtifactGeneratedValidationSuiteArtifactIndexes ?? [])],
    requiredArtifactEvidenceRepos: [...(report.checks.artifacts.requiredArtifactEvidenceRepos ?? [])],
    missingArtifactEvidenceRepos: [...(report.checks.artifacts.missingArtifactEvidenceRepos ?? [])],
    requiredArtifactProviderAccountAliases: [...(report.checks.artifacts.requiredArtifactProviderAccountAliases ?? [])],
    missingArtifactProviderAccountAliases: [...(report.checks.artifacts.missingArtifactProviderAccountAliases ?? [])],
    requiredArtifactValidationPresets: [...(report.checks.artifacts.requiredArtifactValidationPresets ?? [])],
    missingArtifactValidationPresets: [...(report.checks.artifacts.missingArtifactValidationPresets ?? [])],
    requiredArtifactRuntimeAuthorityInvariants: [...(report.checks.artifacts.requiredArtifactRuntimeAuthorityInvariants ?? [])],
    missingArtifactRuntimeAuthorityInvariants: [...(report.checks.artifacts.missingArtifactRuntimeAuthorityInvariants ?? [])],
    requiredArtifactRuntimeSignals: [...(report.checks.artifacts.requiredArtifactRuntimeSignals ?? [])],
    missingArtifactRuntimeSignals: [...(report.checks.artifacts.missingArtifactRuntimeSignals ?? [])],
    requiredArtifactRuntimeSignalOwners: [...(report.checks.artifacts.requiredArtifactRuntimeSignalOwners ?? [])],
    missingArtifactRuntimeSignalOwners: [...(report.checks.artifacts.missingArtifactRuntimeSignalOwners ?? [])],
    requiredArtifactOwners: [...(report.checks.artifacts.requiredArtifactOwners ?? [])],
    missingArtifactOwners: [...(report.checks.artifacts.missingArtifactOwners ?? [])],
    requiredArtifactClassifications: [...(report.checks.artifacts.requiredArtifactClassifications ?? [])],
    missingArtifactClassifications: [...(report.checks.artifacts.missingArtifactClassifications ?? [])],
    requiredArtifactFailureClassifications: [...(report.checks.artifacts.requiredArtifactFailureClassifications ?? [])],
    missingArtifactFailureClassifications: [...(report.checks.artifacts.missingArtifactFailureClassifications ?? [])],
    requiredArtifactPlannedOwners: [...(report.checks.artifacts.requiredArtifactPlannedOwners ?? [])],
    missingArtifactPlannedOwners: [...(report.checks.artifacts.missingArtifactPlannedOwners ?? [])],
    requiredArtifactPlannedClassifications: [...(report.checks.artifacts.requiredArtifactPlannedClassifications ?? [])],
    missingArtifactPlannedClassifications: [...(report.checks.artifacts.missingArtifactPlannedClassifications ?? [])],
    requiredArtifactExitCriterionStatuses: [...(report.checks.artifacts.requiredArtifactExitCriterionStatuses ?? [])],
    missingArtifactExitCriterionStatuses: [...(report.checks.artifacts.missingArtifactExitCriterionStatuses ?? [])],
    requiredArtifactIncompleteExitCriterionStatuses: [...(report.checks.artifacts.requiredArtifactIncompleteExitCriterionStatuses ?? [])],
    missingArtifactIncompleteExitCriterionStatuses: [...(report.checks.artifacts.missingArtifactIncompleteExitCriterionStatuses ?? [])],
    schemas: { ...(report.checks.artifacts.aggregate?.schemas ?? {}) },
    coverageAreas: { ...(report.checks.artifacts.aggregate?.coverageAreas ?? {}) },
    runtimeAuthorityInvariants: { ...(report.checks.artifacts.aggregate?.runtimeAuthorityInvariants ?? {}) },
    runtimeSignals: { ...(report.checks.artifacts.aggregate?.runtimeSignals ?? {}) },
    runtimeSignalOwners: { ...(report.checks.artifacts.aggregate?.runtimeSignalOwners ?? {}) },
    owners: { ...(report.checks.artifacts.aggregate?.owners ?? {}) },
    classifications: { ...(report.checks.artifacts.aggregate?.classifications ?? {}) },
    failureClassifications: { ...(report.checks.artifacts.aggregate?.requiredFailureClassifications ?? {}) },
    plannedOwners: { ...(report.checks.artifacts.aggregate?.plannedOwners ?? {}) },
    plannedClassifications: { ...(report.checks.artifacts.aggregate?.plannedClassifications ?? {}) },
    exitCriterionStatuses: { ...(report.checks.artifacts.aggregate?.exitCriterionStatuses ?? {}) },
    incompleteExitCriterionStatuses: { ...(report.checks.artifacts.aggregate?.incompleteExitCriterionStatuses ?? {}) },
    artifactKinds: { ...(report.checks.artifacts.aggregate?.artifactKinds ?? {}) },
    generatedEvidenceKinds: { ...(report.checks.artifacts.aggregate?.generatedEvidenceKinds ?? {}) },
    generatedEvidenceRepos: { ...(report.checks.artifacts.aggregate?.generatedEvidenceRepos ?? {}) },
    generatedMatrixArtifactIndexes: { ...(report.checks.artifacts.aggregate?.generatedMatrixArtifactIndexes ?? {}) },
    generatedMatrixLimitations: { ...(report.checks.artifacts.aggregate?.generatedMatrixLimitations ?? {}) },
    generatedMatrixNames: { ...(report.checks.artifacts.aggregate?.generatedMatrixNames ?? {}) },
    generatedMatrixRepos: { ...(report.checks.artifacts.aggregate?.generatedMatrixRepos ?? {}) },
    generatedValidationSuiteArtifactIndexes: { ...(report.checks.artifacts.aggregate?.generatedValidationSuiteArtifactIndexes ?? {}) },
    generatedValidationSuiteFailureRoots: { ...(report.checks.artifacts.aggregate?.generatedValidationSuiteFailureRoots ?? {}) },
    evidenceRepos: { ...(report.checks.artifacts.aggregate?.evidenceRepos ?? {}) },
    providerAccountAliases: { ...(report.checks.artifacts.aggregate?.providerAccountAliases ?? {}) },
    validationPresets: { ...(report.checks.artifacts.aggregate?.validationPresets ?? {}) },
    artifactCoverageInputSources: { ...(report.checks.artifacts.aggregate?.artifactCoverageInputSources ?? {}) },
  }
}

export function countValidationGateArtifactCoverage(coverage, artifactCoverage) {
  countStringValues(coverage.requiredArtifactSchemas, artifactCoverage.requiredArtifactSchemas)
  countStringValues(coverage.missingArtifactSchemas, artifactCoverage.missingArtifactSchemas)
  countStringValues(coverage.requiredArtifactKinds, artifactCoverage.requiredArtifactKinds)
  countStringValues(coverage.missingArtifactKinds, artifactCoverage.missingArtifactKinds)
  countStringValues(coverage.requiredArtifactGeneratedEvidenceKinds, artifactCoverage.requiredArtifactGeneratedEvidenceKinds)
  countStringValues(coverage.missingArtifactGeneratedEvidenceKinds, artifactCoverage.missingArtifactGeneratedEvidenceKinds)
  countStringValues(coverage.requiredArtifactGeneratedEvidenceRepos, artifactCoverage.requiredArtifactGeneratedEvidenceRepos)
  countStringValues(coverage.missingArtifactGeneratedEvidenceRepos, artifactCoverage.missingArtifactGeneratedEvidenceRepos)
  countStringValues(coverage.requiredArtifactGeneratedMatrixArtifactIndexes, artifactCoverage.requiredArtifactGeneratedMatrixArtifactIndexes)
  countStringValues(coverage.missingArtifactGeneratedMatrixArtifactIndexes, artifactCoverage.missingArtifactGeneratedMatrixArtifactIndexes)
  countStringValues(coverage.requiredArtifactGeneratedMatrixLimitations, artifactCoverage.requiredArtifactGeneratedMatrixLimitations)
  countStringValues(coverage.missingArtifactGeneratedMatrixLimitations, artifactCoverage.missingArtifactGeneratedMatrixLimitations)
  countStringValues(coverage.requiredArtifactGeneratedMatrixNames, artifactCoverage.requiredArtifactGeneratedMatrixNames)
  countStringValues(coverage.missingArtifactGeneratedMatrixNames, artifactCoverage.missingArtifactGeneratedMatrixNames)
  countStringValues(coverage.requiredArtifactGeneratedMatrixRepos, artifactCoverage.requiredArtifactGeneratedMatrixRepos)
  countStringValues(coverage.missingArtifactGeneratedMatrixRepos, artifactCoverage.missingArtifactGeneratedMatrixRepos)
  countStringValues(coverage.requiredArtifactGeneratedValidationSuiteArtifactIndexes, artifactCoverage.requiredArtifactGeneratedValidationSuiteArtifactIndexes ?? [])
  countStringValues(coverage.missingArtifactGeneratedValidationSuiteArtifactIndexes, artifactCoverage.missingArtifactGeneratedValidationSuiteArtifactIndexes ?? [])
  countStringValues(coverage.requiredArtifactGeneratedValidationSuiteFailureRoots, artifactCoverage.requiredArtifactGeneratedValidationSuiteFailureRoots ?? [])
  countStringValues(coverage.missingArtifactGeneratedValidationSuiteFailureRoots, artifactCoverage.missingArtifactGeneratedValidationSuiteFailureRoots ?? [])
  countStringValues(coverage.requiredArtifactEvidenceRepos, artifactCoverage.requiredArtifactEvidenceRepos)
  countStringValues(coverage.missingArtifactEvidenceRepos, artifactCoverage.missingArtifactEvidenceRepos)
  countStringValues(coverage.requiredArtifactProviderAccountAliases, artifactCoverage.requiredArtifactProviderAccountAliases)
  countStringValues(coverage.missingArtifactProviderAccountAliases, artifactCoverage.missingArtifactProviderAccountAliases)
  countStringValues(coverage.requiredArtifactValidationPresets, artifactCoverage.requiredArtifactValidationPresets)
  countStringValues(coverage.missingArtifactValidationPresets, artifactCoverage.missingArtifactValidationPresets)
  countStringValues(coverage.requiredArtifactRuntimeAuthorityInvariants, artifactCoverage.requiredArtifactRuntimeAuthorityInvariants)
  countStringValues(coverage.missingArtifactRuntimeAuthorityInvariants, artifactCoverage.missingArtifactRuntimeAuthorityInvariants)
  countStringValues(coverage.requiredArtifactRuntimeSignals, artifactCoverage.requiredArtifactRuntimeSignals)
  countStringValues(coverage.missingArtifactRuntimeSignals, artifactCoverage.missingArtifactRuntimeSignals)
  countStringValues(coverage.requiredArtifactRuntimeSignalOwners, artifactCoverage.requiredArtifactRuntimeSignalOwners)
  countStringValues(coverage.missingArtifactRuntimeSignalOwners, artifactCoverage.missingArtifactRuntimeSignalOwners)
  countStringValues(coverage.requiredArtifactOwners, artifactCoverage.requiredArtifactOwners)
  countStringValues(coverage.missingArtifactOwners, artifactCoverage.missingArtifactOwners)
  countStringValues(coverage.requiredArtifactClassifications, artifactCoverage.requiredArtifactClassifications)
  countStringValues(coverage.missingArtifactClassifications, artifactCoverage.missingArtifactClassifications)
  countStringValues(coverage.requiredArtifactFailureClassifications, artifactCoverage.requiredArtifactFailureClassifications)
  countStringValues(coverage.missingArtifactFailureClassifications, artifactCoverage.missingArtifactFailureClassifications)
  countStringValues(coverage.requiredArtifactPlannedOwners, artifactCoverage.requiredArtifactPlannedOwners)
  countStringValues(coverage.missingArtifactPlannedOwners, artifactCoverage.missingArtifactPlannedOwners)
  countStringValues(coverage.requiredArtifactPlannedClassifications, artifactCoverage.requiredArtifactPlannedClassifications)
  countStringValues(coverage.missingArtifactPlannedClassifications, artifactCoverage.missingArtifactPlannedClassifications)
  countStringValues(coverage.requiredArtifactExitCriterionStatuses, artifactCoverage.requiredArtifactExitCriterionStatuses)
  countStringValues(coverage.missingArtifactExitCriterionStatuses, artifactCoverage.missingArtifactExitCriterionStatuses)
  countStringValues(coverage.requiredArtifactIncompleteExitCriterionStatuses, artifactCoverage.requiredArtifactIncompleteExitCriterionStatuses)
  countStringValues(coverage.missingArtifactIncompleteExitCriterionStatuses, artifactCoverage.missingArtifactIncompleteExitCriterionStatuses)
  countStringValues(coverage.requiredArtifactCoverageAreas, artifactCoverage.requiredArtifactCoverageAreas)
  countStringValues(coverage.missingArtifactCoverageAreas, artifactCoverage.missingArtifactCoverageAreas)
  countObjectValues(coverage.artifactSchemas, artifactCoverage.schemas)
  countObjectValues(coverage.artifactCoverageAreas, artifactCoverage.coverageAreas)
  countObjectValues(coverage.artifactRuntimeAuthorityInvariants, artifactCoverage.runtimeAuthorityInvariants)
  countObjectValues(coverage.artifactRuntimeSignals, artifactCoverage.runtimeSignals)
  countObjectValues(coverage.artifactRuntimeSignalOwners, artifactCoverage.runtimeSignalOwners)
  countObjectValues(coverage.artifactOwners, artifactCoverage.owners)
  countObjectValues(coverage.artifactClassifications, artifactCoverage.classifications)
  countObjectValues(coverage.artifactFailureClassifications, artifactCoverage.failureClassifications)
  countObjectValues(coverage.artifactPlannedOwners, artifactCoverage.plannedOwners)
  countObjectValues(coverage.artifactPlannedClassifications, artifactCoverage.plannedClassifications)
  countObjectValues(coverage.artifactExitCriterionStatuses, artifactCoverage.exitCriterionStatuses)
  countObjectValues(coverage.artifactIncompleteExitCriterionStatuses, artifactCoverage.incompleteExitCriterionStatuses)
  countObjectValues(coverage.artifactKinds, artifactCoverage.artifactKinds)
  countObjectValues(coverage.artifactGeneratedEvidenceKinds, artifactCoverage.generatedEvidenceKinds)
  countObjectValues(coverage.artifactGeneratedEvidenceRepos, artifactCoverage.generatedEvidenceRepos)
  countObjectValues(coverage.artifactGeneratedMatrixArtifactIndexes, artifactCoverage.generatedMatrixArtifactIndexes)
  countObjectValues(coverage.artifactGeneratedMatrixLimitations, artifactCoverage.generatedMatrixLimitations)
  countObjectValues(coverage.artifactGeneratedMatrixNames, artifactCoverage.generatedMatrixNames)
  countObjectValues(coverage.artifactGeneratedMatrixRepos, artifactCoverage.generatedMatrixRepos)
  countObjectValues(coverage.artifactGeneratedValidationSuiteArtifactIndexes, artifactCoverage.generatedValidationSuiteArtifactIndexes)
  countObjectValues(coverage.artifactGeneratedValidationSuiteFailureRoots, artifactCoverage.generatedValidationSuiteFailureRoots)
  countObjectValues(coverage.artifactEvidenceRepos, artifactCoverage.evidenceRepos)
  countObjectValues(coverage.artifactProviderAccountAliases, artifactCoverage.providerAccountAliases)
  countObjectValues(coverage.artifactValidationPresets, artifactCoverage.validationPresets)
  countObjectValues(coverage.artifactCoverageInputSources, artifactCoverage.artifactCoverageInputSources)
}

export function validationGateReportFailureCoverage(report) {
  const failures = report.checks.failures
  const runtimeSignals = { ...(failures.aggregate?.runtimeSignals ?? {}) }
  return {
    runtimeSignals,
    runtimeSignalOwners: drillRuntimeSignalOwnerCounts(runtimeSignals),
    owners: { ...(failures.aggregate?.owners ?? {}) },
    classifications: { ...(failures.aggregate?.classifications ?? {}) },
    staleFailureManifests: [...(failures.staleFailureManifests ?? [])],
  }
}

export function validationGateReportMatrixCoverage(report) {
  const matrices = report.checks.matrices
  const runtimeSignals = { ...(matrices.aggregate?.runtimeSignals ?? {}) }
  const runtimeSignalScenarios = cloneRuntimeSignalScenarios(matrices.aggregate?.runtimeSignalScenarios)
  return {
    runtimeSignals,
    runtimeSignalOwners: drillRuntimeSignalOwnerCounts(runtimeSignals),
    owners: { ...(matrices.aggregate?.owners ?? {}) },
    classifications: { ...(matrices.aggregate?.classifications ?? {}) },
    staleMatrixReports: [...(matrices.staleMatrixReports ?? [])],
    requiredMatrices: [...(matrices.requiredMatrices ?? [])],
    missingMatrices: [...(matrices.missingMatrices ?? [])],
    requiredMatrixClassifications: [...(matrices.requiredMatrixClassifications ?? [])],
    missingMatrixClassifications: [...(matrices.missingMatrixClassifications ?? [])],
    requiredMatrixRuntimeSignals: [...(matrices.requiredMatrixRuntimeSignals ?? [])],
    missingMatrixRuntimeSignals: [...(matrices.missingMatrixRuntimeSignals ?? [])],
    requiredDeploymentPresets: [...(matrices.requiredDeploymentPresets ?? [])],
    missingDeploymentPresets: [...(matrices.missingDeploymentPresets ?? [])],
    requiredProviders: [...(matrices.requiredProviders ?? [])],
    missingProviders: [...(matrices.missingProviders ?? [])],
    requiredScenarios: [...(matrices.requiredScenarios ?? [])],
    missingScenarios: [...(matrices.missingScenarios ?? [])],
    ...(Object.keys(runtimeSignalScenarios).length > 0 ? { runtimeSignalScenarios } : {}),
  }
}

export function countStringValues(counts, values) {
  for (const value of values) {
    counts.set(value, (counts.get(value) ?? 0) + 1)
  }
}

export function countObjectValues(counts, values) {
  for (const [value, count] of Object.entries(values ?? {})) {
    counts.set(value, (counts.get(value) ?? 0) + count)
  }
}

export function staleMatrixReportSourceLabels(staleMatrixReports) {
  return (staleMatrixReports ?? []).map((report) => report.source ?? "unknown")
}

export function staleFailureManifestSourceLabels(staleFailureManifests) {
  return (staleFailureManifests ?? []).map((manifest) => manifest.source ?? "unknown")
}

export function missingValidationGateAggregateRequirements(coverage, requirements) {
  return {
    missingPresets: missingCoverageRequirements(coverage.presets, requirements.requiredPresets ?? []),
    missingPlatformCoverageAreas: missingCoverageRequirements(coverage.requiredPlatformCoverageAreas, requirements.requiredPlatformCoverageAreas ?? []),
    missingArtifactCoverageAreas: missingCoverageRequirements(coverage.artifactCoverageAreas, requirements.requiredArtifactCoverageAreas ?? []),
    missingArtifactSchemas: missingCoverageRequirements(coverage.artifactSchemas, requirements.requiredArtifactSchemas ?? []),
    missingArtifactKinds: missingCoverageRequirements(coverage.artifactKinds, requirements.requiredArtifactKinds ?? []),
    missingArtifactGeneratedEvidenceKinds: missingCoverageRequirements(coverage.artifactGeneratedEvidenceKinds, requirements.requiredArtifactGeneratedEvidenceKinds ?? []),
    missingArtifactGeneratedEvidenceRepos: missingCoverageRequirements(coverage.artifactGeneratedEvidenceRepos, requirements.requiredArtifactGeneratedEvidenceRepos ?? []),
    missingArtifactGeneratedMatrixArtifactIndexes: missingCoverageRequirements(coverage.artifactGeneratedMatrixArtifactIndexes, requirements.requiredArtifactGeneratedMatrixArtifactIndexes ?? []),
    missingArtifactGeneratedMatrixLimitations: missingCoverageRequirements(coverage.artifactGeneratedMatrixLimitations, requirements.requiredArtifactGeneratedMatrixLimitations ?? []),
    missingArtifactGeneratedMatrixNames: missingCoverageRequirements(coverage.artifactGeneratedMatrixNames, requirements.requiredArtifactGeneratedMatrixNames ?? []),
    missingArtifactGeneratedMatrixRepos: missingCoverageRequirements(coverage.artifactGeneratedMatrixRepos, requirements.requiredArtifactGeneratedMatrixRepos ?? []),
    missingArtifactGeneratedValidationSuiteArtifactIndexes: missingCoverageRequirements(coverage.artifactGeneratedValidationSuiteArtifactIndexes, requirements.requiredArtifactGeneratedValidationSuiteArtifactIndexes ?? []),
    missingArtifactGeneratedValidationSuiteFailureRoots: missingCoverageRequirements(coverage.artifactGeneratedValidationSuiteFailureRoots, requirements.requiredArtifactGeneratedValidationSuiteFailureRoots ?? []),
    missingArtifactEvidenceRepos: missingCoverageRequirements(coverage.artifactEvidenceRepos, requirements.requiredArtifactEvidenceRepos ?? []),
    missingArtifactProviderAccountAliases: missingCoverageRequirements(coverage.artifactProviderAccountAliases, requirements.requiredArtifactProviderAccountAliases ?? []),
    missingArtifactValidationPresets: missingCoverageRequirements(coverage.artifactValidationPresets, requirements.requiredArtifactValidationPresets ?? []),
    missingArtifactRuntimeAuthorityInvariants: missingCoverageRequirements(coverage.artifactRuntimeAuthorityInvariants, requirements.requiredArtifactRuntimeAuthorityInvariants ?? []),
    missingArtifactRuntimeSignals: missingCoverageRequirements(coverage.artifactRuntimeSignals, requirements.requiredArtifactRuntimeSignals ?? []),
    missingArtifactRuntimeSignalOwners: missingCoverageRequirements(coverage.artifactRuntimeSignalOwners, requirements.requiredArtifactRuntimeSignalOwners ?? []),
    missingArtifactOwners: missingCoverageRequirements(coverage.artifactOwners, requirements.requiredArtifactOwners ?? []),
    missingArtifactClassifications: missingCoverageRequirements(coverage.artifactClassifications, requirements.requiredArtifactClassifications ?? []),
    missingArtifactFailureClassifications: missingCoverageRequirements(coverage.artifactFailureClassifications, requirements.requiredArtifactFailureClassifications ?? []),
    missingArtifactPlannedOwners: missingCoverageRequirements(coverage.artifactPlannedOwners, requirements.requiredArtifactPlannedOwners ?? []),
    missingArtifactPlannedClassifications: missingCoverageRequirements(coverage.artifactPlannedClassifications, requirements.requiredArtifactPlannedClassifications ?? []),
    missingArtifactExitCriterionStatuses: missingCoverageRequirements(coverage.artifactExitCriterionStatuses, requirements.requiredArtifactExitCriterionStatuses ?? []),
    missingArtifactIncompleteExitCriterionStatuses: missingCoverageRequirements(coverage.artifactIncompleteExitCriterionStatuses, requirements.requiredArtifactIncompleteExitCriterionStatuses ?? []),
    missingRuntimeSignals: missingCoverageRequirements(coverage.requiredRuntimeSignals, requirements.requiredRuntimeSignals ?? []),
    missingRuntimeSignalOwners: missingCoverageRequirements(coverage.requiredRuntimeSignalOwners, requirements.requiredRuntimeSignalOwners ?? []),
    missingFailureClassifications: missingCoverageRequirements(coverage.requiredFailureClassifications, requirements.requiredFailureClassifications ?? []),
    missingMatrices: missingCoverageRequirements(coverage.requiredMatrices, requirements.requiredMatrices ?? []),
    missingMatrixClassifications: missingCoverageRequirements(coverage.requiredMatrixClassifications, requirements.requiredMatrixClassifications ?? []),
    missingMatrixRuntimeSignals: missingCoverageRequirements(coverage.requiredMatrixRuntimeSignals, requirements.requiredMatrixRuntimeSignals ?? []),
    missingDeploymentPresets: missingCoverageRequirements(coverage.requiredDeploymentPresets, requirements.requiredDeploymentPresets ?? []),
    missingProviders: missingCoverageRequirements(coverage.requiredProviders, requirements.requiredProviders ?? []),
    missingScenarios: missingCoverageRequirements(coverage.requiredScenarios, requirements.requiredScenarios ?? []),
    missingGeneratedEvidenceKinds: missingCoverageRequirements(coverage.generatedEvidenceKinds, requirements.requiredGeneratedEvidenceKinds ?? []),
    missingGeneratedMatrixArtifactIndexes: missingCoverageRequirements(coverage.generatedMatrixArtifactIndexes, requirements.requiredGeneratedMatrixArtifactIndexes ?? []),
    missingGeneratedMatrixLimitations: missingCoverageRequirements(coverage.generatedMatrixLimitations, requirements.requiredGeneratedMatrixLimitations ?? []),
    missingGeneratedValidationSuiteArtifactIndexes: missingCoverageRequirements(coverage.generatedValidationSuiteArtifactIndexes, requirements.requiredGeneratedValidationSuiteArtifactIndexes ?? []),
    missingGeneratedValidationSuiteFailureRoots: missingCoverageRequirements(coverage.generatedValidationSuiteFailureRoots, requirements.requiredGeneratedValidationSuiteFailureRoots ?? []),
  }
}

export function missingCoverageRequirements(counts, required) {
  const present = new Set(Object.keys(counts ?? {}))
  return required.filter((entry) => !present.has(entry))
}

export function appendMissingValidationGateAggregateNextActions(nextActions, missing) {
  const specs = [
    ["missingPlatformCoverageAreas", "platform-bundle", "provide validation gate reports requiring platform coverage areas"],
    ["missingArtifactCoverageAreas", "artifact-coverage", "provide validation gate reports with artifact coverage areas"],
    ["missingArtifactKinds", "artifact-coverage", "provide validation gate reports with artifact kinds"],
    ["missingArtifactGeneratedEvidenceKinds", "generated-evidence", "provide validation gate artifact indexes with generated evidence kinds"],
    ["missingArtifactGeneratedEvidenceRepos", "generated-evidence", "provide validation gate artifact indexes with generated evidence repos"],
    ["missingArtifactGeneratedMatrixArtifactIndexes", "generated-evidence", "provide validation gate artifact indexes with generated matrix artifact indexes"],
    ["missingArtifactGeneratedMatrixLimitations", "generated-evidence", "provide validation gate artifact indexes with generated matrix limitations"],
    ["missingArtifactGeneratedMatrixNames", "generated-evidence", "provide validation gate artifact indexes with generated matrix names"],
    ["missingArtifactGeneratedMatrixRepos", "generated-evidence", "provide validation gate artifact indexes with generated matrix repos"],
    ["missingArtifactGeneratedValidationSuiteArtifactIndexes", "generated-evidence", "provide validation gate artifact indexes with generated validation-suite artifact indexes"],
    ["missingArtifactGeneratedValidationSuiteFailureRoots", "generated-evidence", "provide validation gate artifact indexes with generated validation-suite failure roots"],
    ["missingArtifactEvidenceRepos", "artifact-coverage", "provide validation gate reports with artifact evidence repos"],
    ["missingArtifactProviderAccountAliases", "artifact-coverage", "provide validation gate reports with artifact provider account aliases"],
    ["missingArtifactValidationPresets", "artifact-coverage", "provide validation gate reports with artifact validation presets"],
    ["missingArtifactRuntimeAuthorityInvariants", "artifact-coverage", "provide validation gate reports with artifact runtime authority invariants"],
    ["missingArtifactRuntimeSignals", "artifact-coverage", "provide validation gate reports with artifact runtime signals"],
    ["missingArtifactRuntimeSignalOwners", "artifact-coverage", "provide validation gate reports with artifact runtime signal owners"],
    ["missingArtifactOwners", "artifact-coverage", "provide validation gate reports with artifact owners"],
    ["missingArtifactClassifications", "artifact-coverage", "provide validation gate reports with artifact classifications"],
    ["missingArtifactFailureClassifications", "artifact-coverage", "provide validation gate reports with artifact failure classifications"],
    ["missingArtifactPlannedOwners", "artifact-coverage", "provide validation gate reports with artifact planned owners"],
    ["missingArtifactPlannedClassifications", "artifact-coverage", "provide validation gate reports with artifact planned classifications"],
    ["missingArtifactExitCriterionStatuses", "artifact-coverage", "provide validation gate reports with artifact exit criterion statuses"],
    ["missingArtifactIncompleteExitCriterionStatuses", "artifact-coverage", "provide validation gate reports with artifact incomplete exit criterion statuses"],
    ["missingRuntimeSignals", "platform-bundle", "provide validation gate reports requiring runtime signals"],
    ["missingRuntimeSignalOwners", "platform-bundle", "provide validation gate reports requiring runtime signal owners"],
    ["missingFailureClassifications", "platform-bundle", "provide validation gate reports requiring failure classifications"],
    ["missingMatrices", "matrix-coverage", "provide validation gate reports requiring matrices"],
    ["missingMatrixClassifications", "matrix-coverage", "provide validation gate reports requiring matrix classifications"],
    ["missingMatrixRuntimeSignals", "matrix-coverage", "provide validation gate reports requiring matrix runtime signals"],
    ["missingDeploymentPresets", "matrix-coverage", "provide validation gate reports requiring deployment presets"],
    ["missingProviders", "matrix-coverage", "provide validation gate reports requiring providers"],
    ["missingScenarios", "matrix-coverage", "provide validation gate reports requiring scenarios"],
    ["missingGeneratedEvidenceKinds", "generated-evidence", "provide validation gate reports with generated evidence kinds"],
    ["missingGeneratedMatrixArtifactIndexes", "generated-evidence", "provide validation gate reports with generated matrix artifact indexes"],
    ["missingGeneratedMatrixLimitations", "generated-evidence", "provide validation gate reports with generated matrix limitations"],
    ["missingGeneratedValidationSuiteArtifactIndexes", "generated-evidence", "provide validation gate reports with generated validation-suite artifact indexes"],
    ["missingGeneratedValidationSuiteFailureRoots", "generated-evidence", "provide validation gate reports with generated validation-suite failure roots"],
  ]
  for (const [key, classification, prefix] of specs) {
    if ((missing[key] ?? []).length > 0) {
      countDrillAggregateNextAction(nextActions, {
        owner: "validation-harness",
        classification,
        nextAction: `${prefix}: ${missing[key].join(", ")}`,
      })
    }
  }
  appendMissingArtifactSchemaNextActions(nextActions, missing.missingArtifactSchemas ?? [])
}

export function appendMissingArtifactSchemaNextActions(nextActions, missingArtifactSchemas) {
  if (missingArtifactSchemas.includes("arroba.drill.validation_suite_run.v1")) {
    countDrillAggregateNextAction(nextActions, {
      owner: "validation-harness",
      classification: "artifact-coverage",
      nextAction: "run an executable validation suite with --run-json --output PATH --output-artifact-index PATH, then rerun the validation gate aggregate",
    })
  }
  const remainingSchemas = missingArtifactSchemas.filter((schema) => schema !== "arroba.drill.validation_suite_run.v1")
  if (remainingSchemas.length > 0) {
    countDrillAggregateNextAction(nextActions, {
      owner: "validation-harness",
      classification: "artifact-coverage",
      nextAction: `provide validation gate reports with artifact schemas: ${remainingSchemas.join(", ")}`,
    })
  }
}

export function assertValidationGateAggregateMissingRequirementsMatch(aggregate, expected, source) {
  const fields = [
    "missingPresets",
    "missingPlatformCoverageAreas",
    "missingArtifactCoverageAreas",
    "missingArtifactSchemas",
    "missingArtifactKinds",
    "missingArtifactGeneratedEvidenceKinds",
    "missingArtifactGeneratedEvidenceRepos",
    "missingArtifactGeneratedMatrixArtifactIndexes",
    "missingArtifactGeneratedMatrixLimitations",
    "missingArtifactGeneratedMatrixNames",
    "missingArtifactGeneratedMatrixRepos",
    "missingArtifactEvidenceRepos",
    "missingArtifactProviderAccountAliases",
    "missingArtifactValidationPresets",
    "missingArtifactRuntimeAuthorityInvariants",
    "missingArtifactRuntimeSignals",
    "missingArtifactRuntimeSignalOwners",
    "missingArtifactOwners",
    "missingArtifactClassifications",
    "missingArtifactPlannedOwners",
    "missingArtifactPlannedClassifications",
    "missingArtifactExitCriterionStatuses",
    "missingArtifactIncompleteExitCriterionStatuses",
    "missingRuntimeSignals",
    "missingFailureClassifications",
    "missingMatrices",
    "missingMatrixClassifications",
    "missingMatrixRuntimeSignals",
    "missingDeploymentPresets",
    "missingProviders",
    "missingScenarios",
    "missingGeneratedEvidenceKinds",
    "missingGeneratedMatrixArtifactIndexes",
    "missingGeneratedMatrixLimitations",
    "missingGeneratedValidationSuiteArtifactIndexes",
    "missingGeneratedValidationSuiteFailureRoots",
  ]
  for (const field of fields) {
    if (JSON.stringify(aggregate[field] ?? []) !== JSON.stringify(expected[field] ?? [])) {
      throw new Error(`${source} ${field} does not match reports`)
    }
  }
}

export function formatValidationGateCoverageCounts(coverage) {
  return {
    presets: countMapToObject(coverage.presets),
    requiredPlatformCoverageAreas: countMapToObject(coverage.requiredPlatformCoverageAreas),
    missingPlatformCoverageAreas: countMapToObject(coverage.missingPlatformCoverageAreas),
    requiredArtifactCoverageAreas: countMapToObject(coverage.requiredArtifactCoverageAreas),
    missingArtifactCoverageAreas: countMapToObject(coverage.missingArtifactCoverageAreas),
    requiredArtifactSchemas: countMapToObject(coverage.requiredArtifactSchemas),
    missingArtifactSchemas: countMapToObject(coverage.missingArtifactSchemas),
    requiredArtifactKinds: countMapToObject(coverage.requiredArtifactKinds),
    missingArtifactKinds: countMapToObject(coverage.missingArtifactKinds),
    requiredArtifactGeneratedEvidenceKinds: countMapToObject(coverage.requiredArtifactGeneratedEvidenceKinds),
    missingArtifactGeneratedEvidenceKinds: countMapToObject(coverage.missingArtifactGeneratedEvidenceKinds),
    requiredArtifactGeneratedEvidenceRepos: countMapToObject(coverage.requiredArtifactGeneratedEvidenceRepos),
    missingArtifactGeneratedEvidenceRepos: countMapToObject(coverage.missingArtifactGeneratedEvidenceRepos),
    requiredArtifactGeneratedMatrixArtifactIndexes: countMapToObject(coverage.requiredArtifactGeneratedMatrixArtifactIndexes),
    missingArtifactGeneratedMatrixArtifactIndexes: countMapToObject(coverage.missingArtifactGeneratedMatrixArtifactIndexes),
    requiredArtifactGeneratedMatrixLimitations: countMapToObject(coverage.requiredArtifactGeneratedMatrixLimitations),
    missingArtifactGeneratedMatrixLimitations: countMapToObject(coverage.missingArtifactGeneratedMatrixLimitations),
    requiredArtifactGeneratedMatrixNames: countMapToObject(coverage.requiredArtifactGeneratedMatrixNames),
    missingArtifactGeneratedMatrixNames: countMapToObject(coverage.missingArtifactGeneratedMatrixNames),
    requiredArtifactGeneratedMatrixRepos: countMapToObject(coverage.requiredArtifactGeneratedMatrixRepos),
    missingArtifactGeneratedMatrixRepos: countMapToObject(coverage.missingArtifactGeneratedMatrixRepos),
    requiredArtifactGeneratedValidationSuiteArtifactIndexes: countMapToObject(coverage.requiredArtifactGeneratedValidationSuiteArtifactIndexes),
    missingArtifactGeneratedValidationSuiteArtifactIndexes: countMapToObject(coverage.missingArtifactGeneratedValidationSuiteArtifactIndexes),
    requiredArtifactGeneratedValidationSuiteFailureRoots: countMapToObject(coverage.requiredArtifactGeneratedValidationSuiteFailureRoots),
    missingArtifactGeneratedValidationSuiteFailureRoots: countMapToObject(coverage.missingArtifactGeneratedValidationSuiteFailureRoots),
    requiredArtifactEvidenceRepos: countMapToObject(coverage.requiredArtifactEvidenceRepos),
    missingArtifactEvidenceRepos: countMapToObject(coverage.missingArtifactEvidenceRepos),
    requiredArtifactProviderAccountAliases: countMapToObject(coverage.requiredArtifactProviderAccountAliases),
    missingArtifactProviderAccountAliases: countMapToObject(coverage.missingArtifactProviderAccountAliases),
    requiredArtifactValidationPresets: countMapToObject(coverage.requiredArtifactValidationPresets),
    missingArtifactValidationPresets: countMapToObject(coverage.missingArtifactValidationPresets),
    requiredArtifactRuntimeAuthorityInvariants: countMapToObject(coverage.requiredArtifactRuntimeAuthorityInvariants),
    missingArtifactRuntimeAuthorityInvariants: countMapToObject(coverage.missingArtifactRuntimeAuthorityInvariants),
    requiredArtifactRuntimeSignals: countMapToObject(coverage.requiredArtifactRuntimeSignals),
    missingArtifactRuntimeSignals: countMapToObject(coverage.missingArtifactRuntimeSignals),
    requiredArtifactRuntimeSignalOwners: countMapToObject(coverage.requiredArtifactRuntimeSignalOwners),
    missingArtifactRuntimeSignalOwners: countMapToObject(coverage.missingArtifactRuntimeSignalOwners),
    requiredArtifactOwners: countMapToObject(coverage.requiredArtifactOwners),
    missingArtifactOwners: countMapToObject(coverage.missingArtifactOwners),
    requiredArtifactClassifications: countMapToObject(coverage.requiredArtifactClassifications),
    missingArtifactClassifications: countMapToObject(coverage.missingArtifactClassifications),
    requiredArtifactFailureClassifications: countMapToObject(coverage.requiredArtifactFailureClassifications),
    missingArtifactFailureClassifications: countMapToObject(coverage.missingArtifactFailureClassifications),
    requiredArtifactPlannedOwners: countMapToObject(coverage.requiredArtifactPlannedOwners),
    missingArtifactPlannedOwners: countMapToObject(coverage.missingArtifactPlannedOwners),
    requiredArtifactPlannedClassifications: countMapToObject(coverage.requiredArtifactPlannedClassifications),
    missingArtifactPlannedClassifications: countMapToObject(coverage.missingArtifactPlannedClassifications),
    requiredArtifactExitCriterionStatuses: countMapToObject(coverage.requiredArtifactExitCriterionStatuses),
    missingArtifactExitCriterionStatuses: countMapToObject(coverage.missingArtifactExitCriterionStatuses),
    requiredArtifactIncompleteExitCriterionStatuses: countMapToObject(coverage.requiredArtifactIncompleteExitCriterionStatuses),
    missingArtifactIncompleteExitCriterionStatuses: countMapToObject(coverage.missingArtifactIncompleteExitCriterionStatuses),
    artifactSchemas: countMapToObject(coverage.artifactSchemas),
    artifactCoverageAreas: countMapToObject(coverage.artifactCoverageAreas),
    artifactRuntimeAuthorityInvariants: countMapToObject(coverage.artifactRuntimeAuthorityInvariants),
    requiredRuntimeSignals: countMapToObject(coverage.requiredRuntimeSignals),
    missingRuntimeSignals: countMapToObject(coverage.missingRuntimeSignals),
    requiredRuntimeSignalOwners: countMapToObject(coverage.requiredRuntimeSignalOwners),
    missingRuntimeSignalOwners: countMapToObject(coverage.missingRuntimeSignalOwners),
    requiredFailureClassifications: countMapToObject(coverage.requiredFailureClassifications),
    missingFailureClassifications: countMapToObject(coverage.missingFailureClassifications),
    artifactRuntimeSignals: countMapToObject(coverage.artifactRuntimeSignals),
    artifactRuntimeSignalOwners: countMapToObject(coverage.artifactRuntimeSignalOwners),
    artifactOwners: countMapToObject(coverage.artifactOwners),
    artifactClassifications: countMapToObject(coverage.artifactClassifications),
    artifactFailureClassifications: countMapToObject(coverage.artifactFailureClassifications),
    artifactPlannedOwners: countMapToObject(coverage.artifactPlannedOwners),
    artifactPlannedClassifications: countMapToObject(coverage.artifactPlannedClassifications),
    artifactExitCriterionStatuses: countMapToObject(coverage.artifactExitCriterionStatuses),
    artifactIncompleteExitCriterionStatuses: countMapToObject(coverage.artifactIncompleteExitCriterionStatuses),
    artifactKinds: countMapToObject(coverage.artifactKinds),
    artifactGeneratedEvidenceKinds: countMapToObject(coverage.artifactGeneratedEvidenceKinds),
    artifactGeneratedEvidenceRepos: countMapToObject(coverage.artifactGeneratedEvidenceRepos),
    artifactGeneratedMatrixArtifactIndexes: countMapToObject(coverage.artifactGeneratedMatrixArtifactIndexes),
    artifactGeneratedMatrixLimitations: countMapToObject(coverage.artifactGeneratedMatrixLimitations),
    artifactGeneratedMatrixNames: countMapToObject(coverage.artifactGeneratedMatrixNames),
    artifactGeneratedMatrixRepos: countMapToObject(coverage.artifactGeneratedMatrixRepos),
    artifactGeneratedValidationSuiteArtifactIndexes: countMapToObject(coverage.artifactGeneratedValidationSuiteArtifactIndexes),
    artifactGeneratedValidationSuiteFailureRoots: countMapToObject(coverage.artifactGeneratedValidationSuiteFailureRoots),
    artifactEvidenceRepos: countMapToObject(coverage.artifactEvidenceRepos),
    artifactProviderAccountAliases: countMapToObject(coverage.artifactProviderAccountAliases),
    artifactValidationPresets: countMapToObject(coverage.artifactValidationPresets),
    artifactCoverageInputSources: countMapToObject(coverage.artifactCoverageInputSources),
    failureRuntimeSignals: countMapToObject(coverage.failureRuntimeSignals),
    failureRuntimeSignalOwners: countMapToObject(coverage.failureRuntimeSignalOwners),
    failureOwners: countMapToObject(coverage.failureOwners),
    failureClassifications: countMapToObject(coverage.failureClassifications),
    failureStaleManifests: countMapToObject(coverage.failureStaleManifests),
    matrixRuntimeSignals: countMapToObject(coverage.matrixRuntimeSignals),
    matrixRuntimeSignalOwners: countMapToObject(coverage.matrixRuntimeSignalOwners),
    matrixOwners: countMapToObject(coverage.matrixOwners),
    matrixClassifications: countMapToObject(coverage.matrixClassifications),
    matrixStaleReports: countMapToObject(coverage.matrixStaleReports),
    requiredMatrices: countMapToObject(coverage.requiredMatrices),
    missingMatrices: countMapToObject(coverage.missingMatrices),
    requiredMatrixClassifications: countMapToObject(coverage.requiredMatrixClassifications),
    missingMatrixClassifications: countMapToObject(coverage.missingMatrixClassifications),
    requiredMatrixRuntimeSignals: countMapToObject(coverage.requiredMatrixRuntimeSignals),
    missingMatrixRuntimeSignals: countMapToObject(coverage.missingMatrixRuntimeSignals),
    requiredDeploymentPresets: countMapToObject(coverage.requiredDeploymentPresets),
    missingDeploymentPresets: countMapToObject(coverage.missingDeploymentPresets),
    requiredProviders: countMapToObject(coverage.requiredProviders),
    missingProviders: countMapToObject(coverage.missingProviders),
    requiredScenarios: countMapToObject(coverage.requiredScenarios),
    missingScenarios: countMapToObject(coverage.missingScenarios),
    generatedEvidenceKinds: countMapToObject(coverage.generatedEvidenceKinds),
    generatedMatrixArtifactIndexes: countMapToObject(coverage.generatedMatrixArtifactIndexes),
    generatedMatrixLimitations: countMapToObject(coverage.generatedMatrixLimitations),
    generatedValidationSuiteArtifactIndexes: countMapToObject(coverage.generatedValidationSuiteArtifactIndexes),
    generatedValidationSuiteFailureRoots: countMapToObject(coverage.generatedValidationSuiteFailureRoots),
    requiredGeneratedEvidenceKinds: countMapToObject(coverage.requiredGeneratedEvidenceKinds),
    missingGeneratedEvidenceKinds: countMapToObject(coverage.missingGeneratedEvidenceKinds),
    requiredGeneratedMatrixArtifactIndexes: countMapToObject(coverage.requiredGeneratedMatrixArtifactIndexes),
    missingGeneratedMatrixArtifactIndexes: countMapToObject(coverage.missingGeneratedMatrixArtifactIndexes),
    requiredGeneratedMatrixLimitations: countMapToObject(coverage.requiredGeneratedMatrixLimitations),
    missingGeneratedMatrixLimitations: countMapToObject(coverage.missingGeneratedMatrixLimitations),
    requiredGeneratedValidationSuiteArtifactIndexes: countMapToObject(coverage.requiredGeneratedValidationSuiteArtifactIndexes),
    missingGeneratedValidationSuiteArtifactIndexes: countMapToObject(coverage.missingGeneratedValidationSuiteArtifactIndexes),
    requiredGeneratedValidationSuiteFailureRoots: countMapToObject(coverage.requiredGeneratedValidationSuiteFailureRoots),
    missingGeneratedValidationSuiteFailureRoots: countMapToObject(coverage.missingGeneratedValidationSuiteFailureRoots),
  }
}

export function countMapToObject(counts) {
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

export function formatValidationGateCoverageSummary(coverage) {
  const lines = []
  appendCoverageLine(lines, "presets", coverage.presets)
  appendCoverageLine(lines, "required_platform_coverage_areas", coverage.requiredPlatformCoverageAreas)
  appendCoverageLine(lines, "missing_platform_coverage_areas", coverage.missingPlatformCoverageAreas)
  appendCoverageLine(lines, "required_artifact_coverage_areas", coverage.requiredArtifactCoverageAreas)
  appendCoverageLine(lines, "missing_artifact_coverage_areas", coverage.missingArtifactCoverageAreas)
  appendCoverageLine(lines, "required_artifact_schemas", coverage.requiredArtifactSchemas)
  appendCoverageLine(lines, "missing_artifact_schemas", coverage.missingArtifactSchemas)
  appendCoverageLine(lines, "required_artifact_kinds", coverage.requiredArtifactKinds)
  appendCoverageLine(lines, "missing_artifact_kinds", coverage.missingArtifactKinds)
  appendCoverageLine(lines, "required_artifact_generated_evidence_kinds", coverage.requiredArtifactGeneratedEvidenceKinds)
  appendCoverageLine(lines, "missing_artifact_generated_evidence_kinds", coverage.missingArtifactGeneratedEvidenceKinds)
  appendCoverageLine(lines, "required_artifact_generated_evidence_repos", coverage.requiredArtifactGeneratedEvidenceRepos)
  appendCoverageLine(lines, "missing_artifact_generated_evidence_repos", coverage.missingArtifactGeneratedEvidenceRepos)
  appendCoverageLine(lines, "required_artifact_generated_matrix_artifact_indexes", coverage.requiredArtifactGeneratedMatrixArtifactIndexes)
  appendCoverageLine(lines, "missing_artifact_generated_matrix_artifact_indexes", coverage.missingArtifactGeneratedMatrixArtifactIndexes)
  appendCoverageLine(lines, "required_artifact_generated_matrix_limitations", coverage.requiredArtifactGeneratedMatrixLimitations)
  appendCoverageLine(lines, "missing_artifact_generated_matrix_limitations", coverage.missingArtifactGeneratedMatrixLimitations)
  appendCoverageLine(lines, "required_artifact_generated_matrix_names", coverage.requiredArtifactGeneratedMatrixNames)
  appendCoverageLine(lines, "missing_artifact_generated_matrix_names", coverage.missingArtifactGeneratedMatrixNames)
  appendCoverageLine(lines, "required_artifact_generated_matrix_repos", coverage.requiredArtifactGeneratedMatrixRepos)
  appendCoverageLine(lines, "missing_artifact_generated_matrix_repos", coverage.missingArtifactGeneratedMatrixRepos)
  appendCoverageLine(lines, "required_artifact_generated_validation_suite_artifact_indexes", coverage.requiredArtifactGeneratedValidationSuiteArtifactIndexes)
  appendCoverageLine(lines, "missing_artifact_generated_validation_suite_artifact_indexes", coverage.missingArtifactGeneratedValidationSuiteArtifactIndexes)
  appendCoverageLine(lines, "required_artifact_generated_validation_suite_failure_roots", coverage.requiredArtifactGeneratedValidationSuiteFailureRoots)
  appendCoverageLine(lines, "missing_artifact_generated_validation_suite_failure_roots", coverage.missingArtifactGeneratedValidationSuiteFailureRoots)
  appendCoverageLine(lines, "required_artifact_evidence_repos", coverage.requiredArtifactEvidenceRepos)
  appendCoverageLine(lines, "missing_artifact_evidence_repos", coverage.missingArtifactEvidenceRepos)
  appendCoverageLine(lines, "required_artifact_provider_account_aliases", coverage.requiredArtifactProviderAccountAliases)
  appendCoverageLine(lines, "missing_artifact_provider_account_aliases", coverage.missingArtifactProviderAccountAliases)
  appendCoverageLine(lines, "required_artifact_validation_presets", coverage.requiredArtifactValidationPresets)
  appendCoverageLine(lines, "missing_artifact_validation_presets", coverage.missingArtifactValidationPresets)
  appendCoverageLine(lines, "required_artifact_runtime_authority_invariants", coverage.requiredArtifactRuntimeAuthorityInvariants)
  appendCoverageLine(lines, "missing_artifact_runtime_authority_invariants", coverage.missingArtifactRuntimeAuthorityInvariants)
  appendCoverageLine(lines, "required_artifact_runtime_signals", coverage.requiredArtifactRuntimeSignals)
  appendCoverageLine(lines, "missing_artifact_runtime_signals", coverage.missingArtifactRuntimeSignals)
  appendCoverageLine(lines, "required_artifact_runtime_signal_owners", coverage.requiredArtifactRuntimeSignalOwners)
  appendCoverageLine(lines, "missing_artifact_runtime_signal_owners", coverage.missingArtifactRuntimeSignalOwners)
  appendCoverageLine(lines, "required_artifact_owners", coverage.requiredArtifactOwners)
  appendCoverageLine(lines, "missing_artifact_owners", coverage.missingArtifactOwners)
  appendCoverageLine(lines, "required_artifact_classifications", coverage.requiredArtifactClassifications)
  appendCoverageLine(lines, "missing_artifact_classifications", coverage.missingArtifactClassifications)
  appendCoverageLine(lines, "required_artifact_failure_classifications", coverage.requiredArtifactFailureClassifications)
  appendCoverageLine(lines, "missing_artifact_failure_classifications", coverage.missingArtifactFailureClassifications)
  appendCoverageLine(lines, "required_artifact_planned_owners", coverage.requiredArtifactPlannedOwners)
  appendCoverageLine(lines, "missing_artifact_planned_owners", coverage.missingArtifactPlannedOwners)
  appendCoverageLine(lines, "required_artifact_planned_classifications", coverage.requiredArtifactPlannedClassifications)
  appendCoverageLine(lines, "missing_artifact_planned_classifications", coverage.missingArtifactPlannedClassifications)
  appendCoverageLine(lines, "required_artifact_exit_criterion_statuses", coverage.requiredArtifactExitCriterionStatuses)
  appendCoverageLine(lines, "missing_artifact_exit_criterion_statuses", coverage.missingArtifactExitCriterionStatuses)
  appendCoverageLine(lines, "required_artifact_incomplete_exit_criterion_statuses", coverage.requiredArtifactIncompleteExitCriterionStatuses)
  appendCoverageLine(lines, "missing_artifact_incomplete_exit_criterion_statuses", coverage.missingArtifactIncompleteExitCriterionStatuses)
  appendCoverageLine(lines, "artifact_schemas", coverage.artifactSchemas)
  appendCoverageLine(lines, "artifact_coverage_areas", coverage.artifactCoverageAreas)
  appendCoverageLine(lines, "artifact_runtime_authority_invariants", coverage.artifactRuntimeAuthorityInvariants)
  appendCoverageLine(lines, "required_runtime_signals", coverage.requiredRuntimeSignals)
  appendCoverageLine(lines, "missing_runtime_signals", coverage.missingRuntimeSignals)
  appendCoverageLine(lines, "required_runtime_signal_owners", coverage.requiredRuntimeSignalOwners)
  appendCoverageLine(lines, "missing_runtime_signal_owners", coverage.missingRuntimeSignalOwners)
  appendCoverageLine(lines, "required_failure_classifications", coverage.requiredFailureClassifications)
  appendCoverageLine(lines, "missing_failure_classifications", coverage.missingFailureClassifications)
  appendCoverageLine(lines, "artifact_runtime_signals", coverage.artifactRuntimeSignals)
  appendCoverageLine(lines, "artifact_runtime_signal_owners", coverage.artifactRuntimeSignalOwners)
  appendCoverageLine(lines, "artifact_owners", coverage.artifactOwners)
  appendCoverageLine(lines, "artifact_classifications", coverage.artifactClassifications)
  appendCoverageLine(lines, "artifact_failure_classifications", coverage.artifactFailureClassifications)
  appendCoverageLine(lines, "artifact_planned_owners", coverage.artifactPlannedOwners)
  appendCoverageLine(lines, "artifact_planned_classifications", coverage.artifactPlannedClassifications)
  appendCoverageLine(lines, "artifact_exit_criterion_statuses", coverage.artifactExitCriterionStatuses)
  appendCoverageLine(lines, "artifact_incomplete_exit_criterion_statuses", coverage.artifactIncompleteExitCriterionStatuses)
  appendCoverageLine(lines, "artifact_kinds", coverage.artifactKinds)
  appendCoverageLine(lines, "artifact_generated_evidence_kinds", coverage.artifactGeneratedEvidenceKinds)
  appendCoverageLine(lines, "artifact_generated_evidence_repos", coverage.artifactGeneratedEvidenceRepos)
  appendCoverageLine(lines, "artifact_generated_matrix_artifact_indexes", coverage.artifactGeneratedMatrixArtifactIndexes)
  appendCoverageLine(lines, "artifact_generated_matrix_limitations", coverage.artifactGeneratedMatrixLimitations)
  appendCoverageLine(lines, "artifact_generated_matrix_names", coverage.artifactGeneratedMatrixNames)
  appendCoverageLine(lines, "artifact_generated_matrix_repos", coverage.artifactGeneratedMatrixRepos)
  appendCoverageLine(lines, "artifact_generated_validation_suite_artifact_indexes", coverage.artifactGeneratedValidationSuiteArtifactIndexes)
  appendCoverageLine(lines, "artifact_generated_validation_suite_failure_roots", coverage.artifactGeneratedValidationSuiteFailureRoots)
  appendCoverageLine(lines, "artifact_evidence_repos", coverage.artifactEvidenceRepos)
  appendCoverageLine(lines, "artifact_provider_account_aliases", coverage.artifactProviderAccountAliases)
  appendCoverageLine(lines, "artifact_validation_presets", coverage.artifactValidationPresets)
  appendCoverageLine(lines, "artifact_coverage_input_sources", coverage.artifactCoverageInputSources)
  appendCoverageLine(lines, "failure_runtime_signals", coverage.failureRuntimeSignals)
  appendCoverageLine(lines, "failure_runtime_signal_owners", coverage.failureRuntimeSignalOwners)
  appendCoverageLine(lines, "failure_owners", coverage.failureOwners)
  appendCoverageLine(lines, "failure_classifications", coverage.failureClassifications)
  appendCoverageLine(lines, "failure_stale_manifests", coverage.failureStaleManifests)
  appendCoverageLine(lines, "matrix_runtime_signals", coverage.matrixRuntimeSignals)
  appendCoverageLine(lines, "matrix_runtime_signal_owners", coverage.matrixRuntimeSignalOwners)
  appendCoverageLine(lines, "matrix_owners", coverage.matrixOwners)
  appendCoverageLine(lines, "matrix_classifications", coverage.matrixClassifications)
  appendCoverageLine(lines, "matrix_stale_reports", coverage.matrixStaleReports)
  appendCoverageLine(lines, "required_matrices", coverage.requiredMatrices)
  appendCoverageLine(lines, "missing_matrices", coverage.missingMatrices)
  appendCoverageLine(lines, "required_matrix_classifications", coverage.requiredMatrixClassifications)
  appendCoverageLine(lines, "missing_matrix_classifications", coverage.missingMatrixClassifications)
  appendCoverageLine(lines, "required_matrix_runtime_signals", coverage.requiredMatrixRuntimeSignals)
  appendCoverageLine(lines, "missing_matrix_runtime_signals", coverage.missingMatrixRuntimeSignals)
  appendCoverageLine(lines, "required_deployment_presets", coverage.requiredDeploymentPresets)
  appendCoverageLine(lines, "missing_deployment_presets", coverage.missingDeploymentPresets)
  appendCoverageLine(lines, "required_providers", coverage.requiredProviders)
  appendCoverageLine(lines, "missing_providers", coverage.missingProviders)
  appendCoverageLine(lines, "required_scenarios", coverage.requiredScenarios)
  appendCoverageLine(lines, "missing_scenarios", coverage.missingScenarios)
  appendCoverageLine(lines, "generated_evidence_kinds", coverage.generatedEvidenceKinds)
  appendCoverageLine(lines, "generated_matrix_artifact_indexes", coverage.generatedMatrixArtifactIndexes)
  appendCoverageLine(lines, "generated_matrix_limitations", coverage.generatedMatrixLimitations)
  appendCoverageLine(lines, "generated_validation_suite_artifact_indexes", coverage.generatedValidationSuiteArtifactIndexes)
  appendCoverageLine(lines, "generated_validation_suite_failure_roots", coverage.generatedValidationSuiteFailureRoots)
  appendCoverageLine(lines, "required_generated_evidence_kinds", coverage.requiredGeneratedEvidenceKinds)
  appendCoverageLine(lines, "missing_generated_evidence_kinds", coverage.missingGeneratedEvidenceKinds)
  appendCoverageLine(lines, "required_generated_matrix_artifact_indexes", coverage.requiredGeneratedMatrixArtifactIndexes)
  appendCoverageLine(lines, "missing_generated_matrix_artifact_indexes", coverage.missingGeneratedMatrixArtifactIndexes)
  appendCoverageLine(lines, "required_generated_matrix_limitations", coverage.requiredGeneratedMatrixLimitations)
  appendCoverageLine(lines, "missing_generated_matrix_limitations", coverage.missingGeneratedMatrixLimitations)
  appendCoverageLine(lines, "required_generated_validation_suite_artifact_indexes", coverage.requiredGeneratedValidationSuiteArtifactIndexes)
  appendCoverageLine(lines, "missing_generated_validation_suite_artifact_indexes", coverage.missingGeneratedValidationSuiteArtifactIndexes)
  appendCoverageLine(lines, "required_generated_validation_suite_failure_roots", coverage.requiredGeneratedValidationSuiteFailureRoots)
  appendCoverageLine(lines, "missing_generated_validation_suite_failure_roots", coverage.missingGeneratedValidationSuiteFailureRoots)
  return lines
}

export function appendCoverageLine(lines, label, counts) {
  const entries = Object.entries(counts ?? {})
  if (entries.length > 0) {
    lines.push(`- ${label}: ${entries.map(([key, count]) => `${key}=${count}`).join(" ")}`)
  }
}

export function appendAggregateRequirementLine(lines, label, required, missing) {
  if ((required ?? []).length > 0) {
    lines.push(`${label}=${required.join(",")} missing=${(missing ?? []).join(",") || "none"}`)
  }
}

export function appendAggregateMatrixRuntimeSignalSources(lines, matrixRuntimeSignalSources, requiredMatrixRuntimeSignals) {
  if ((requiredMatrixRuntimeSignals ?? []).length === 0) return
  const sources = matrixRuntimeSignalSources && typeof matrixRuntimeSignalSources === "object" && !Array.isArray(matrixRuntimeSignalSources)
    ? matrixRuntimeSignalSources
    : {}
  lines.push("matrix_runtime_signal_sources:")
  for (const signal of requiredMatrixRuntimeSignals) {
    const entries = Array.isArray(sources[signal]) ? sources[signal] : []
    lines.push(`- ${signal}: ${entries.length > 0 ? entries.map(formatMatrixRuntimeSignalSource).join(", ") : "missing"}`)
  }
}

export function formatMatrixRuntimeSignalSource(entry) {
  const report = entry.reportSource ? ` report=${entry.reportSource}` : ""
  const source = entry.source ? ` source=${entry.source}` : ""
  return `${entry.matrix}/${entry.id}(${entry.status})${source}${report}`
}

export function validationGateReportGeneratedEvidence(report) {
  const generatedEvidence = report.generatedEvidence
  if (!generatedEvidence || typeof generatedEvidence !== "object" || Array.isArray(generatedEvidence)) return null
  const validationSuites = generatedEvidence.validationSuites ?? {}
  const matrixReports = generatedEvidence.matrixReports ?? {}
  const stringArray = (value) => Array.isArray(value) ? [...value] : []
  const kinds = []
  if (validationSuites.enabled === true) kinds.push("validation-suite-run")
  if (matrixReports.enabled === true) kinds.push("matrix-report")
  return {
    kinds,
    validationSuites: {
      enabled: validationSuites.enabled === true,
      artifactIndexes: stringArray(validationSuites.artifactIndexes).length > 0
        ? stringArray(validationSuites.artifactIndexes)
        : (Array.isArray(validationSuites.commands) ? validationSuites.commands : [])
          .map((command) => command?.artifactIndexPath)
          .filter((artifactIndexPath) => typeof artifactIndexPath === "string" && artifactIndexPath.length > 0),
      failureRoots: stringArray(validationSuites.failureRoots).length > 0
        ? stringArray(validationSuites.failureRoots)
        : (Array.isArray(validationSuites.commands) ? validationSuites.commands : [])
          .map((command) => command?.failureRoot)
          .filter((failureRoot) => typeof failureRoot === "string" && failureRoot.length > 0),
      commands: (Array.isArray(validationSuites.commands) ? validationSuites.commands : []).map((command) => {
        const commandRecord = command && typeof command === "object" && !Array.isArray(command) ? command : {}
        return {
          artifactIndexPath: commandRecord.artifactIndexPath,
          args: stringArray(commandRecord.args),
          cwd: commandRecord.cwd,
          failureRoot: commandRecord.failureRoot,
          nodeArgs: stringArray(commandRecord.nodeArgs),
          reportPath: commandRecord.reportPath,
          scriptPath: commandRecord.scriptPath,
        }
      }),
      outputRoots: stringArray(validationSuites.outputRoots),
    },
    matrixReports: {
      enabled: matrixReports.enabled === true,
      artifactIndexes: stringArray(matrixReports.artifactIndexes).length > 0
        ? stringArray(matrixReports.artifactIndexes)
        : (Array.isArray(matrixReports.commands) ? matrixReports.commands : [])
          .map((command) => command?.artifactIndexPath)
          .filter((artifactIndexPath) => typeof artifactIndexPath === "string" && artifactIndexPath.length > 0),
      roots: stringArray(matrixReports.roots),
      dryRun: matrixReports.dryRun === true,
      continueOnFailure: matrixReports.continueOnFailure === true,
      limitations: (Array.isArray(matrixReports.limitations) ? matrixReports.limitations : []).map((limitation) => {
        const record = limitation && typeof limitation === "object" && !Array.isArray(limitation) ? limitation : {}
        return {
          kind: record.kind,
          owner: record.owner,
          nextAction: record.nextAction,
        }
      }),
      commands: (Array.isArray(matrixReports.commands) ? matrixReports.commands : []).map((command) => {
        const commandRecord = command && typeof command === "object" && !Array.isArray(command) ? command : {}
        return {
          artifactIndexFlag: commandRecord.artifactIndexFlag,
          artifactIndexPath: commandRecord.artifactIndexPath,
          args: stringArray(commandRecord.args),
          cwd: commandRecord.cwd,
          matrix: commandRecord.matrix,
          nodeArgs: stringArray(commandRecord.nodeArgs),
          repo: commandRecord.repo,
          reportPath: commandRecord.reportPath,
          scriptPath: commandRecord.scriptPath,
        }
      }),
    },
  }
}

export function cloneRuntimeSignalScenarios(runtimeSignalScenarios) {
  if (!runtimeSignalScenarios || typeof runtimeSignalScenarios !== "object" || Array.isArray(runtimeSignalScenarios)) return {}
  return Object.fromEntries(Object.entries(runtimeSignalScenarios)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([signal, scenarios]) => [signal, Array.isArray(scenarios)
      ? scenarios.map((scenario) => ({
        matrix: scenario.matrix,
        source: scenario.source ?? null,
        id: scenario.id,
        status: scenario.status,
      })).sort(compareMatrixRuntimeSignalSource)
      : []]))
}

export function appendMatrixRuntimeSignalSources(target, { reportSource, runtimeSignalScenarios }) {
  for (const [signal, scenarios] of Object.entries(cloneRuntimeSignalScenarios(runtimeSignalScenarios))) {
    const entries = target.get(signal) ?? []
    for (const scenario of scenarios) {
      entries.push({
        reportSource,
        matrix: scenario.matrix,
        source: scenario.source ?? null,
        id: scenario.id,
        status: scenario.status,
      })
    }
    target.set(signal, entries)
  }
}

export function formatMatrixRuntimeSignalSources(sources) {
  return Object.fromEntries([...sources.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([signal, entries]) => [signal, entries
      .map((entry) => ({
        reportSource: entry.reportSource ?? null,
        matrix: entry.matrix,
        source: entry.source ?? null,
        id: entry.id,
        status: entry.status,
      }))
      .sort(compareMatrixRuntimeSignalSource)]))
}

export function compareMatrixRuntimeSignalSource(left, right) {
  return String(left.reportSource ?? "").localeCompare(String(right.reportSource ?? ""))
    || String(left.matrix ?? "").localeCompare(String(right.matrix ?? ""))
    || String(left.source ?? "").localeCompare(String(right.source ?? ""))
    || String(left.id ?? "").localeCompare(String(right.id ?? ""))
    || String(left.status ?? "").localeCompare(String(right.status ?? ""))
}
