import {
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  formatDrillAggregateNextActionSourceDetails,
  validateDrillAggregateNextAction,
} from "./drill-aggregate-actions.mjs"
import { validateDrillValidationResultStatus } from "./drill-validation-statuses.mjs"

import {
  appendAggregateMatrixRuntimeSignalSources,
  appendAggregateRequirementLine,
  appendMatrixRuntimeSignalSources,
  appendMissingValidationGateAggregateNextActions,
  assertValidationGateAggregateMissingRequirementsMatch,
  countMapToObject,
  countObjectValues,
  countStringValues,
  countValidationGateArtifactCoverage,
  formatMatrixRuntimeSignalSources,
  formatValidationGateCoverageCounts,
  formatValidationGateCoverageSummary,
  missingCoverageRequirements,
  missingValidationGateAggregateRequirements,
  staleFailureManifestSourceLabels,
  staleMatrixReportSourceLabels,
  validationGateReportArtifactCoverage,
  validationGateReportFailureCoverage,
  validationGateReportGeneratedEvidence,
  validationGateReportMatrixCoverage,
  validationGateReportPlatformCoverage,
} from "./drill-validation-gate-aggregate-coverage.mjs"
import {
  validateArtifactEvidenceRepoArray,
  validateArtifactKindArray,
  validateArtifactValidationPresetArray,
  validateDeploymentPresetArray,
  validateExitCriterionStatusArray,
  validateFailureClassificationArray,
  validateGeneratedEvidenceKindArray,
  validateGeneratedEvidencePathArray,
  validateGeneratedMatrixLimitationArray,
  validateMatrixRuntimeSignalSources,
  validatePresetArray,
  validateProviderAccountAliasArray,
  validateProviderArray,
  validateRuntimeAuthorityInvariantArray,
  validateRuntimeSignalArray,
  validateRuntimeSignalOwnerArray,
  validateStringArray,
} from "./drill-validation-gate-aggregate-primitives.mjs"
import {
  assertMatrixRuntimeSignalSourcesMatchReports,
  assertValidationGateCoverageMatchesReports,
  validateGateAggregateArtifactCoverageInput,
  validateGateAggregateReportSummary,
  validateValidationGateCoverageAggregate,
} from "./drill-validation-gate-aggregate-validation.mjs"

export const DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA = "chariox.drill.validation_gate.aggregate.v1"

export function summarizeValidationGateReportAggregate(
  reports,
  {
    sources = [],
    supplementalArtifactReports = [],
    supplementalArtifactSources = [],
    normalizedRequiredPresets = [],
    normalizedAggregateRequirements = {},
    validateReport,
  } = {},
) {
  const totals = {
    reports: reports.length,
    passed: 0,
    failed: 0,
  }
  const nextActions = new Map()
  const matrixRuntimeSignalSources = new Map()
  const coverage = {
    presets: new Map(),
    requiredPlatformCoverageAreas: new Map(),
    missingPlatformCoverageAreas: new Map(),
    requiredRuntimeSignals: new Map(),
    missingRuntimeSignals: new Map(),
    requiredRuntimeSignalOwners: new Map(),
    missingRuntimeSignalOwners: new Map(),
    requiredFailureClassifications: new Map(),
    missingFailureClassifications: new Map(),
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
    requiredArtifactCoverageAreas: new Map(),
    missingArtifactCoverageAreas: new Map(),
    artifactSchemas: new Map(),
    artifactCoverageAreas: new Map(),
    artifactRuntimeAuthorityInvariants: new Map(),
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
  const summaries = reports.map((report, index) => {
    validateReport(report, sources[index] ?? "validation gate report")
    totals[report.status] += 1
    for (const action of report.nextActions) {
      countDrillAggregateNextAction(nextActions, action)
    }
    countStringValues(coverage.presets, report.presets ?? [])
    const platformCoverage = validationGateReportPlatformCoverage(report)
    countStringValues(coverage.requiredPlatformCoverageAreas, platformCoverage.requiredCoverageAreas)
    countStringValues(coverage.missingPlatformCoverageAreas, platformCoverage.missingCoverageAreas)
    countStringValues(coverage.requiredRuntimeSignals, platformCoverage.requiredRuntimeSignals)
    countStringValues(coverage.missingRuntimeSignals, platformCoverage.missingRuntimeSignals)
    countStringValues(coverage.requiredRuntimeSignalOwners, platformCoverage.requiredRuntimeSignalOwners)
    countStringValues(coverage.missingRuntimeSignalOwners, platformCoverage.missingRuntimeSignalOwners)
    countStringValues(coverage.requiredFailureClassifications, platformCoverage.requiredFailureClassifications)
    countStringValues(coverage.missingFailureClassifications, platformCoverage.missingFailureClassifications)
    const artifactCoverage = validationGateReportArtifactCoverage(report)
    countValidationGateArtifactCoverage(coverage, artifactCoverage)
    const failureCoverage = validationGateReportFailureCoverage(report)
    countObjectValues(coverage.failureRuntimeSignals, failureCoverage.runtimeSignals)
    countObjectValues(coverage.failureRuntimeSignalOwners, failureCoverage.runtimeSignalOwners)
    countObjectValues(coverage.failureOwners, failureCoverage.owners)
    countObjectValues(coverage.failureClassifications, failureCoverage.classifications)
    countStringValues(coverage.failureStaleManifests, staleFailureManifestSourceLabels(failureCoverage.staleFailureManifests))
    const matrixCoverage = validationGateReportMatrixCoverage(report)
    countObjectValues(coverage.matrixRuntimeSignals, matrixCoverage.runtimeSignals)
    countObjectValues(coverage.matrixRuntimeSignalOwners, matrixCoverage.runtimeSignalOwners)
    countObjectValues(coverage.matrixOwners, matrixCoverage.owners)
    countObjectValues(coverage.matrixClassifications, matrixCoverage.classifications)
    countStringValues(coverage.matrixStaleReports, staleMatrixReportSourceLabels(matrixCoverage.staleMatrixReports))
    countStringValues(coverage.requiredMatrices, matrixCoverage.requiredMatrices)
    countStringValues(coverage.missingMatrices, matrixCoverage.missingMatrices)
    countStringValues(coverage.requiredMatrixClassifications, matrixCoverage.requiredMatrixClassifications)
    countStringValues(coverage.missingMatrixClassifications, matrixCoverage.missingMatrixClassifications)
    countStringValues(coverage.requiredMatrixRuntimeSignals, matrixCoverage.requiredMatrixRuntimeSignals)
    countStringValues(coverage.missingMatrixRuntimeSignals, matrixCoverage.missingMatrixRuntimeSignals)
    countStringValues(coverage.requiredDeploymentPresets, matrixCoverage.requiredDeploymentPresets)
    countStringValues(coverage.missingDeploymentPresets, matrixCoverage.missingDeploymentPresets)
    countStringValues(coverage.requiredProviders, matrixCoverage.requiredProviders)
    countStringValues(coverage.missingProviders, matrixCoverage.missingProviders)
    countStringValues(coverage.requiredScenarios, matrixCoverage.requiredScenarios)
    countStringValues(coverage.missingScenarios, matrixCoverage.missingScenarios)
    appendMatrixRuntimeSignalSources(matrixRuntimeSignalSources, {
      reportSource: sources[index] ?? null,
      runtimeSignalScenarios: matrixCoverage.runtimeSignalScenarios,
    })
    const generatedEvidence = validationGateReportGeneratedEvidence(report)
    countStringValues(coverage.generatedEvidenceKinds, generatedEvidence?.kinds ?? [])
    countStringValues(
      coverage.generatedMatrixLimitations,
      (generatedEvidence?.matrixReports?.limitations ?? []).map((limitation) => limitation.kind),
    )
    countStringValues(coverage.generatedMatrixArtifactIndexes, generatedEvidence?.matrixReports?.artifactIndexes ?? [])
    countStringValues(coverage.generatedValidationSuiteArtifactIndexes, generatedEvidence?.validationSuites?.artifactIndexes ?? [])
    countStringValues(coverage.generatedValidationSuiteFailureRoots, generatedEvidence?.validationSuites?.failureRoots ?? [])
    return {
      source: sources[index] ?? null,
      status: report.status,
      presets: [...(report.presets ?? [])],
      checks: Object.fromEntries(Object.entries(report.checks).map(([name, check]) => [name, check.status])),
      platformCoverage,
      artifactCoverage,
      failureCoverage,
      matrixCoverage,
      ...(generatedEvidence ? { generatedEvidence } : {}),
    }
  })
  const artifactCoverageInputs = supplementalArtifactReports.map((report, index) => {
    validateReport(report, supplementalArtifactSources[index] ?? "validation gate artifact metadata input")
    for (const action of report.nextActions) {
      countDrillAggregateNextAction(nextActions, action)
    }
    const artifactCoverage = validationGateReportArtifactCoverage(report)
    countValidationGateArtifactCoverage(coverage, artifactCoverage)
    return {
      source: supplementalArtifactSources[index] ?? null,
      status: report.status,
      checks: Object.fromEntries(Object.entries(report.checks).map(([name, check]) => [name, check.status])),
      artifactCoverage,
    }
  })
  countStringValues(coverage.requiredGeneratedEvidenceKinds, normalizedAggregateRequirements.requiredGeneratedEvidenceKinds ?? [])
  countStringValues(
    coverage.missingGeneratedEvidenceKinds,
    missingCoverageRequirements(
      countMapToObject(coverage.generatedEvidenceKinds),
      normalizedAggregateRequirements.requiredGeneratedEvidenceKinds ?? [],
    ),
  )
  countStringValues(coverage.requiredGeneratedMatrixArtifactIndexes, normalizedAggregateRequirements.requiredGeneratedMatrixArtifactIndexes ?? [])
  countStringValues(
    coverage.missingGeneratedMatrixArtifactIndexes,
    missingCoverageRequirements(
      countMapToObject(coverage.generatedMatrixArtifactIndexes),
      normalizedAggregateRequirements.requiredGeneratedMatrixArtifactIndexes ?? [],
    ),
  )
  countStringValues(coverage.requiredGeneratedMatrixLimitations, normalizedAggregateRequirements.requiredGeneratedMatrixLimitations ?? [])
  countStringValues(
    coverage.missingGeneratedMatrixLimitations,
    missingCoverageRequirements(
      countMapToObject(coverage.generatedMatrixLimitations),
      normalizedAggregateRequirements.requiredGeneratedMatrixLimitations ?? [],
    ),
  )
  countStringValues(coverage.requiredGeneratedValidationSuiteArtifactIndexes, normalizedAggregateRequirements.requiredGeneratedValidationSuiteArtifactIndexes ?? [])
  countStringValues(
    coverage.missingGeneratedValidationSuiteArtifactIndexes,
    missingCoverageRequirements(
      countMapToObject(coverage.generatedValidationSuiteArtifactIndexes),
      normalizedAggregateRequirements.requiredGeneratedValidationSuiteArtifactIndexes ?? [],
    ),
  )
  countStringValues(coverage.requiredGeneratedValidationSuiteFailureRoots, normalizedAggregateRequirements.requiredGeneratedValidationSuiteFailureRoots ?? [])
  countStringValues(
    coverage.missingGeneratedValidationSuiteFailureRoots,
    missingCoverageRequirements(
      countMapToObject(coverage.generatedValidationSuiteFailureRoots),
      normalizedAggregateRequirements.requiredGeneratedValidationSuiteFailureRoots ?? [],
    ),
  )
  const coverageCounts = formatValidationGateCoverageCounts(coverage)
  const missingRequirements = missingValidationGateAggregateRequirements(coverageCounts, {
    ...normalizedAggregateRequirements,
    requiredPresets: normalizedRequiredPresets,
  })
  appendMissingValidationGateAggregateNextActions(nextActions, missingRequirements)
  const hasMissingRequirements = Object.values(missingRequirements).some((missing) => missing.length > 0)
  const hasFailedArtifactCoverageInputs = artifactCoverageInputs.some((input) => input.status === "failed")
  if (missingRequirements.missingPresets.length > 0) {
    countDrillAggregateNextAction(nextActions, {
      owner: "validation-harness",
      classification: "validation-gate",
      nextAction: `provide validation gate reports for presets: ${missingRequirements.missingPresets.join(", ")}`,
    })
  }
  const aggregate = {
    schema: DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
    status: totals.failed > 0 || hasMissingRequirements || hasFailedArtifactCoverageInputs ? "failed" : "passed",
    totals,
    requiredPresets: normalizedRequiredPresets,
    missingPresets: missingRequirements.missingPresets,
    requiredPlatformCoverageAreas: normalizedAggregateRequirements.requiredPlatformCoverageAreas,
    missingPlatformCoverageAreas: missingRequirements.missingPlatformCoverageAreas,
    requiredArtifactCoverageAreas: normalizedAggregateRequirements.requiredArtifactCoverageAreas,
    missingArtifactCoverageAreas: missingRequirements.missingArtifactCoverageAreas,
    requiredArtifactSchemas: normalizedAggregateRequirements.requiredArtifactSchemas,
    missingArtifactSchemas: missingRequirements.missingArtifactSchemas,
    requiredArtifactKinds: normalizedAggregateRequirements.requiredArtifactKinds,
    missingArtifactKinds: missingRequirements.missingArtifactKinds,
    requiredArtifactGeneratedEvidenceKinds: normalizedAggregateRequirements.requiredArtifactGeneratedEvidenceKinds,
    missingArtifactGeneratedEvidenceKinds: missingRequirements.missingArtifactGeneratedEvidenceKinds,
    requiredArtifactGeneratedEvidenceRepos: normalizedAggregateRequirements.requiredArtifactGeneratedEvidenceRepos,
    missingArtifactGeneratedEvidenceRepos: missingRequirements.missingArtifactGeneratedEvidenceRepos,
    requiredArtifactGeneratedMatrixArtifactIndexes: normalizedAggregateRequirements.requiredArtifactGeneratedMatrixArtifactIndexes,
    missingArtifactGeneratedMatrixArtifactIndexes: missingRequirements.missingArtifactGeneratedMatrixArtifactIndexes,
    requiredArtifactGeneratedMatrixLimitations: normalizedAggregateRequirements.requiredArtifactGeneratedMatrixLimitations,
    missingArtifactGeneratedMatrixLimitations: missingRequirements.missingArtifactGeneratedMatrixLimitations,
    requiredArtifactGeneratedMatrixNames: normalizedAggregateRequirements.requiredArtifactGeneratedMatrixNames,
    missingArtifactGeneratedMatrixNames: missingRequirements.missingArtifactGeneratedMatrixNames,
    requiredArtifactGeneratedMatrixRepos: normalizedAggregateRequirements.requiredArtifactGeneratedMatrixRepos,
    missingArtifactGeneratedMatrixRepos: missingRequirements.missingArtifactGeneratedMatrixRepos,
    requiredArtifactGeneratedValidationSuiteArtifactIndexes: normalizedAggregateRequirements.requiredArtifactGeneratedValidationSuiteArtifactIndexes,
    missingArtifactGeneratedValidationSuiteArtifactIndexes: missingRequirements.missingArtifactGeneratedValidationSuiteArtifactIndexes,
    requiredArtifactGeneratedValidationSuiteFailureRoots: normalizedAggregateRequirements.requiredArtifactGeneratedValidationSuiteFailureRoots,
    missingArtifactGeneratedValidationSuiteFailureRoots: missingRequirements.missingArtifactGeneratedValidationSuiteFailureRoots,
    requiredArtifactEvidenceRepos: normalizedAggregateRequirements.requiredArtifactEvidenceRepos,
    missingArtifactEvidenceRepos: missingRequirements.missingArtifactEvidenceRepos,
    requiredArtifactProviderAccountAliases: normalizedAggregateRequirements.requiredArtifactProviderAccountAliases,
    missingArtifactProviderAccountAliases: missingRequirements.missingArtifactProviderAccountAliases,
    requiredArtifactValidationPresets: normalizedAggregateRequirements.requiredArtifactValidationPresets,
    missingArtifactValidationPresets: missingRequirements.missingArtifactValidationPresets,
    requiredArtifactRuntimeAuthorityInvariants: normalizedAggregateRequirements.requiredArtifactRuntimeAuthorityInvariants,
    missingArtifactRuntimeAuthorityInvariants: missingRequirements.missingArtifactRuntimeAuthorityInvariants,
    requiredArtifactRuntimeSignals: normalizedAggregateRequirements.requiredArtifactRuntimeSignals,
    missingArtifactRuntimeSignals: missingRequirements.missingArtifactRuntimeSignals,
    requiredArtifactRuntimeSignalOwners: normalizedAggregateRequirements.requiredArtifactRuntimeSignalOwners,
    missingArtifactRuntimeSignalOwners: missingRequirements.missingArtifactRuntimeSignalOwners,
    requiredArtifactOwners: normalizedAggregateRequirements.requiredArtifactOwners,
    missingArtifactOwners: missingRequirements.missingArtifactOwners,
    requiredArtifactClassifications: normalizedAggregateRequirements.requiredArtifactClassifications,
    missingArtifactClassifications: missingRequirements.missingArtifactClassifications,
    requiredArtifactFailureClassifications: normalizedAggregateRequirements.requiredArtifactFailureClassifications,
    missingArtifactFailureClassifications: missingRequirements.missingArtifactFailureClassifications,
    requiredArtifactPlannedOwners: normalizedAggregateRequirements.requiredArtifactPlannedOwners,
    missingArtifactPlannedOwners: missingRequirements.missingArtifactPlannedOwners,
    requiredArtifactPlannedClassifications: normalizedAggregateRequirements.requiredArtifactPlannedClassifications,
    missingArtifactPlannedClassifications: missingRequirements.missingArtifactPlannedClassifications,
    requiredArtifactExitCriterionStatuses: normalizedAggregateRequirements.requiredArtifactExitCriterionStatuses,
    missingArtifactExitCriterionStatuses: missingRequirements.missingArtifactExitCriterionStatuses,
    requiredArtifactIncompleteExitCriterionStatuses: normalizedAggregateRequirements.requiredArtifactIncompleteExitCriterionStatuses,
    missingArtifactIncompleteExitCriterionStatuses: missingRequirements.missingArtifactIncompleteExitCriterionStatuses,
    requiredRuntimeSignals: normalizedAggregateRequirements.requiredRuntimeSignals,
    missingRuntimeSignals: missingRequirements.missingRuntimeSignals,
    requiredRuntimeSignalOwners: normalizedAggregateRequirements.requiredRuntimeSignalOwners,
    missingRuntimeSignalOwners: missingRequirements.missingRuntimeSignalOwners,
    requiredFailureClassifications: normalizedAggregateRequirements.requiredFailureClassifications,
    missingFailureClassifications: missingRequirements.missingFailureClassifications,
    requiredMatrices: normalizedAggregateRequirements.requiredMatrices,
    missingMatrices: missingRequirements.missingMatrices,
    requiredMatrixClassifications: normalizedAggregateRequirements.requiredMatrixClassifications,
    missingMatrixClassifications: missingRequirements.missingMatrixClassifications,
    requiredMatrixRuntimeSignals: normalizedAggregateRequirements.requiredMatrixRuntimeSignals,
    missingMatrixRuntimeSignals: missingRequirements.missingMatrixRuntimeSignals,
    requiredDeploymentPresets: normalizedAggregateRequirements.requiredDeploymentPresets,
    missingDeploymentPresets: missingRequirements.missingDeploymentPresets,
    requiredProviders: normalizedAggregateRequirements.requiredProviders,
    missingProviders: missingRequirements.missingProviders,
    requiredScenarios: normalizedAggregateRequirements.requiredScenarios,
    missingScenarios: missingRequirements.missingScenarios,
    requiredGeneratedEvidenceKinds: normalizedAggregateRequirements.requiredGeneratedEvidenceKinds,
    missingGeneratedEvidenceKinds: missingRequirements.missingGeneratedEvidenceKinds,
    requiredGeneratedMatrixArtifactIndexes: normalizedAggregateRequirements.requiredGeneratedMatrixArtifactIndexes,
    missingGeneratedMatrixArtifactIndexes: missingRequirements.missingGeneratedMatrixArtifactIndexes,
    requiredGeneratedMatrixLimitations: normalizedAggregateRequirements.requiredGeneratedMatrixLimitations,
    missingGeneratedMatrixLimitations: missingRequirements.missingGeneratedMatrixLimitations,
    requiredGeneratedValidationSuiteArtifactIndexes: normalizedAggregateRequirements.requiredGeneratedValidationSuiteArtifactIndexes,
    missingGeneratedValidationSuiteArtifactIndexes: missingRequirements.missingGeneratedValidationSuiteArtifactIndexes,
    requiredGeneratedValidationSuiteFailureRoots: normalizedAggregateRequirements.requiredGeneratedValidationSuiteFailureRoots,
    missingGeneratedValidationSuiteFailureRoots: missingRequirements.missingGeneratedValidationSuiteFailureRoots,
    matrixRuntimeSignalSources: formatMatrixRuntimeSignalSources(matrixRuntimeSignalSources),
    coverage: coverageCounts,
    nextActions: formatDrillAggregateNextActionCounts(nextActions),
    reports: summaries,
    ...(artifactCoverageInputs.length > 0 ? { artifactCoverageInputs } : {}),
  }
  validateDrillValidationGateAggregate(aggregate)
  return aggregate
}

export function formatDrillValidationGateAggregateSummary(aggregate) {
  validateDrillValidationGateAggregate(aggregate)
  const lines = [
    "drill validation gate aggregate:",
    `status=${aggregate.status} reports=${aggregate.totals.reports} passed=${aggregate.totals.passed} failed=${aggregate.totals.failed}`,
  ]
  const artifactCoverageInputs = aggregate.artifactCoverageInputs ?? []
  if (artifactCoverageInputs.length > 0) {
    const sources = artifactCoverageInputs
      .map((input) => input.source)
      .filter((source) => typeof source === "string" && source.length > 0)
    const failedInputs = artifactCoverageInputs.filter((input) => input.status === "failed").length
    lines.push(`artifact_coverage_inputs=${artifactCoverageInputs.length} failed=${failedInputs}${sources.length > 0 ? ` sources=${sources.join(",")}` : ""}`)
  }
  if (aggregate.nextActions.length > 0) {
    lines.push("next actions:")
    for (const action of aggregate.nextActions) {
      lines.push(`- owner=${action.owner} classification=${action.classification} count=${action.count}: ${action.nextAction}`)
      const sources = formatDrillAggregateNextActionSourceDetails(action.sourceDetails)
      if (sources) {
        lines.push(`  sources: ${sources}`)
      }
    }
  }
  if (aggregate.coverage) {
    const coverageLines = formatValidationGateCoverageSummary(aggregate.coverage)
    if (coverageLines.length > 0) {
      lines.push("coverage:")
      lines.push(...coverageLines)
    }
  }
  if ((aggregate.requiredPresets ?? []).length > 0) {
    lines.push(`required_presets=${aggregate.requiredPresets.join(",")} missing=${(aggregate.missingPresets ?? []).join(",") || "none"}`)
  }
  appendAggregateRequirementLine(lines, "required_platform_coverage_areas", aggregate.requiredPlatformCoverageAreas, aggregate.missingPlatformCoverageAreas)
  appendAggregateRequirementLine(lines, "required_artifact_coverage_areas", aggregate.requiredArtifactCoverageAreas, aggregate.missingArtifactCoverageAreas)
  appendAggregateRequirementLine(lines, "required_artifact_schemas", aggregate.requiredArtifactSchemas, aggregate.missingArtifactSchemas)
  appendAggregateRequirementLine(lines, "required_artifact_kinds", aggregate.requiredArtifactKinds, aggregate.missingArtifactKinds)
  appendAggregateRequirementLine(lines, "required_artifact_generated_evidence_kinds", aggregate.requiredArtifactGeneratedEvidenceKinds, aggregate.missingArtifactGeneratedEvidenceKinds)
  appendAggregateRequirementLine(lines, "required_artifact_generated_evidence_repos", aggregate.requiredArtifactGeneratedEvidenceRepos, aggregate.missingArtifactGeneratedEvidenceRepos)
  appendAggregateRequirementLine(lines, "required_artifact_generated_matrix_artifact_indexes", aggregate.requiredArtifactGeneratedMatrixArtifactIndexes, aggregate.missingArtifactGeneratedMatrixArtifactIndexes)
  appendAggregateRequirementLine(lines, "required_artifact_generated_matrix_limitations", aggregate.requiredArtifactGeneratedMatrixLimitations, aggregate.missingArtifactGeneratedMatrixLimitations)
  appendAggregateRequirementLine(lines, "required_artifact_generated_matrix_names", aggregate.requiredArtifactGeneratedMatrixNames, aggregate.missingArtifactGeneratedMatrixNames)
  appendAggregateRequirementLine(lines, "required_artifact_generated_matrix_repos", aggregate.requiredArtifactGeneratedMatrixRepos, aggregate.missingArtifactGeneratedMatrixRepos)
  appendAggregateRequirementLine(lines, "required_artifact_generated_validation_suite_artifact_indexes", aggregate.requiredArtifactGeneratedValidationSuiteArtifactIndexes, aggregate.missingArtifactGeneratedValidationSuiteArtifactIndexes)
  appendAggregateRequirementLine(lines, "required_artifact_generated_validation_suite_failure_roots", aggregate.requiredArtifactGeneratedValidationSuiteFailureRoots, aggregate.missingArtifactGeneratedValidationSuiteFailureRoots)
  appendAggregateRequirementLine(lines, "required_artifact_evidence_repos", aggregate.requiredArtifactEvidenceRepos, aggregate.missingArtifactEvidenceRepos)
  appendAggregateRequirementLine(lines, "required_artifact_provider_account_aliases", aggregate.requiredArtifactProviderAccountAliases, aggregate.missingArtifactProviderAccountAliases)
  appendAggregateRequirementLine(lines, "required_artifact_validation_presets", aggregate.requiredArtifactValidationPresets, aggregate.missingArtifactValidationPresets)
  appendAggregateRequirementLine(lines, "required_artifact_runtime_authority_invariants", aggregate.requiredArtifactRuntimeAuthorityInvariants, aggregate.missingArtifactRuntimeAuthorityInvariants)
  appendAggregateRequirementLine(lines, "required_artifact_runtime_signals", aggregate.requiredArtifactRuntimeSignals, aggregate.missingArtifactRuntimeSignals)
  appendAggregateRequirementLine(lines, "required_artifact_runtime_signal_owners", aggregate.requiredArtifactRuntimeSignalOwners, aggregate.missingArtifactRuntimeSignalOwners)
  appendAggregateRequirementLine(lines, "required_artifact_owners", aggregate.requiredArtifactOwners, aggregate.missingArtifactOwners)
  appendAggregateRequirementLine(lines, "required_artifact_classifications", aggregate.requiredArtifactClassifications, aggregate.missingArtifactClassifications)
  appendAggregateRequirementLine(lines, "required_artifact_failure_classifications", aggregate.requiredArtifactFailureClassifications, aggregate.missingArtifactFailureClassifications)
  appendAggregateRequirementLine(lines, "required_artifact_planned_owners", aggregate.requiredArtifactPlannedOwners, aggregate.missingArtifactPlannedOwners)
  appendAggregateRequirementLine(lines, "required_artifact_planned_classifications", aggregate.requiredArtifactPlannedClassifications, aggregate.missingArtifactPlannedClassifications)
  appendAggregateRequirementLine(lines, "required_artifact_exit_criterion_statuses", aggregate.requiredArtifactExitCriterionStatuses, aggregate.missingArtifactExitCriterionStatuses)
  appendAggregateRequirementLine(lines, "required_artifact_incomplete_exit_criterion_statuses", aggregate.requiredArtifactIncompleteExitCriterionStatuses, aggregate.missingArtifactIncompleteExitCriterionStatuses)
  appendAggregateRequirementLine(lines, "required_runtime_signals", aggregate.requiredRuntimeSignals, aggregate.missingRuntimeSignals)
  appendAggregateRequirementLine(lines, "required_runtime_signal_owners", aggregate.requiredRuntimeSignalOwners, aggregate.missingRuntimeSignalOwners)
  appendAggregateRequirementLine(lines, "required_failure_classifications", aggregate.requiredFailureClassifications, aggregate.missingFailureClassifications)
  appendAggregateRequirementLine(lines, "required_matrices", aggregate.requiredMatrices, aggregate.missingMatrices)
  appendAggregateRequirementLine(lines, "required_matrix_classifications", aggregate.requiredMatrixClassifications, aggregate.missingMatrixClassifications)
  appendAggregateRequirementLine(lines, "required_matrix_runtime_signals", aggregate.requiredMatrixRuntimeSignals, aggregate.missingMatrixRuntimeSignals)
  appendAggregateMatrixRuntimeSignalSources(lines, aggregate.matrixRuntimeSignalSources, aggregate.requiredMatrixRuntimeSignals)
  appendAggregateRequirementLine(lines, "required_deployment_presets", aggregate.requiredDeploymentPresets, aggregate.missingDeploymentPresets)
  appendAggregateRequirementLine(lines, "required_providers", aggregate.requiredProviders, aggregate.missingProviders)
  appendAggregateRequirementLine(lines, "required_scenarios", aggregate.requiredScenarios, aggregate.missingScenarios)
  appendAggregateRequirementLine(lines, "required_generated_evidence_kinds", aggregate.requiredGeneratedEvidenceKinds, aggregate.missingGeneratedEvidenceKinds)
  appendAggregateRequirementLine(lines, "required_generated_matrix_artifact_indexes", aggregate.requiredGeneratedMatrixArtifactIndexes, aggregate.missingGeneratedMatrixArtifactIndexes)
  appendAggregateRequirementLine(lines, "required_generated_matrix_limitations", aggregate.requiredGeneratedMatrixLimitations, aggregate.missingGeneratedMatrixLimitations)
  appendAggregateRequirementLine(lines, "required_generated_validation_suite_artifact_indexes", aggregate.requiredGeneratedValidationSuiteArtifactIndexes, aggregate.missingGeneratedValidationSuiteArtifactIndexes)
  appendAggregateRequirementLine(lines, "required_generated_validation_suite_failure_roots", aggregate.requiredGeneratedValidationSuiteFailureRoots, aggregate.missingGeneratedValidationSuiteFailureRoots)
  lines.push(aggregate.status === "passed"
    ? "next: all validation gate reports passed"
    : "next: inspect failed validation gate reports and rerun the relevant drills")
  return lines.join("\n")
}

export function drillValidationGateAggregateExitCode(aggregate) {
  validateDrillValidationGateAggregate(aggregate)
  return aggregate.status === "failed" ? 1 : 0
}

export function validateDrillValidationGateAggregate(aggregate, source = "validation gate aggregate") {
  if (!aggregate || typeof aggregate !== "object" || Array.isArray(aggregate)) {
    throw new Error(`${source} is not an object`)
  }
  if (aggregate.schema !== DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(aggregate.schema)}`)
  }
  validateDrillValidationResultStatus(aggregate.status, source)
  if (!aggregate.totals || typeof aggregate.totals !== "object" || Array.isArray(aggregate.totals)) {
    throw new Error(`${source} has invalid totals`)
  }
  for (const key of ["reports", "passed", "failed"]) {
    if (!Number.isSafeInteger(aggregate.totals[key]) || aggregate.totals[key] < 0) {
      throw new Error(`${source}.totals has invalid ${key}`)
    }
  }
  if (!Array.isArray(aggregate.nextActions)) {
    throw new Error(`${source} has invalid nextActions`)
  }
  validatePresetArray(aggregate.requiredPresets ?? [], `${source}.requiredPresets`)
  validatePresetArray(aggregate.missingPresets ?? [], `${source}.missingPresets`)
  validateStringArray(aggregate.requiredPlatformCoverageAreas ?? [], `${source}.requiredPlatformCoverageAreas`)
  validateStringArray(aggregate.missingPlatformCoverageAreas ?? [], `${source}.missingPlatformCoverageAreas`)
  validateStringArray(aggregate.requiredArtifactCoverageAreas ?? [], `${source}.requiredArtifactCoverageAreas`)
  validateStringArray(aggregate.missingArtifactCoverageAreas ?? [], `${source}.missingArtifactCoverageAreas`)
  validateStringArray(aggregate.requiredArtifactSchemas ?? [], `${source}.requiredArtifactSchemas`)
  validateStringArray(aggregate.missingArtifactSchemas ?? [], `${source}.missingArtifactSchemas`)
  validateArtifactKindArray(aggregate.requiredArtifactKinds ?? [], `${source}.requiredArtifactKinds`)
  validateArtifactKindArray(aggregate.missingArtifactKinds ?? [], `${source}.missingArtifactKinds`)
  validateGeneratedEvidenceKindArray(aggregate.requiredArtifactGeneratedEvidenceKinds ?? [], `${source}.requiredArtifactGeneratedEvidenceKinds`)
  validateGeneratedEvidenceKindArray(aggregate.missingArtifactGeneratedEvidenceKinds ?? [], `${source}.missingArtifactGeneratedEvidenceKinds`)
  validateArtifactEvidenceRepoArray(aggregate.requiredArtifactGeneratedEvidenceRepos ?? [], `${source}.requiredArtifactGeneratedEvidenceRepos`)
  validateArtifactEvidenceRepoArray(aggregate.missingArtifactGeneratedEvidenceRepos ?? [], `${source}.missingArtifactGeneratedEvidenceRepos`)
  validateGeneratedEvidencePathArray(aggregate.requiredArtifactGeneratedMatrixArtifactIndexes ?? [], `${source}.requiredArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedEvidencePathArray(aggregate.missingArtifactGeneratedMatrixArtifactIndexes ?? [], `${source}.missingArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationArray(aggregate.requiredArtifactGeneratedMatrixLimitations ?? [], `${source}.requiredArtifactGeneratedMatrixLimitations`)
  validateGeneratedMatrixLimitationArray(aggregate.missingArtifactGeneratedMatrixLimitations ?? [], `${source}.missingArtifactGeneratedMatrixLimitations`)
  validateStringArray(aggregate.requiredArtifactGeneratedMatrixNames ?? [], `${source}.requiredArtifactGeneratedMatrixNames`)
  validateStringArray(aggregate.missingArtifactGeneratedMatrixNames ?? [], `${source}.missingArtifactGeneratedMatrixNames`)
  validateArtifactEvidenceRepoArray(aggregate.requiredArtifactGeneratedMatrixRepos ?? [], `${source}.requiredArtifactGeneratedMatrixRepos`)
  validateArtifactEvidenceRepoArray(aggregate.missingArtifactGeneratedMatrixRepos ?? [], `${source}.missingArtifactGeneratedMatrixRepos`)
  validateGeneratedEvidencePathArray(aggregate.requiredArtifactGeneratedValidationSuiteArtifactIndexes ?? [], `${source}.requiredArtifactGeneratedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathArray(aggregate.missingArtifactGeneratedValidationSuiteArtifactIndexes ?? [], `${source}.missingArtifactGeneratedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathArray(aggregate.requiredArtifactGeneratedValidationSuiteFailureRoots ?? [], `${source}.requiredArtifactGeneratedValidationSuiteFailureRoots`)
  validateGeneratedEvidencePathArray(aggregate.missingArtifactGeneratedValidationSuiteFailureRoots ?? [], `${source}.missingArtifactGeneratedValidationSuiteFailureRoots`)
  validateArtifactEvidenceRepoArray(aggregate.requiredArtifactEvidenceRepos ?? [], `${source}.requiredArtifactEvidenceRepos`)
  validateArtifactEvidenceRepoArray(aggregate.missingArtifactEvidenceRepos ?? [], `${source}.missingArtifactEvidenceRepos`)
  validateProviderAccountAliasArray(aggregate.requiredArtifactProviderAccountAliases ?? [], `${source}.requiredArtifactProviderAccountAliases`)
  validateProviderAccountAliasArray(aggregate.missingArtifactProviderAccountAliases ?? [], `${source}.missingArtifactProviderAccountAliases`)
  validateArtifactValidationPresetArray(aggregate.requiredArtifactValidationPresets ?? [], `${source}.requiredArtifactValidationPresets`)
  validateArtifactValidationPresetArray(aggregate.missingArtifactValidationPresets ?? [], `${source}.missingArtifactValidationPresets`)
  validateRuntimeAuthorityInvariantArray(aggregate.requiredArtifactRuntimeAuthorityInvariants ?? [], `${source}.requiredArtifactRuntimeAuthorityInvariants`)
  validateRuntimeAuthorityInvariantArray(aggregate.missingArtifactRuntimeAuthorityInvariants ?? [], `${source}.missingArtifactRuntimeAuthorityInvariants`)
  validateRuntimeSignalArray(aggregate.requiredArtifactRuntimeSignals ?? [], `${source}.requiredArtifactRuntimeSignals`)
  validateRuntimeSignalArray(aggregate.missingArtifactRuntimeSignals ?? [], `${source}.missingArtifactRuntimeSignals`)
  validateRuntimeSignalOwnerArray(aggregate.requiredArtifactRuntimeSignalOwners ?? [], `${source}.requiredArtifactRuntimeSignalOwners`)
  validateRuntimeSignalOwnerArray(aggregate.missingArtifactRuntimeSignalOwners ?? [], `${source}.missingArtifactRuntimeSignalOwners`)
  validateStringArray(aggregate.requiredArtifactOwners ?? [], `${source}.requiredArtifactOwners`)
  validateStringArray(aggregate.missingArtifactOwners ?? [], `${source}.missingArtifactOwners`)
  validateStringArray(aggregate.requiredArtifactClassifications ?? [], `${source}.requiredArtifactClassifications`)
  validateStringArray(aggregate.missingArtifactClassifications ?? [], `${source}.missingArtifactClassifications`)
  validateFailureClassificationArray(aggregate.requiredArtifactFailureClassifications ?? [], `${source}.requiredArtifactFailureClassifications`)
  validateFailureClassificationArray(aggregate.missingArtifactFailureClassifications ?? [], `${source}.missingArtifactFailureClassifications`)
  validateStringArray(aggregate.requiredArtifactPlannedOwners ?? [], `${source}.requiredArtifactPlannedOwners`)
  validateStringArray(aggregate.missingArtifactPlannedOwners ?? [], `${source}.missingArtifactPlannedOwners`)
  validateStringArray(aggregate.requiredArtifactPlannedClassifications ?? [], `${source}.requiredArtifactPlannedClassifications`)
  validateStringArray(aggregate.missingArtifactPlannedClassifications ?? [], `${source}.missingArtifactPlannedClassifications`)
  validateExitCriterionStatusArray(aggregate.requiredArtifactExitCriterionStatuses ?? [], `${source}.requiredArtifactExitCriterionStatuses`)
  validateExitCriterionStatusArray(aggregate.missingArtifactExitCriterionStatuses ?? [], `${source}.missingArtifactExitCriterionStatuses`)
  validateExitCriterionStatusArray(aggregate.requiredArtifactIncompleteExitCriterionStatuses ?? [], `${source}.requiredArtifactIncompleteExitCriterionStatuses`)
  validateExitCriterionStatusArray(aggregate.missingArtifactIncompleteExitCriterionStatuses ?? [], `${source}.missingArtifactIncompleteExitCriterionStatuses`)
  validateRuntimeSignalArray(aggregate.requiredRuntimeSignals ?? [], `${source}.requiredRuntimeSignals`)
  validateRuntimeSignalArray(aggregate.missingRuntimeSignals ?? [], `${source}.missingRuntimeSignals`)
  validateRuntimeSignalOwnerArray(aggregate.requiredRuntimeSignalOwners ?? [], `${source}.requiredRuntimeSignalOwners`)
  validateRuntimeSignalOwnerArray(aggregate.missingRuntimeSignalOwners ?? [], `${source}.missingRuntimeSignalOwners`)
  validateFailureClassificationArray(aggregate.requiredFailureClassifications ?? [], `${source}.requiredFailureClassifications`)
  validateFailureClassificationArray(aggregate.missingFailureClassifications ?? [], `${source}.missingFailureClassifications`)
  validateStringArray(aggregate.requiredMatrices ?? [], `${source}.requiredMatrices`)
  validateStringArray(aggregate.missingMatrices ?? [], `${source}.missingMatrices`)
  validateFailureClassificationArray(aggregate.requiredMatrixClassifications ?? [], `${source}.requiredMatrixClassifications`)
  validateFailureClassificationArray(aggregate.missingMatrixClassifications ?? [], `${source}.missingMatrixClassifications`)
  validateRuntimeSignalArray(aggregate.requiredMatrixRuntimeSignals ?? [], `${source}.requiredMatrixRuntimeSignals`)
  validateRuntimeSignalArray(aggregate.missingMatrixRuntimeSignals ?? [], `${source}.missingMatrixRuntimeSignals`)
  if (aggregate.matrixRuntimeSignalSources !== undefined) {
    validateMatrixRuntimeSignalSources(aggregate.matrixRuntimeSignalSources, `${source}.matrixRuntimeSignalSources`)
  }
  validateDeploymentPresetArray(aggregate.requiredDeploymentPresets ?? [], `${source}.requiredDeploymentPresets`)
  validateDeploymentPresetArray(aggregate.missingDeploymentPresets ?? [], `${source}.missingDeploymentPresets`)
  validateProviderArray(aggregate.requiredProviders ?? [], `${source}.requiredProviders`)
  validateProviderArray(aggregate.missingProviders ?? [], `${source}.missingProviders`)
  validateStringArray(aggregate.requiredScenarios ?? [], `${source}.requiredScenarios`)
  validateStringArray(aggregate.missingScenarios ?? [], `${source}.missingScenarios`)
  validateGeneratedEvidenceKindArray(aggregate.requiredGeneratedEvidenceKinds ?? [], `${source}.requiredGeneratedEvidenceKinds`)
  validateGeneratedEvidenceKindArray(aggregate.missingGeneratedEvidenceKinds ?? [], `${source}.missingGeneratedEvidenceKinds`)
  validateGeneratedEvidencePathArray(aggregate.requiredGeneratedMatrixArtifactIndexes ?? [], `${source}.requiredGeneratedMatrixArtifactIndexes`)
  validateGeneratedEvidencePathArray(aggregate.missingGeneratedMatrixArtifactIndexes ?? [], `${source}.missingGeneratedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationArray(aggregate.requiredGeneratedMatrixLimitations ?? [], `${source}.requiredGeneratedMatrixLimitations`)
  validateGeneratedMatrixLimitationArray(aggregate.missingGeneratedMatrixLimitations ?? [], `${source}.missingGeneratedMatrixLimitations`)
  validateGeneratedEvidencePathArray(aggregate.requiredGeneratedValidationSuiteArtifactIndexes ?? [], `${source}.requiredGeneratedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathArray(aggregate.missingGeneratedValidationSuiteArtifactIndexes ?? [], `${source}.missingGeneratedValidationSuiteArtifactIndexes`)
  validateGeneratedEvidencePathArray(aggregate.requiredGeneratedValidationSuiteFailureRoots ?? [], `${source}.requiredGeneratedValidationSuiteFailureRoots`)
  validateGeneratedEvidencePathArray(aggregate.missingGeneratedValidationSuiteFailureRoots ?? [], `${source}.missingGeneratedValidationSuiteFailureRoots`)
  for (const [index, action] of aggregate.nextActions.entries()) {
    validateDrillAggregateNextAction(action, `${source}.nextActions[${index}]`)
  }
  if (!Array.isArray(aggregate.reports)) {
    throw new Error(`${source} has invalid reports`)
  }
  for (const [index, report] of aggregate.reports.entries()) {
    validateGateAggregateReportSummary(report, `${source}.reports[${index}]`)
  }
  if (aggregate.artifactCoverageInputs !== undefined) {
    if (!Array.isArray(aggregate.artifactCoverageInputs)) {
      throw new Error(`${source} has invalid artifactCoverageInputs`)
    }
    for (const [index, input] of aggregate.artifactCoverageInputs.entries()) {
      validateGateAggregateArtifactCoverageInput(input, `${source}.artifactCoverageInputs[${index}]`)
    }
  }
  if (aggregate.coverage !== undefined) {
    validateValidationGateCoverageAggregate(aggregate.coverage, `${source}.coverage`)
  }
  if (aggregate.totals.reports !== aggregate.reports.length) {
    throw new Error(`${source} totals.reports does not match reports`)
  }
  const passed = aggregate.reports.filter((report) => report.status === "passed").length
  const failed = aggregate.reports.filter((report) => report.status === "failed").length
  if (aggregate.totals.passed !== passed || aggregate.totals.failed !== failed) {
    throw new Error(`${source} totals do not match reports`)
  }
  const expectedMissingRequirements = missingValidationGateAggregateRequirements(aggregate.coverage ?? {}, {
    requiredPresets: aggregate.requiredPresets ?? [],
    requiredPlatformCoverageAreas: aggregate.requiredPlatformCoverageAreas ?? [],
    requiredArtifactCoverageAreas: aggregate.requiredArtifactCoverageAreas ?? [],
    requiredArtifactSchemas: aggregate.requiredArtifactSchemas ?? [],
    requiredArtifactKinds: aggregate.requiredArtifactKinds ?? [],
    requiredArtifactGeneratedEvidenceKinds: aggregate.requiredArtifactGeneratedEvidenceKinds ?? [],
    requiredArtifactGeneratedEvidenceRepos: aggregate.requiredArtifactGeneratedEvidenceRepos ?? [],
    requiredArtifactGeneratedMatrixArtifactIndexes: aggregate.requiredArtifactGeneratedMatrixArtifactIndexes ?? [],
    requiredArtifactGeneratedMatrixLimitations: aggregate.requiredArtifactGeneratedMatrixLimitations ?? [],
    requiredArtifactGeneratedMatrixNames: aggregate.requiredArtifactGeneratedMatrixNames ?? [],
    requiredArtifactGeneratedMatrixRepos: aggregate.requiredArtifactGeneratedMatrixRepos ?? [],
    requiredArtifactGeneratedValidationSuiteArtifactIndexes: aggregate.requiredArtifactGeneratedValidationSuiteArtifactIndexes ?? [],
    requiredArtifactGeneratedValidationSuiteFailureRoots: aggregate.requiredArtifactGeneratedValidationSuiteFailureRoots ?? [],
    requiredArtifactEvidenceRepos: aggregate.requiredArtifactEvidenceRepos ?? [],
    requiredArtifactProviderAccountAliases: aggregate.requiredArtifactProviderAccountAliases ?? [],
    requiredArtifactValidationPresets: aggregate.requiredArtifactValidationPresets ?? [],
    requiredArtifactRuntimeAuthorityInvariants: aggregate.requiredArtifactRuntimeAuthorityInvariants ?? [],
    requiredArtifactRuntimeSignals: aggregate.requiredArtifactRuntimeSignals ?? [],
    requiredArtifactRuntimeSignalOwners: aggregate.requiredArtifactRuntimeSignalOwners ?? [],
    requiredArtifactOwners: aggregate.requiredArtifactOwners ?? [],
    requiredArtifactClassifications: aggregate.requiredArtifactClassifications ?? [],
    requiredArtifactFailureClassifications: aggregate.requiredArtifactFailureClassifications ?? [],
    requiredArtifactPlannedOwners: aggregate.requiredArtifactPlannedOwners ?? [],
    requiredArtifactPlannedClassifications: aggregate.requiredArtifactPlannedClassifications ?? [],
    requiredArtifactExitCriterionStatuses: aggregate.requiredArtifactExitCriterionStatuses ?? [],
    requiredArtifactIncompleteExitCriterionStatuses: aggregate.requiredArtifactIncompleteExitCriterionStatuses ?? [],
    requiredRuntimeSignals: aggregate.requiredRuntimeSignals ?? [],
    requiredRuntimeSignalOwners: aggregate.requiredRuntimeSignalOwners ?? [],
    requiredFailureClassifications: aggregate.requiredFailureClassifications ?? [],
    requiredMatrices: aggregate.requiredMatrices ?? [],
    requiredMatrixClassifications: aggregate.requiredMatrixClassifications ?? [],
    requiredMatrixRuntimeSignals: aggregate.requiredMatrixRuntimeSignals ?? [],
    requiredDeploymentPresets: aggregate.requiredDeploymentPresets ?? [],
    requiredProviders: aggregate.requiredProviders ?? [],
    requiredScenarios: aggregate.requiredScenarios ?? [],
    requiredGeneratedEvidenceKinds: aggregate.requiredGeneratedEvidenceKinds ?? [],
    requiredGeneratedMatrixArtifactIndexes: aggregate.requiredGeneratedMatrixArtifactIndexes ?? [],
    requiredGeneratedMatrixLimitations: aggregate.requiredGeneratedMatrixLimitations ?? [],
    requiredGeneratedValidationSuiteArtifactIndexes: aggregate.requiredGeneratedValidationSuiteArtifactIndexes ?? [],
    requiredGeneratedValidationSuiteFailureRoots: aggregate.requiredGeneratedValidationSuiteFailureRoots ?? [],
  })
  assertValidationGateAggregateMissingRequirementsMatch(aggregate, expectedMissingRequirements, source)
  const hasMissingRequirements = Object.values(expectedMissingRequirements).some((missing) => missing.length > 0)
  const hasFailedArtifactCoverageInputs = (aggregate.artifactCoverageInputs ?? [])
    .some((input) => input.status === "failed")
  const expectedStatus = aggregate.totals.failed > 0 || hasMissingRequirements || hasFailedArtifactCoverageInputs
    ? "failed"
    : "passed"
  if (aggregate.status !== expectedStatus) {
    throw new Error(`${source} status does not match totals and requirements`)
  }
  if (aggregate.coverage !== undefined) {
    assertValidationGateCoverageMatchesReports(aggregate, source)
  }
  if (aggregate.matrixRuntimeSignalSources !== undefined) {
    assertMatrixRuntimeSignalSourcesMatchReports(aggregate, source)
  }
}
