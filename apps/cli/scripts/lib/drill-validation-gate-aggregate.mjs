import {
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  validateDrillAggregateNextAction,
} from "./drill-aggregate-actions.mjs"
import { validateDrillArtifactKind } from "./drill-artifact-kinds.mjs"
import { validateDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import { isKnownDrillDeploymentPreset } from "./drill-environment-presets.mjs"
import { validateDrillFailureClassification } from "./drill-failure-taxonomy.mjs"
import {
  validateDrillGeneratedEvidenceKind,
  validateDrillGeneratedEvidencePath,
} from "./drill-generated-evidence-metadata.mjs"
import { validateDrillGeneratedMatrixCommandMetadata } from "./drill-generated-matrix-command-metadata.mjs"
import { validateDrillGeneratedMatrixLimitation } from "./drill-generated-matrix-limitations.mjs"
import {
  parseProviderAccountAlias,
  validateDrillProvider,
} from "./drill-provider-profiles.mjs"
import {
  isKnownDrillArtifactValidationPreset,
  isKnownDrillValidationGatePreset,
} from "./drill-validation-gate-presets.mjs"
import {
  DRILL_RUNTIME_SIGNAL_OWNERS,
  drillRuntimeSignalOwnerCounts,
  isKnownDrillRuntimeSignal,
} from "./drill-runtime-signals.mjs"

export const DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA = "arroba.drill.validation_gate.aggregate.v1"

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
    requiredArtifactGeneratedMatrixArtifactIndexes: new Map(),
    missingArtifactGeneratedMatrixArtifactIndexes: new Map(),
    requiredArtifactGeneratedMatrixLimitations: new Map(),
    missingArtifactGeneratedMatrixLimitations: new Map(),
    requiredArtifactGeneratedMatrixNames: new Map(),
    missingArtifactGeneratedMatrixNames: new Map(),
    requiredArtifactGeneratedMatrixRepos: new Map(),
    missingArtifactGeneratedMatrixRepos: new Map(),
    requiredArtifactEvidenceRepos: new Map(),
    missingArtifactEvidenceRepos: new Map(),
    requiredArtifactProviderAccountAliases: new Map(),
    missingArtifactProviderAccountAliases: new Map(),
    requiredArtifactValidationPresets: new Map(),
    missingArtifactValidationPresets: new Map(),
    requiredArtifactRuntimeSignals: new Map(),
    missingArtifactRuntimeSignals: new Map(),
    requiredArtifactRuntimeSignalOwners: new Map(),
    missingArtifactRuntimeSignalOwners: new Map(),
    requiredArtifactOwners: new Map(),
    missingArtifactOwners: new Map(),
    requiredArtifactClassifications: new Map(),
    missingArtifactClassifications: new Map(),
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
    artifactRuntimeSignals: new Map(),
    artifactRuntimeSignalOwners: new Map(),
    artifactOwners: new Map(),
    artifactClassifications: new Map(),
    artifactPlannedOwners: new Map(),
    artifactPlannedClassifications: new Map(),
    artifactExitCriterionStatuses: new Map(),
    artifactIncompleteExitCriterionStatuses: new Map(),
    artifactKinds: new Map(),
    artifactGeneratedEvidenceKinds: new Map(),
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
    requiredArtifactGeneratedMatrixArtifactIndexes: normalizedAggregateRequirements.requiredArtifactGeneratedMatrixArtifactIndexes,
    missingArtifactGeneratedMatrixArtifactIndexes: missingRequirements.missingArtifactGeneratedMatrixArtifactIndexes,
    requiredArtifactGeneratedMatrixLimitations: normalizedAggregateRequirements.requiredArtifactGeneratedMatrixLimitations,
    missingArtifactGeneratedMatrixLimitations: missingRequirements.missingArtifactGeneratedMatrixLimitations,
    requiredArtifactGeneratedMatrixNames: normalizedAggregateRequirements.requiredArtifactGeneratedMatrixNames,
    missingArtifactGeneratedMatrixNames: missingRequirements.missingArtifactGeneratedMatrixNames,
    requiredArtifactGeneratedMatrixRepos: normalizedAggregateRequirements.requiredArtifactGeneratedMatrixRepos,
    missingArtifactGeneratedMatrixRepos: missingRequirements.missingArtifactGeneratedMatrixRepos,
    requiredArtifactEvidenceRepos: normalizedAggregateRequirements.requiredArtifactEvidenceRepos,
    missingArtifactEvidenceRepos: missingRequirements.missingArtifactEvidenceRepos,
    requiredArtifactProviderAccountAliases: normalizedAggregateRequirements.requiredArtifactProviderAccountAliases,
    missingArtifactProviderAccountAliases: missingRequirements.missingArtifactProviderAccountAliases,
    requiredArtifactValidationPresets: normalizedAggregateRequirements.requiredArtifactValidationPresets,
    missingArtifactValidationPresets: missingRequirements.missingArtifactValidationPresets,
    requiredArtifactRuntimeSignals: normalizedAggregateRequirements.requiredArtifactRuntimeSignals,
    missingArtifactRuntimeSignals: missingRequirements.missingArtifactRuntimeSignals,
    requiredArtifactRuntimeSignalOwners: normalizedAggregateRequirements.requiredArtifactRuntimeSignalOwners,
    missingArtifactRuntimeSignalOwners: missingRequirements.missingArtifactRuntimeSignalOwners,
    requiredArtifactOwners: normalizedAggregateRequirements.requiredArtifactOwners,
    missingArtifactOwners: missingRequirements.missingArtifactOwners,
    requiredArtifactClassifications: normalizedAggregateRequirements.requiredArtifactClassifications,
    missingArtifactClassifications: missingRequirements.missingArtifactClassifications,
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
  appendAggregateRequirementLine(lines, "required_artifact_generated_matrix_artifact_indexes", aggregate.requiredArtifactGeneratedMatrixArtifactIndexes, aggregate.missingArtifactGeneratedMatrixArtifactIndexes)
  appendAggregateRequirementLine(lines, "required_artifact_generated_matrix_limitations", aggregate.requiredArtifactGeneratedMatrixLimitations, aggregate.missingArtifactGeneratedMatrixLimitations)
  appendAggregateRequirementLine(lines, "required_artifact_generated_matrix_names", aggregate.requiredArtifactGeneratedMatrixNames, aggregate.missingArtifactGeneratedMatrixNames)
  appendAggregateRequirementLine(lines, "required_artifact_generated_matrix_repos", aggregate.requiredArtifactGeneratedMatrixRepos, aggregate.missingArtifactGeneratedMatrixRepos)
  appendAggregateRequirementLine(lines, "required_artifact_evidence_repos", aggregate.requiredArtifactEvidenceRepos, aggregate.missingArtifactEvidenceRepos)
  appendAggregateRequirementLine(lines, "required_artifact_provider_account_aliases", aggregate.requiredArtifactProviderAccountAliases, aggregate.missingArtifactProviderAccountAliases)
  appendAggregateRequirementLine(lines, "required_artifact_validation_presets", aggregate.requiredArtifactValidationPresets, aggregate.missingArtifactValidationPresets)
  appendAggregateRequirementLine(lines, "required_artifact_runtime_signals", aggregate.requiredArtifactRuntimeSignals, aggregate.missingArtifactRuntimeSignals)
  appendAggregateRequirementLine(lines, "required_artifact_runtime_signal_owners", aggregate.requiredArtifactRuntimeSignalOwners, aggregate.missingArtifactRuntimeSignalOwners)
  appendAggregateRequirementLine(lines, "required_artifact_owners", aggregate.requiredArtifactOwners, aggregate.missingArtifactOwners)
  appendAggregateRequirementLine(lines, "required_artifact_classifications", aggregate.requiredArtifactClassifications, aggregate.missingArtifactClassifications)
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
  if (!["passed", "failed"].includes(aggregate.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(aggregate.status)}`)
  }
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
  validateGeneratedEvidencePathArray(aggregate.requiredArtifactGeneratedMatrixArtifactIndexes ?? [], `${source}.requiredArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedEvidencePathArray(aggregate.missingArtifactGeneratedMatrixArtifactIndexes ?? [], `${source}.missingArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationArray(aggregate.requiredArtifactGeneratedMatrixLimitations ?? [], `${source}.requiredArtifactGeneratedMatrixLimitations`)
  validateGeneratedMatrixLimitationArray(aggregate.missingArtifactGeneratedMatrixLimitations ?? [], `${source}.missingArtifactGeneratedMatrixLimitations`)
  validateStringArray(aggregate.requiredArtifactGeneratedMatrixNames ?? [], `${source}.requiredArtifactGeneratedMatrixNames`)
  validateStringArray(aggregate.missingArtifactGeneratedMatrixNames ?? [], `${source}.missingArtifactGeneratedMatrixNames`)
  validateArtifactEvidenceRepoArray(aggregate.requiredArtifactGeneratedMatrixRepos ?? [], `${source}.requiredArtifactGeneratedMatrixRepos`)
  validateArtifactEvidenceRepoArray(aggregate.missingArtifactGeneratedMatrixRepos ?? [], `${source}.missingArtifactGeneratedMatrixRepos`)
  validateArtifactEvidenceRepoArray(aggregate.requiredArtifactEvidenceRepos ?? [], `${source}.requiredArtifactEvidenceRepos`)
  validateArtifactEvidenceRepoArray(aggregate.missingArtifactEvidenceRepos ?? [], `${source}.missingArtifactEvidenceRepos`)
  validateProviderAccountAliasArray(aggregate.requiredArtifactProviderAccountAliases ?? [], `${source}.requiredArtifactProviderAccountAliases`)
  validateProviderAccountAliasArray(aggregate.missingArtifactProviderAccountAliases ?? [], `${source}.missingArtifactProviderAccountAliases`)
  validateArtifactValidationPresetArray(aggregate.requiredArtifactValidationPresets ?? [], `${source}.requiredArtifactValidationPresets`)
  validateArtifactValidationPresetArray(aggregate.missingArtifactValidationPresets ?? [], `${source}.missingArtifactValidationPresets`)
  validateRuntimeSignalArray(aggregate.requiredArtifactRuntimeSignals ?? [], `${source}.requiredArtifactRuntimeSignals`)
  validateRuntimeSignalArray(aggregate.missingArtifactRuntimeSignals ?? [], `${source}.missingArtifactRuntimeSignals`)
  validateRuntimeSignalOwnerArray(aggregate.requiredArtifactRuntimeSignalOwners ?? [], `${source}.requiredArtifactRuntimeSignalOwners`)
  validateRuntimeSignalOwnerArray(aggregate.missingArtifactRuntimeSignalOwners ?? [], `${source}.missingArtifactRuntimeSignalOwners`)
  validateStringArray(aggregate.requiredArtifactOwners ?? [], `${source}.requiredArtifactOwners`)
  validateStringArray(aggregate.missingArtifactOwners ?? [], `${source}.missingArtifactOwners`)
  validateStringArray(aggregate.requiredArtifactClassifications ?? [], `${source}.requiredArtifactClassifications`)
  validateStringArray(aggregate.missingArtifactClassifications ?? [], `${source}.missingArtifactClassifications`)
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
    requiredArtifactGeneratedMatrixArtifactIndexes: aggregate.requiredArtifactGeneratedMatrixArtifactIndexes ?? [],
    requiredArtifactGeneratedMatrixLimitations: aggregate.requiredArtifactGeneratedMatrixLimitations ?? [],
    requiredArtifactGeneratedMatrixNames: aggregate.requiredArtifactGeneratedMatrixNames ?? [],
    requiredArtifactGeneratedMatrixRepos: aggregate.requiredArtifactGeneratedMatrixRepos ?? [],
    requiredArtifactEvidenceRepos: aggregate.requiredArtifactEvidenceRepos ?? [],
    requiredArtifactProviderAccountAliases: aggregate.requiredArtifactProviderAccountAliases ?? [],
    requiredArtifactValidationPresets: aggregate.requiredArtifactValidationPresets ?? [],
    requiredArtifactRuntimeSignals: aggregate.requiredArtifactRuntimeSignals ?? [],
    requiredArtifactRuntimeSignalOwners: aggregate.requiredArtifactRuntimeSignalOwners ?? [],
    requiredArtifactOwners: aggregate.requiredArtifactOwners ?? [],
    requiredArtifactClassifications: aggregate.requiredArtifactClassifications ?? [],
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

function validationGateReportPlatformCoverage(report) {
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

function validationGateReportArtifactCoverage(report) {
  return {
    requiredArtifactCoverageAreas: [...(report.checks.artifacts.requiredArtifactCoverageAreas ?? [])],
    missingArtifactCoverageAreas: [...(report.checks.artifacts.missingArtifactCoverageAreas ?? [])],
    requiredArtifactSchemas: [...(report.checks.artifacts.requiredArtifactSchemas ?? [])],
    missingArtifactSchemas: [...(report.checks.artifacts.missingArtifactSchemas ?? [])],
    requiredArtifactKinds: [...(report.checks.artifacts.requiredArtifactKinds ?? [])],
    missingArtifactKinds: [...(report.checks.artifacts.missingArtifactKinds ?? [])],
    requiredArtifactGeneratedEvidenceKinds: [...(report.checks.artifacts.requiredArtifactGeneratedEvidenceKinds ?? [])],
    missingArtifactGeneratedEvidenceKinds: [...(report.checks.artifacts.missingArtifactGeneratedEvidenceKinds ?? [])],
    requiredArtifactGeneratedMatrixArtifactIndexes: [...(report.checks.artifacts.requiredArtifactGeneratedMatrixArtifactIndexes ?? [])],
    missingArtifactGeneratedMatrixArtifactIndexes: [...(report.checks.artifacts.missingArtifactGeneratedMatrixArtifactIndexes ?? [])],
    requiredArtifactGeneratedMatrixLimitations: [...(report.checks.artifacts.requiredArtifactGeneratedMatrixLimitations ?? [])],
    missingArtifactGeneratedMatrixLimitations: [...(report.checks.artifacts.missingArtifactGeneratedMatrixLimitations ?? [])],
    requiredArtifactGeneratedMatrixNames: [...(report.checks.artifacts.requiredArtifactGeneratedMatrixNames ?? [])],
    missingArtifactGeneratedMatrixNames: [...(report.checks.artifacts.missingArtifactGeneratedMatrixNames ?? [])],
    requiredArtifactGeneratedMatrixRepos: [...(report.checks.artifacts.requiredArtifactGeneratedMatrixRepos ?? [])],
    missingArtifactGeneratedMatrixRepos: [...(report.checks.artifacts.missingArtifactGeneratedMatrixRepos ?? [])],
    requiredArtifactEvidenceRepos: [...(report.checks.artifacts.requiredArtifactEvidenceRepos ?? [])],
    missingArtifactEvidenceRepos: [...(report.checks.artifacts.missingArtifactEvidenceRepos ?? [])],
    requiredArtifactProviderAccountAliases: [...(report.checks.artifacts.requiredArtifactProviderAccountAliases ?? [])],
    missingArtifactProviderAccountAliases: [...(report.checks.artifacts.missingArtifactProviderAccountAliases ?? [])],
    requiredArtifactValidationPresets: [...(report.checks.artifacts.requiredArtifactValidationPresets ?? [])],
    missingArtifactValidationPresets: [...(report.checks.artifacts.missingArtifactValidationPresets ?? [])],
    requiredArtifactRuntimeSignals: [...(report.checks.artifacts.requiredArtifactRuntimeSignals ?? [])],
    missingArtifactRuntimeSignals: [...(report.checks.artifacts.missingArtifactRuntimeSignals ?? [])],
    requiredArtifactRuntimeSignalOwners: [...(report.checks.artifacts.requiredArtifactRuntimeSignalOwners ?? [])],
    missingArtifactRuntimeSignalOwners: [...(report.checks.artifacts.missingArtifactRuntimeSignalOwners ?? [])],
    requiredArtifactOwners: [...(report.checks.artifacts.requiredArtifactOwners ?? [])],
    missingArtifactOwners: [...(report.checks.artifacts.missingArtifactOwners ?? [])],
    requiredArtifactClassifications: [...(report.checks.artifacts.requiredArtifactClassifications ?? [])],
    missingArtifactClassifications: [...(report.checks.artifacts.missingArtifactClassifications ?? [])],
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
    runtimeSignals: { ...(report.checks.artifacts.aggregate?.runtimeSignals ?? {}) },
    runtimeSignalOwners: { ...(report.checks.artifacts.aggregate?.runtimeSignalOwners ?? {}) },
    owners: { ...(report.checks.artifacts.aggregate?.owners ?? {}) },
    classifications: { ...(report.checks.artifacts.aggregate?.classifications ?? {}) },
    plannedOwners: { ...(report.checks.artifacts.aggregate?.plannedOwners ?? {}) },
    plannedClassifications: { ...(report.checks.artifacts.aggregate?.plannedClassifications ?? {}) },
    exitCriterionStatuses: { ...(report.checks.artifacts.aggregate?.exitCriterionStatuses ?? {}) },
    incompleteExitCriterionStatuses: { ...(report.checks.artifacts.aggregate?.incompleteExitCriterionStatuses ?? {}) },
    artifactKinds: { ...(report.checks.artifacts.aggregate?.artifactKinds ?? {}) },
    generatedEvidenceKinds: { ...(report.checks.artifacts.aggregate?.generatedEvidenceKinds ?? {}) },
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

function countValidationGateArtifactCoverage(coverage, artifactCoverage) {
  countStringValues(coverage.requiredArtifactSchemas, artifactCoverage.requiredArtifactSchemas)
  countStringValues(coverage.missingArtifactSchemas, artifactCoverage.missingArtifactSchemas)
  countStringValues(coverage.requiredArtifactKinds, artifactCoverage.requiredArtifactKinds)
  countStringValues(coverage.missingArtifactKinds, artifactCoverage.missingArtifactKinds)
  countStringValues(coverage.requiredArtifactGeneratedEvidenceKinds, artifactCoverage.requiredArtifactGeneratedEvidenceKinds)
  countStringValues(coverage.missingArtifactGeneratedEvidenceKinds, artifactCoverage.missingArtifactGeneratedEvidenceKinds)
  countStringValues(coverage.requiredArtifactGeneratedMatrixArtifactIndexes, artifactCoverage.requiredArtifactGeneratedMatrixArtifactIndexes)
  countStringValues(coverage.missingArtifactGeneratedMatrixArtifactIndexes, artifactCoverage.missingArtifactGeneratedMatrixArtifactIndexes)
  countStringValues(coverage.requiredArtifactGeneratedMatrixLimitations, artifactCoverage.requiredArtifactGeneratedMatrixLimitations)
  countStringValues(coverage.missingArtifactGeneratedMatrixLimitations, artifactCoverage.missingArtifactGeneratedMatrixLimitations)
  countStringValues(coverage.requiredArtifactGeneratedMatrixNames, artifactCoverage.requiredArtifactGeneratedMatrixNames)
  countStringValues(coverage.missingArtifactGeneratedMatrixNames, artifactCoverage.missingArtifactGeneratedMatrixNames)
  countStringValues(coverage.requiredArtifactGeneratedMatrixRepos, artifactCoverage.requiredArtifactGeneratedMatrixRepos)
  countStringValues(coverage.missingArtifactGeneratedMatrixRepos, artifactCoverage.missingArtifactGeneratedMatrixRepos)
  countStringValues(coverage.requiredArtifactEvidenceRepos, artifactCoverage.requiredArtifactEvidenceRepos)
  countStringValues(coverage.missingArtifactEvidenceRepos, artifactCoverage.missingArtifactEvidenceRepos)
  countStringValues(coverage.requiredArtifactProviderAccountAliases, artifactCoverage.requiredArtifactProviderAccountAliases)
  countStringValues(coverage.missingArtifactProviderAccountAliases, artifactCoverage.missingArtifactProviderAccountAliases)
  countStringValues(coverage.requiredArtifactValidationPresets, artifactCoverage.requiredArtifactValidationPresets)
  countStringValues(coverage.missingArtifactValidationPresets, artifactCoverage.missingArtifactValidationPresets)
  countStringValues(coverage.requiredArtifactRuntimeSignals, artifactCoverage.requiredArtifactRuntimeSignals)
  countStringValues(coverage.missingArtifactRuntimeSignals, artifactCoverage.missingArtifactRuntimeSignals)
  countStringValues(coverage.requiredArtifactRuntimeSignalOwners, artifactCoverage.requiredArtifactRuntimeSignalOwners)
  countStringValues(coverage.missingArtifactRuntimeSignalOwners, artifactCoverage.missingArtifactRuntimeSignalOwners)
  countStringValues(coverage.requiredArtifactOwners, artifactCoverage.requiredArtifactOwners)
  countStringValues(coverage.missingArtifactOwners, artifactCoverage.missingArtifactOwners)
  countStringValues(coverage.requiredArtifactClassifications, artifactCoverage.requiredArtifactClassifications)
  countStringValues(coverage.missingArtifactClassifications, artifactCoverage.missingArtifactClassifications)
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
  countObjectValues(coverage.artifactRuntimeSignals, artifactCoverage.runtimeSignals)
  countObjectValues(coverage.artifactRuntimeSignalOwners, artifactCoverage.runtimeSignalOwners)
  countObjectValues(coverage.artifactOwners, artifactCoverage.owners)
  countObjectValues(coverage.artifactClassifications, artifactCoverage.classifications)
  countObjectValues(coverage.artifactPlannedOwners, artifactCoverage.plannedOwners)
  countObjectValues(coverage.artifactPlannedClassifications, artifactCoverage.plannedClassifications)
  countObjectValues(coverage.artifactExitCriterionStatuses, artifactCoverage.exitCriterionStatuses)
  countObjectValues(coverage.artifactIncompleteExitCriterionStatuses, artifactCoverage.incompleteExitCriterionStatuses)
  countObjectValues(coverage.artifactKinds, artifactCoverage.artifactKinds)
  countObjectValues(coverage.artifactGeneratedEvidenceKinds, artifactCoverage.generatedEvidenceKinds)
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

function validationGateReportFailureCoverage(report) {
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

function validationGateReportMatrixCoverage(report) {
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

function countStringValues(counts, values) {
  for (const value of values) {
    counts.set(value, (counts.get(value) ?? 0) + 1)
  }
}

function countObjectValues(counts, values) {
  for (const [value, count] of Object.entries(values ?? {})) {
    counts.set(value, (counts.get(value) ?? 0) + count)
  }
}

function staleMatrixReportSourceLabels(staleMatrixReports) {
  return (staleMatrixReports ?? []).map((report) => report.source ?? "unknown")
}

function staleFailureManifestSourceLabels(staleFailureManifests) {
  return (staleFailureManifests ?? []).map((manifest) => manifest.source ?? "unknown")
}

function missingValidationGateAggregateRequirements(coverage, requirements) {
  return {
    missingPresets: missingCoverageRequirements(coverage.presets, requirements.requiredPresets ?? []),
    missingPlatformCoverageAreas: missingCoverageRequirements(coverage.requiredPlatformCoverageAreas, requirements.requiredPlatformCoverageAreas ?? []),
    missingArtifactCoverageAreas: missingCoverageRequirements(coverage.artifactCoverageAreas, requirements.requiredArtifactCoverageAreas ?? []),
    missingArtifactSchemas: missingCoverageRequirements(coverage.artifactSchemas, requirements.requiredArtifactSchemas ?? []),
    missingArtifactKinds: missingCoverageRequirements(coverage.artifactKinds, requirements.requiredArtifactKinds ?? []),
    missingArtifactGeneratedEvidenceKinds: missingCoverageRequirements(coverage.artifactGeneratedEvidenceKinds, requirements.requiredArtifactGeneratedEvidenceKinds ?? []),
    missingArtifactGeneratedMatrixArtifactIndexes: missingCoverageRequirements(coverage.artifactGeneratedMatrixArtifactIndexes, requirements.requiredArtifactGeneratedMatrixArtifactIndexes ?? []),
    missingArtifactGeneratedMatrixLimitations: missingCoverageRequirements(coverage.artifactGeneratedMatrixLimitations, requirements.requiredArtifactGeneratedMatrixLimitations ?? []),
    missingArtifactGeneratedMatrixNames: missingCoverageRequirements(coverage.artifactGeneratedMatrixNames, requirements.requiredArtifactGeneratedMatrixNames ?? []),
    missingArtifactGeneratedMatrixRepos: missingCoverageRequirements(coverage.artifactGeneratedMatrixRepos, requirements.requiredArtifactGeneratedMatrixRepos ?? []),
    missingArtifactEvidenceRepos: missingCoverageRequirements(coverage.artifactEvidenceRepos, requirements.requiredArtifactEvidenceRepos ?? []),
    missingArtifactProviderAccountAliases: missingCoverageRequirements(coverage.artifactProviderAccountAliases, requirements.requiredArtifactProviderAccountAliases ?? []),
    missingArtifactValidationPresets: missingCoverageRequirements(coverage.artifactValidationPresets, requirements.requiredArtifactValidationPresets ?? []),
    missingArtifactRuntimeSignals: missingCoverageRequirements(coverage.artifactRuntimeSignals, requirements.requiredArtifactRuntimeSignals ?? []),
    missingArtifactRuntimeSignalOwners: missingCoverageRequirements(coverage.artifactRuntimeSignalOwners, requirements.requiredArtifactRuntimeSignalOwners ?? []),
    missingArtifactOwners: missingCoverageRequirements(coverage.artifactOwners, requirements.requiredArtifactOwners ?? []),
    missingArtifactClassifications: missingCoverageRequirements(coverage.artifactClassifications, requirements.requiredArtifactClassifications ?? []),
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

function missingCoverageRequirements(counts, required) {
  const present = new Set(Object.keys(counts ?? {}))
  return required.filter((entry) => !present.has(entry))
}

function appendMissingValidationGateAggregateNextActions(nextActions, missing) {
  const specs = [
    ["missingPlatformCoverageAreas", "platform-bundle", "provide validation gate reports requiring platform coverage areas"],
    ["missingArtifactCoverageAreas", "artifact-coverage", "provide validation gate reports with artifact coverage areas"],
    ["missingArtifactKinds", "artifact-coverage", "provide validation gate reports with artifact kinds"],
    ["missingArtifactGeneratedEvidenceKinds", "generated-evidence", "provide validation gate artifact indexes with generated evidence kinds"],
    ["missingArtifactGeneratedMatrixArtifactIndexes", "generated-evidence", "provide validation gate artifact indexes with generated matrix artifact indexes"],
    ["missingArtifactGeneratedMatrixLimitations", "generated-evidence", "provide validation gate artifact indexes with generated matrix limitations"],
    ["missingArtifactGeneratedMatrixNames", "generated-evidence", "provide validation gate artifact indexes with generated matrix names"],
    ["missingArtifactGeneratedMatrixRepos", "generated-evidence", "provide validation gate artifact indexes with generated matrix repos"],
    ["missingArtifactEvidenceRepos", "artifact-coverage", "provide validation gate reports with artifact evidence repos"],
    ["missingArtifactProviderAccountAliases", "artifact-coverage", "provide validation gate reports with artifact provider account aliases"],
    ["missingArtifactValidationPresets", "artifact-coverage", "provide validation gate reports with artifact validation presets"],
    ["missingArtifactRuntimeSignals", "artifact-coverage", "provide validation gate reports with artifact runtime signals"],
    ["missingArtifactRuntimeSignalOwners", "artifact-coverage", "provide validation gate reports with artifact runtime signal owners"],
    ["missingArtifactOwners", "artifact-coverage", "provide validation gate reports with artifact owners"],
    ["missingArtifactClassifications", "artifact-coverage", "provide validation gate reports with artifact classifications"],
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

function appendMissingArtifactSchemaNextActions(nextActions, missingArtifactSchemas) {
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

function assertValidationGateAggregateMissingRequirementsMatch(aggregate, expected, source) {
  const fields = [
    "missingPresets",
    "missingPlatformCoverageAreas",
    "missingArtifactCoverageAreas",
    "missingArtifactSchemas",
    "missingArtifactKinds",
    "missingArtifactGeneratedEvidenceKinds",
    "missingArtifactGeneratedMatrixArtifactIndexes",
    "missingArtifactGeneratedMatrixLimitations",
    "missingArtifactGeneratedMatrixNames",
    "missingArtifactGeneratedMatrixRepos",
    "missingArtifactEvidenceRepos",
    "missingArtifactProviderAccountAliases",
    "missingArtifactValidationPresets",
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

function formatValidationGateCoverageCounts(coverage) {
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
    requiredArtifactGeneratedMatrixArtifactIndexes: countMapToObject(coverage.requiredArtifactGeneratedMatrixArtifactIndexes),
    missingArtifactGeneratedMatrixArtifactIndexes: countMapToObject(coverage.missingArtifactGeneratedMatrixArtifactIndexes),
    requiredArtifactGeneratedMatrixLimitations: countMapToObject(coverage.requiredArtifactGeneratedMatrixLimitations),
    missingArtifactGeneratedMatrixLimitations: countMapToObject(coverage.missingArtifactGeneratedMatrixLimitations),
    requiredArtifactGeneratedMatrixNames: countMapToObject(coverage.requiredArtifactGeneratedMatrixNames),
    missingArtifactGeneratedMatrixNames: countMapToObject(coverage.missingArtifactGeneratedMatrixNames),
    requiredArtifactGeneratedMatrixRepos: countMapToObject(coverage.requiredArtifactGeneratedMatrixRepos),
    missingArtifactGeneratedMatrixRepos: countMapToObject(coverage.missingArtifactGeneratedMatrixRepos),
    requiredArtifactEvidenceRepos: countMapToObject(coverage.requiredArtifactEvidenceRepos),
    missingArtifactEvidenceRepos: countMapToObject(coverage.missingArtifactEvidenceRepos),
    requiredArtifactProviderAccountAliases: countMapToObject(coverage.requiredArtifactProviderAccountAliases),
    missingArtifactProviderAccountAliases: countMapToObject(coverage.missingArtifactProviderAccountAliases),
    requiredArtifactValidationPresets: countMapToObject(coverage.requiredArtifactValidationPresets),
    missingArtifactValidationPresets: countMapToObject(coverage.missingArtifactValidationPresets),
    requiredArtifactRuntimeSignals: countMapToObject(coverage.requiredArtifactRuntimeSignals),
    missingArtifactRuntimeSignals: countMapToObject(coverage.missingArtifactRuntimeSignals),
    requiredArtifactRuntimeSignalOwners: countMapToObject(coverage.requiredArtifactRuntimeSignalOwners),
    missingArtifactRuntimeSignalOwners: countMapToObject(coverage.missingArtifactRuntimeSignalOwners),
    requiredArtifactOwners: countMapToObject(coverage.requiredArtifactOwners),
    missingArtifactOwners: countMapToObject(coverage.missingArtifactOwners),
    requiredArtifactClassifications: countMapToObject(coverage.requiredArtifactClassifications),
    missingArtifactClassifications: countMapToObject(coverage.missingArtifactClassifications),
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
    artifactPlannedOwners: countMapToObject(coverage.artifactPlannedOwners),
    artifactPlannedClassifications: countMapToObject(coverage.artifactPlannedClassifications),
    artifactExitCriterionStatuses: countMapToObject(coverage.artifactExitCriterionStatuses),
    artifactIncompleteExitCriterionStatuses: countMapToObject(coverage.artifactIncompleteExitCriterionStatuses),
    artifactKinds: countMapToObject(coverage.artifactKinds),
    artifactGeneratedEvidenceKinds: countMapToObject(coverage.artifactGeneratedEvidenceKinds),
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

function countMapToObject(counts) {
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

function formatValidationGateCoverageSummary(coverage) {
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
  appendCoverageLine(lines, "required_artifact_generated_matrix_artifact_indexes", coverage.requiredArtifactGeneratedMatrixArtifactIndexes)
  appendCoverageLine(lines, "missing_artifact_generated_matrix_artifact_indexes", coverage.missingArtifactGeneratedMatrixArtifactIndexes)
  appendCoverageLine(lines, "required_artifact_generated_matrix_limitations", coverage.requiredArtifactGeneratedMatrixLimitations)
  appendCoverageLine(lines, "missing_artifact_generated_matrix_limitations", coverage.missingArtifactGeneratedMatrixLimitations)
  appendCoverageLine(lines, "required_artifact_generated_matrix_names", coverage.requiredArtifactGeneratedMatrixNames)
  appendCoverageLine(lines, "missing_artifact_generated_matrix_names", coverage.missingArtifactGeneratedMatrixNames)
  appendCoverageLine(lines, "required_artifact_generated_matrix_repos", coverage.requiredArtifactGeneratedMatrixRepos)
  appendCoverageLine(lines, "missing_artifact_generated_matrix_repos", coverage.missingArtifactGeneratedMatrixRepos)
  appendCoverageLine(lines, "required_artifact_evidence_repos", coverage.requiredArtifactEvidenceRepos)
  appendCoverageLine(lines, "missing_artifact_evidence_repos", coverage.missingArtifactEvidenceRepos)
  appendCoverageLine(lines, "required_artifact_provider_account_aliases", coverage.requiredArtifactProviderAccountAliases)
  appendCoverageLine(lines, "missing_artifact_provider_account_aliases", coverage.missingArtifactProviderAccountAliases)
  appendCoverageLine(lines, "required_artifact_validation_presets", coverage.requiredArtifactValidationPresets)
  appendCoverageLine(lines, "missing_artifact_validation_presets", coverage.missingArtifactValidationPresets)
  appendCoverageLine(lines, "required_artifact_runtime_signals", coverage.requiredArtifactRuntimeSignals)
  appendCoverageLine(lines, "missing_artifact_runtime_signals", coverage.missingArtifactRuntimeSignals)
  appendCoverageLine(lines, "required_artifact_runtime_signal_owners", coverage.requiredArtifactRuntimeSignalOwners)
  appendCoverageLine(lines, "missing_artifact_runtime_signal_owners", coverage.missingArtifactRuntimeSignalOwners)
  appendCoverageLine(lines, "required_artifact_owners", coverage.requiredArtifactOwners)
  appendCoverageLine(lines, "missing_artifact_owners", coverage.missingArtifactOwners)
  appendCoverageLine(lines, "required_artifact_classifications", coverage.requiredArtifactClassifications)
  appendCoverageLine(lines, "missing_artifact_classifications", coverage.missingArtifactClassifications)
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
  appendCoverageLine(lines, "artifact_planned_owners", coverage.artifactPlannedOwners)
  appendCoverageLine(lines, "artifact_planned_classifications", coverage.artifactPlannedClassifications)
  appendCoverageLine(lines, "artifact_exit_criterion_statuses", coverage.artifactExitCriterionStatuses)
  appendCoverageLine(lines, "artifact_incomplete_exit_criterion_statuses", coverage.artifactIncompleteExitCriterionStatuses)
  appendCoverageLine(lines, "artifact_kinds", coverage.artifactKinds)
  appendCoverageLine(lines, "artifact_generated_evidence_kinds", coverage.artifactGeneratedEvidenceKinds)
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

function appendCoverageLine(lines, label, counts) {
  const entries = Object.entries(counts ?? {})
  if (entries.length > 0) {
    lines.push(`- ${label}: ${entries.map(([key, count]) => `${key}=${count}`).join(" ")}`)
  }
}

function appendAggregateRequirementLine(lines, label, required, missing) {
  if ((required ?? []).length > 0) {
    lines.push(`${label}=${required.join(",")} missing=${(missing ?? []).join(",") || "none"}`)
  }
}

function appendAggregateMatrixRuntimeSignalSources(lines, matrixRuntimeSignalSources, requiredMatrixRuntimeSignals) {
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

function formatMatrixRuntimeSignalSource(entry) {
  const report = entry.reportSource ? ` report=${entry.reportSource}` : ""
  const source = entry.source ? ` source=${entry.source}` : ""
  return `${entry.matrix}/${entry.id}(${entry.status})${source}${report}`
}

function validateValidationGateCoverageAggregate(coverage, source) {
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
  validateGeneratedEvidencePathCountObject(coverage.requiredArtifactGeneratedMatrixArtifactIndexes ?? {}, `${source}.requiredArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedEvidencePathCountObject(coverage.missingArtifactGeneratedMatrixArtifactIndexes ?? {}, `${source}.missingArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationCountObject(coverage.requiredArtifactGeneratedMatrixLimitations ?? {}, `${source}.requiredArtifactGeneratedMatrixLimitations`)
  validateGeneratedMatrixLimitationCountObject(coverage.missingArtifactGeneratedMatrixLimitations ?? {}, `${source}.missingArtifactGeneratedMatrixLimitations`)
  validateCountObject(coverage.requiredArtifactGeneratedMatrixNames ?? {}, `${source}.requiredArtifactGeneratedMatrixNames`)
  validateCountObject(coverage.missingArtifactGeneratedMatrixNames ?? {}, `${source}.missingArtifactGeneratedMatrixNames`)
  validateArtifactEvidenceRepoCountObject(coverage.requiredArtifactGeneratedMatrixRepos ?? {}, `${source}.requiredArtifactGeneratedMatrixRepos`)
  validateArtifactEvidenceRepoCountObject(coverage.missingArtifactGeneratedMatrixRepos ?? {}, `${source}.missingArtifactGeneratedMatrixRepos`)
  validateArtifactEvidenceRepoCountObject(coverage.requiredArtifactEvidenceRepos ?? {}, `${source}.requiredArtifactEvidenceRepos`)
  validateArtifactEvidenceRepoCountObject(coverage.missingArtifactEvidenceRepos ?? {}, `${source}.missingArtifactEvidenceRepos`)
  validateProviderAccountAliasCountObject(coverage.requiredArtifactProviderAccountAliases ?? {}, `${source}.requiredArtifactProviderAccountAliases`)
  validateProviderAccountAliasCountObject(coverage.missingArtifactProviderAccountAliases ?? {}, `${source}.missingArtifactProviderAccountAliases`)
  validateArtifactValidationPresetCountObject(coverage.requiredArtifactValidationPresets ?? {}, `${source}.requiredArtifactValidationPresets`)
  validateArtifactValidationPresetCountObject(coverage.missingArtifactValidationPresets ?? {}, `${source}.missingArtifactValidationPresets`)
  validateRuntimeSignalCountObject(coverage.requiredArtifactRuntimeSignals ?? {}, `${source}.requiredArtifactRuntimeSignals`)
  validateRuntimeSignalCountObject(coverage.missingArtifactRuntimeSignals ?? {}, `${source}.missingArtifactRuntimeSignals`)
  validateRuntimeSignalOwnerCountObject(coverage.requiredArtifactRuntimeSignalOwners ?? {}, `${source}.requiredArtifactRuntimeSignalOwners`)
  validateRuntimeSignalOwnerCountObject(coverage.missingArtifactRuntimeSignalOwners ?? {}, `${source}.missingArtifactRuntimeSignalOwners`)
  validateCountObject(coverage.requiredArtifactOwners ?? {}, `${source}.requiredArtifactOwners`)
  validateCountObject(coverage.missingArtifactOwners ?? {}, `${source}.missingArtifactOwners`)
  validateCountObject(coverage.requiredArtifactClassifications ?? {}, `${source}.requiredArtifactClassifications`)
  validateCountObject(coverage.missingArtifactClassifications ?? {}, `${source}.missingArtifactClassifications`)
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
  validateCountObject(coverage.artifactPlannedOwners ?? {}, `${source}.artifactPlannedOwners`)
  validateCountObject(coverage.artifactPlannedClassifications ?? {}, `${source}.artifactPlannedClassifications`)
  validateExitCriterionStatusCountObject(coverage.artifactExitCriterionStatuses ?? {}, `${source}.artifactExitCriterionStatuses`)
  validateExitCriterionStatusCountObject(coverage.artifactIncompleteExitCriterionStatuses ?? {}, `${source}.artifactIncompleteExitCriterionStatuses`)
  validateArtifactKindCountObject(coverage.artifactKinds ?? {}, `${source}.artifactKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.artifactGeneratedEvidenceKinds ?? {}, `${source}.artifactGeneratedEvidenceKinds`)
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

function validateValidationGateMatrixCoverage(coverage, source) {
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

function validateValidationGateFailureCoverage(coverage, source) {
  if (!coverage || typeof coverage !== "object" || Array.isArray(coverage)) {
    throw new Error(`${source} is not an object`)
  }
  validateRuntimeSignalCountObject(coverage.runtimeSignals ?? {}, `${source}.runtimeSignals`)
  validateRuntimeSignalOwnerCountsMatch(coverage.runtimeSignals ?? {}, coverage.runtimeSignalOwners ?? {}, `${source}.runtimeSignalOwners`)
  validateCountObject(coverage.owners ?? {}, `${source}.owners`)
  validateFailureClassificationCountObject(coverage.classifications ?? {}, `${source}.classifications`)
  validateStaleFailureManifestSummaries(coverage.staleFailureManifests ?? [], `${source}.staleFailureManifests`)
}

function validateStaleMatrixReportSummaries(reports, source) {
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

function validateStaleFailureManifestSummaries(manifests, source) {
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

function validateValidationGatePlatformCoverage(coverage, source) {
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

function assertValidationGateCoverageMatchesReports(aggregate, source) {
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
    requiredArtifactGeneratedMatrixArtifactIndexes: new Map(),
    missingArtifactGeneratedMatrixArtifactIndexes: new Map(),
    requiredArtifactGeneratedMatrixLimitations: new Map(),
    missingArtifactGeneratedMatrixLimitations: new Map(),
    requiredArtifactGeneratedMatrixNames: new Map(),
    missingArtifactGeneratedMatrixNames: new Map(),
    requiredArtifactGeneratedMatrixRepos: new Map(),
    missingArtifactGeneratedMatrixRepos: new Map(),
    requiredArtifactEvidenceRepos: new Map(),
    missingArtifactEvidenceRepos: new Map(),
    requiredArtifactProviderAccountAliases: new Map(),
    missingArtifactProviderAccountAliases: new Map(),
    requiredArtifactValidationPresets: new Map(),
    missingArtifactValidationPresets: new Map(),
    requiredArtifactRuntimeSignals: new Map(),
    missingArtifactRuntimeSignals: new Map(),
    requiredArtifactRuntimeSignalOwners: new Map(),
    missingArtifactRuntimeSignalOwners: new Map(),
    requiredArtifactOwners: new Map(),
    missingArtifactOwners: new Map(),
    requiredArtifactClassifications: new Map(),
    missingArtifactClassifications: new Map(),
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
    artifactPlannedOwners: new Map(),
    artifactPlannedClassifications: new Map(),
    artifactExitCriterionStatuses: new Map(),
    artifactIncompleteExitCriterionStatuses: new Map(),
    artifactKinds: new Map(),
    artifactGeneratedEvidenceKinds: new Map(),
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
    countStringValues(expected.requiredArtifactGeneratedMatrixArtifactIndexes, report.artifactCoverage?.requiredArtifactGeneratedMatrixArtifactIndexes ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixArtifactIndexes, report.artifactCoverage?.missingArtifactGeneratedMatrixArtifactIndexes ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixLimitations, report.artifactCoverage?.requiredArtifactGeneratedMatrixLimitations ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixLimitations, report.artifactCoverage?.missingArtifactGeneratedMatrixLimitations ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixNames, report.artifactCoverage?.requiredArtifactGeneratedMatrixNames ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixNames, report.artifactCoverage?.missingArtifactGeneratedMatrixNames ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixRepos, report.artifactCoverage?.requiredArtifactGeneratedMatrixRepos ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixRepos, report.artifactCoverage?.missingArtifactGeneratedMatrixRepos ?? [])
    countStringValues(expected.requiredArtifactEvidenceRepos, report.artifactCoverage?.requiredArtifactEvidenceRepos ?? [])
    countStringValues(expected.missingArtifactEvidenceRepos, report.artifactCoverage?.missingArtifactEvidenceRepos ?? [])
    countStringValues(expected.requiredArtifactProviderAccountAliases, report.artifactCoverage?.requiredArtifactProviderAccountAliases ?? [])
    countStringValues(expected.missingArtifactProviderAccountAliases, report.artifactCoverage?.missingArtifactProviderAccountAliases ?? [])
    countStringValues(expected.requiredArtifactValidationPresets, report.artifactCoverage?.requiredArtifactValidationPresets ?? [])
    countStringValues(expected.missingArtifactValidationPresets, report.artifactCoverage?.missingArtifactValidationPresets ?? [])
    countStringValues(expected.requiredArtifactRuntimeSignals, report.artifactCoverage?.requiredArtifactRuntimeSignals ?? [])
    countStringValues(expected.missingArtifactRuntimeSignals, report.artifactCoverage?.missingArtifactRuntimeSignals ?? [])
    countStringValues(expected.requiredArtifactRuntimeSignalOwners, report.artifactCoverage?.requiredArtifactRuntimeSignalOwners ?? [])
    countStringValues(expected.missingArtifactRuntimeSignalOwners, report.artifactCoverage?.missingArtifactRuntimeSignalOwners ?? [])
    countStringValues(expected.requiredArtifactOwners, report.artifactCoverage?.requiredArtifactOwners ?? [])
    countStringValues(expected.missingArtifactOwners, report.artifactCoverage?.missingArtifactOwners ?? [])
    countStringValues(expected.requiredArtifactClassifications, report.artifactCoverage?.requiredArtifactClassifications ?? [])
    countStringValues(expected.missingArtifactClassifications, report.artifactCoverage?.missingArtifactClassifications ?? [])
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
    countObjectValues(expected.artifactRuntimeSignals, report.artifactCoverage?.runtimeSignals)
    countObjectValues(expected.artifactRuntimeSignalOwners, report.artifactCoverage?.runtimeSignalOwners)
    countObjectValues(expected.artifactOwners, report.artifactCoverage?.owners)
    countObjectValues(expected.artifactClassifications, report.artifactCoverage?.classifications)
    countObjectValues(expected.artifactPlannedOwners, report.artifactCoverage?.plannedOwners)
    countObjectValues(expected.artifactPlannedClassifications, report.artifactCoverage?.plannedClassifications)
    countObjectValues(expected.artifactExitCriterionStatuses, report.artifactCoverage?.exitCriterionStatuses)
    countObjectValues(expected.artifactIncompleteExitCriterionStatuses, report.artifactCoverage?.incompleteExitCriterionStatuses)
    countObjectValues(expected.artifactKinds, report.artifactCoverage?.artifactKinds)
    countObjectValues(expected.artifactGeneratedEvidenceKinds, report.artifactCoverage?.generatedEvidenceKinds)
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
    countStringValues(expected.requiredArtifactGeneratedMatrixArtifactIndexes, input.artifactCoverage?.requiredArtifactGeneratedMatrixArtifactIndexes ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixArtifactIndexes, input.artifactCoverage?.missingArtifactGeneratedMatrixArtifactIndexes ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixLimitations, input.artifactCoverage?.requiredArtifactGeneratedMatrixLimitations ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixLimitations, input.artifactCoverage?.missingArtifactGeneratedMatrixLimitations ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixNames, input.artifactCoverage?.requiredArtifactGeneratedMatrixNames ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixNames, input.artifactCoverage?.missingArtifactGeneratedMatrixNames ?? [])
    countStringValues(expected.requiredArtifactGeneratedMatrixRepos, input.artifactCoverage?.requiredArtifactGeneratedMatrixRepos ?? [])
    countStringValues(expected.missingArtifactGeneratedMatrixRepos, input.artifactCoverage?.missingArtifactGeneratedMatrixRepos ?? [])
    countStringValues(expected.requiredArtifactEvidenceRepos, input.artifactCoverage?.requiredArtifactEvidenceRepos ?? [])
    countStringValues(expected.missingArtifactEvidenceRepos, input.artifactCoverage?.missingArtifactEvidenceRepos ?? [])
    countStringValues(expected.requiredArtifactProviderAccountAliases, input.artifactCoverage?.requiredArtifactProviderAccountAliases ?? [])
    countStringValues(expected.missingArtifactProviderAccountAliases, input.artifactCoverage?.missingArtifactProviderAccountAliases ?? [])
    countStringValues(expected.requiredArtifactValidationPresets, input.artifactCoverage?.requiredArtifactValidationPresets ?? [])
    countStringValues(expected.missingArtifactValidationPresets, input.artifactCoverage?.missingArtifactValidationPresets ?? [])
    countStringValues(expected.requiredArtifactRuntimeSignals, input.artifactCoverage?.requiredArtifactRuntimeSignals ?? [])
    countStringValues(expected.missingArtifactRuntimeSignals, input.artifactCoverage?.missingArtifactRuntimeSignals ?? [])
    countStringValues(expected.requiredArtifactRuntimeSignalOwners, input.artifactCoverage?.requiredArtifactRuntimeSignalOwners ?? [])
    countStringValues(expected.missingArtifactRuntimeSignalOwners, input.artifactCoverage?.missingArtifactRuntimeSignalOwners ?? [])
    countStringValues(expected.requiredArtifactOwners, input.artifactCoverage?.requiredArtifactOwners ?? [])
    countStringValues(expected.missingArtifactOwners, input.artifactCoverage?.missingArtifactOwners ?? [])
    countStringValues(expected.requiredArtifactClassifications, input.artifactCoverage?.requiredArtifactClassifications ?? [])
    countStringValues(expected.missingArtifactClassifications, input.artifactCoverage?.missingArtifactClassifications ?? [])
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
    countObjectValues(expected.artifactRuntimeSignals, input.artifactCoverage?.runtimeSignals)
    countObjectValues(expected.artifactRuntimeSignalOwners, input.artifactCoverage?.runtimeSignalOwners)
    countObjectValues(expected.artifactOwners, input.artifactCoverage?.owners)
    countObjectValues(expected.artifactClassifications, input.artifactCoverage?.classifications)
    countObjectValues(expected.artifactPlannedOwners, input.artifactCoverage?.plannedOwners)
    countObjectValues(expected.artifactPlannedClassifications, input.artifactCoverage?.plannedClassifications)
    countObjectValues(expected.artifactExitCriterionStatuses, input.artifactCoverage?.exitCriterionStatuses)
    countObjectValues(expected.artifactIncompleteExitCriterionStatuses, input.artifactCoverage?.incompleteExitCriterionStatuses)
    countObjectValues(expected.artifactKinds, input.artifactCoverage?.artifactKinds)
    countObjectValues(expected.artifactGeneratedEvidenceKinds, input.artifactCoverage?.generatedEvidenceKinds)
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

function assertMatrixRuntimeSignalSourcesMatchReports(aggregate, source) {
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

function validateGateAggregateReportSummary(report, source) {
  if (!report || typeof report !== "object" || Array.isArray(report)) {
    throw new Error(`${source} is not an object`)
  }
  if (report.source !== null && typeof report.source !== "string") {
    throw new Error(`${source} has invalid source`)
  }
  if (!["passed", "failed"].includes(report.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(report.status)}`)
  }
  validatePresetArray(report.presets ?? [], `${source}.presets`)
  if (!report.checks || typeof report.checks !== "object" || Array.isArray(report.checks)) {
    throw new Error(`${source} has invalid checks`)
  }
  for (const name of ["configuration", "platformBundle", "artifacts", "matrices", "failures"]) {
    if (!["passed", "failed", "skipped"].includes(report.checks[name])) {
      throw new Error(`${source}.checks has invalid ${name}`)
    }
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

function validateGateAggregateArtifactCoverageInput(input, source) {
  if (!input || typeof input !== "object" || Array.isArray(input)) {
    throw new Error(`${source} is not an object`)
  }
  if (input.source !== null && typeof input.source !== "string") {
    throw new Error(`${source} has invalid source`)
  }
  if (!["passed", "failed"].includes(input.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(input.status)}`)
  }
  if (!input.checks || typeof input.checks !== "object" || Array.isArray(input.checks)) {
    throw new Error(`${source} has invalid checks`)
  }
  for (const name of ["configuration", "platformBundle", "artifacts", "matrices", "failures"]) {
    if (!["passed", "failed", "skipped"].includes(input.checks[name])) {
      throw new Error(`${source}.checks has invalid ${name}`)
    }
  }
  validateValidationGateArtifactCoverage(input.artifactCoverage, `${source}.artifactCoverage`)
}

function validationGateReportGeneratedEvidence(report) {
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
          artifactIndexPath: commandRecord.artifactIndexPath,
          args: stringArray(commandRecord.args),
          cwd: commandRecord.cwd,
          matrix: commandRecord.matrix,
          repo: commandRecord.repo,
          reportPath: commandRecord.reportPath,
          scriptPath: commandRecord.scriptPath,
        }
      }),
    },
  }
}

function validateValidationGateGeneratedEvidenceSummary(generatedEvidence, source) {
  if (!generatedEvidence || typeof generatedEvidence !== "object" || Array.isArray(generatedEvidence)) {
    throw new Error(`${source} is not an object`)
  }
  validateGeneratedEvidenceKindArray(generatedEvidence.kinds ?? [], `${source}.kinds`)
  validateGeneratedValidationSuitesSummary(generatedEvidence.validationSuites, `${source}.validationSuites`)
  validateGeneratedMatrixReportsSummary(generatedEvidence.matrixReports, `${source}.matrixReports`)
}

function validateGeneratedValidationSuitesSummary(validationSuites, source) {
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

function validateGeneratedValidationSuiteCommandSummary(command, source) {
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
}

function validateGeneratedMatrixReportsSummary(matrixReports, source) {
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

function validateGeneratedMatrixLimitations(limitations, source) {
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

function validateGeneratedMatrixCommandSummary(command, source) {
  if (!command || typeof command !== "object" || Array.isArray(command)) {
    throw new Error(`${source} is not an object`)
  }
  validateDrillGeneratedMatrixCommandMetadata(command, source)
  for (const key of ["artifactIndexPath", "cwd", "reportPath", "scriptPath"]) {
    if (!nonEmptyString(command[key])) {
      throw new Error(`${source} has invalid ${key}`)
    }
    validateGeneratedEvidencePathText(command[key], `${source}.${key}`)
  }
  validateGeneratedEvidencePathArray(command.args ?? [], `${source}.args`)
}

function validateValidationGateArtifactCoverage(coverage, source) {
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
  validateGeneratedEvidencePathArray(coverage.requiredArtifactGeneratedMatrixArtifactIndexes ?? [], `${source}.requiredArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedEvidencePathArray(coverage.missingArtifactGeneratedMatrixArtifactIndexes ?? [], `${source}.missingArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationArray(coverage.requiredArtifactGeneratedMatrixLimitations ?? [], `${source}.requiredArtifactGeneratedMatrixLimitations`)
  validateGeneratedMatrixLimitationArray(coverage.missingArtifactGeneratedMatrixLimitations ?? [], `${source}.missingArtifactGeneratedMatrixLimitations`)
  validateStringArray(coverage.requiredArtifactGeneratedMatrixNames ?? [], `${source}.requiredArtifactGeneratedMatrixNames`)
  validateStringArray(coverage.missingArtifactGeneratedMatrixNames ?? [], `${source}.missingArtifactGeneratedMatrixNames`)
  validateArtifactEvidenceRepoArray(coverage.requiredArtifactGeneratedMatrixRepos ?? [], `${source}.requiredArtifactGeneratedMatrixRepos`)
  validateArtifactEvidenceRepoArray(coverage.missingArtifactGeneratedMatrixRepos ?? [], `${source}.missingArtifactGeneratedMatrixRepos`)
  validateArtifactEvidenceRepoArray(coverage.requiredArtifactEvidenceRepos ?? [], `${source}.requiredArtifactEvidenceRepos`)
  validateArtifactEvidenceRepoArray(coverage.missingArtifactEvidenceRepos ?? [], `${source}.missingArtifactEvidenceRepos`)
  validateProviderAccountAliasArray(coverage.requiredArtifactProviderAccountAliases ?? [], `${source}.requiredArtifactProviderAccountAliases`)
  validateProviderAccountAliasArray(coverage.missingArtifactProviderAccountAliases ?? [], `${source}.missingArtifactProviderAccountAliases`)
  validateArtifactValidationPresetArray(coverage.requiredArtifactValidationPresets ?? [], `${source}.requiredArtifactValidationPresets`)
  validateArtifactValidationPresetArray(coverage.missingArtifactValidationPresets ?? [], `${source}.missingArtifactValidationPresets`)
  validateRuntimeSignalArray(coverage.requiredArtifactRuntimeSignals ?? [], `${source}.requiredArtifactRuntimeSignals`)
  validateRuntimeSignalArray(coverage.missingArtifactRuntimeSignals ?? [], `${source}.missingArtifactRuntimeSignals`)
  validateRuntimeSignalOwnerArray(coverage.requiredArtifactRuntimeSignalOwners ?? [], `${source}.requiredArtifactRuntimeSignalOwners`)
  validateRuntimeSignalOwnerArray(coverage.missingArtifactRuntimeSignalOwners ?? [], `${source}.missingArtifactRuntimeSignalOwners`)
  validateStringArray(coverage.requiredArtifactOwners ?? [], `${source}.requiredArtifactOwners`)
  validateStringArray(coverage.missingArtifactOwners ?? [], `${source}.missingArtifactOwners`)
  validateStringArray(coverage.requiredArtifactClassifications ?? [], `${source}.requiredArtifactClassifications`)
  validateStringArray(coverage.missingArtifactClassifications ?? [], `${source}.missingArtifactClassifications`)
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
  validateRuntimeSignalCountObject(coverage.runtimeSignals ?? {}, `${source}.runtimeSignals`)
  validateRuntimeSignalOwnerCountObject(coverage.runtimeSignalOwners ?? {}, `${source}.runtimeSignalOwners`)
  validateCountObject(coverage.owners ?? {}, `${source}.owners`)
  validateCountObject(coverage.classifications ?? {}, `${source}.classifications`)
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

function cloneRuntimeSignalScenarios(runtimeSignalScenarios) {
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

function appendMatrixRuntimeSignalSources(target, { reportSource, runtimeSignalScenarios }) {
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

function formatMatrixRuntimeSignalSources(sources) {
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

function compareMatrixRuntimeSignalSource(left, right) {
  return String(left.reportSource ?? "").localeCompare(String(right.reportSource ?? ""))
    || String(left.matrix ?? "").localeCompare(String(right.matrix ?? ""))
    || String(left.source ?? "").localeCompare(String(right.source ?? ""))
    || String(left.id ?? "").localeCompare(String(right.id ?? ""))
    || String(left.status ?? "").localeCompare(String(right.status ?? ""))
}

function validateMatrixRuntimeSignalSources(value, source) {
  validateRuntimeSignalScenarioMap(value, source, { reportSource: true })
}

function validateRuntimeSignalScenarioMap(value, source, { reportSource }) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is not an object`)
  }
  for (const [signal, scenarios] of Object.entries(value)) {
    if (!isKnownDrillRuntimeSignal(signal)) {
      throw new Error(`${source} has unknown runtime signal ${JSON.stringify(signal)}`)
    }
    if (!Array.isArray(scenarios)) {
      throw new Error(`${source}.${signal} is not an array`)
    }
    for (const [index, scenario] of scenarios.entries()) {
      validateRuntimeSignalScenario(scenario, `${source}.${signal}[${index}]`, { reportSource })
    }
  }
}

function validateRuntimeSignalScenario(scenario, source, { reportSource }) {
  if (!scenario || typeof scenario !== "object" || Array.isArray(scenario)) {
    throw new Error(`${source} is not an object`)
  }
  if (reportSource && scenario.reportSource !== null && scenario.reportSource !== undefined && !nonEmptyString(scenario.reportSource)) {
    throw new Error(`${source} has invalid reportSource`)
  }
  if (!nonEmptyString(scenario.matrix)) {
    throw new Error(`${source} is missing matrix`)
  }
  if (scenario.source !== null && scenario.source !== undefined && !nonEmptyString(scenario.source)) {
    throw new Error(`${source} has invalid source`)
  }
  if (!nonEmptyString(scenario.id)) {
    throw new Error(`${source} is missing id`)
  }
  if (!["passed", "failed", "skipped", "dry-run"].includes(scenario.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(scenario.status)}`)
  }
}

function validateRuntimeSignalArray(value, source) {
  validateStringArray(value, source)
  for (const [index, signal] of value.entries()) {
    if (!isKnownDrillRuntimeSignal(signal)) {
      throw new Error(`${source}[${index}] has unknown runtime signal ${JSON.stringify(signal)}`)
    }
  }
}

function validateRuntimeSignalOwnerArray(value, source) {
  validateStringArray(value, source)
  for (const [index, owner] of value.entries()) {
    if (!DRILL_RUNTIME_SIGNAL_OWNERS.includes(owner)) {
      throw new Error(`${source}[${index}] has unknown runtime signal owner ${JSON.stringify(owner)}`)
    }
  }
}

function validateFailureClassificationArray(value, source) {
  validateStringArray(value, source)
  for (const [index, classification] of value.entries()) {
    validateDrillFailureClassification(classification, `${source}[${index}]`, {
      label: "failure classification",
    })
  }
}

function validateArtifactEvidenceRepoArray(value, source) {
  validateStringArray(value, source)
  for (const [index, repo] of value.entries()) {
    validateDrillArtifactEvidenceRepo(repo, `${source}[${index}]`)
  }
}

function validateArtifactKindArray(value, source) {
  validateStringArray(value, source)
  for (const [index, kind] of value.entries()) {
    validateDrillArtifactKind(kind, `${source}[${index}]`)
  }
}

function validateProviderArray(value, source) {
  validateStringArray(value, source)
  for (const [index, provider] of value.entries()) {
    validateDrillProvider(provider, `${source}[${index}]`)
  }
}

function validateProviderAccountAliasArray(value, source) {
  validateStringArray(value, source)
  for (const [index, alias] of value.entries()) {
    const { provider } = parseProviderAccountAlias(alias)
    validateDrillProvider(provider, `${source}[${index}]`, {
      label: "provider account alias provider",
    })
  }
}

function validateArtifactValidationPresetArray(value, source) {
  validateStringArray(value, source)
  for (const [index, preset] of value.entries()) {
    if (!isKnownDrillArtifactValidationPreset(preset)) {
      throw new Error(`${source}[${index}] has unknown artifact validation preset ${JSON.stringify(preset)}`)
    }
  }
}

function validatePresetArray(value, source) {
  validateStringArray(value, source)
  for (const [index, preset] of value.entries()) {
    if (!isKnownDrillValidationGatePreset(preset)) {
      throw new Error(`${source}[${index}] has unknown validation gate preset ${JSON.stringify(preset)}`)
    }
  }
}

function validateDeploymentPresetArray(value, source) {
  validateStringArray(value, source)
  for (const [index, preset] of value.entries()) {
    if (!isKnownDrillDeploymentPreset(preset)) {
      throw new Error(`${source}[${index}] has unknown deployment preset ${JSON.stringify(preset)}`)
    }
  }
}

function validateGeneratedEvidenceKindArray(value, source) {
  validateStringArray(value, source)
  for (const [index, kind] of value.entries()) {
    validateDrillGeneratedEvidenceKind(kind, `${source}[${index}]`)
  }
}

function validateGeneratedMatrixLimitationArray(value, source) {
  validateStringArray(value, source)
  for (const [index, limitation] of value.entries()) {
    validateDrillGeneratedMatrixLimitation(limitation, `${source}[${index}]`)
  }
}

function validateExitCriterionStatusArray(value, source) {
  validateStringArray(value, source)
  for (const [index, status] of value.entries()) {
    if (!["satisfied", "failed", "skipped", "dry-run"].includes(status)) {
      throw new Error(`${source}[${index}] has unknown exit criterion status ${JSON.stringify(status)}`)
    }
  }
}

function validateStringArray(value, source) {
  if (!Array.isArray(value)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, entry] of value.entries()) {
    if (typeof entry !== "string") {
      throw new Error(`${source}[${index}] is not a string`)
    }
  }
}

function validateGeneratedEvidencePathArray(value, source) {
  validateStringArray(value, source)
  for (const [index, entry] of value.entries()) {
    validateGeneratedEvidencePathText(entry, `${source}[${index}]`)
  }
}

function validateGeneratedEvidencePathText(value, source) {
  validateDrillGeneratedEvidencePath(value, source)
}

function validateRuntimeSignalCountObject(value, source) {
  validateCountObject(value, source)
  for (const signal of Object.keys(value)) {
    if (!isKnownDrillRuntimeSignal(signal)) {
      throw new Error(`${source} has unknown runtime signal ${JSON.stringify(signal)}`)
    }
  }
}

function validateRuntimeSignalOwnerCountObject(value, source) {
  validateCountObject(value, source)
  for (const owner of Object.keys(value)) {
    if (!DRILL_RUNTIME_SIGNAL_OWNERS.includes(owner)) {
      throw new Error(`${source} has unknown runtime signal owner ${JSON.stringify(owner)}`)
    }
  }
}

function validateFailureClassificationCountObject(value, source) {
  validateCountObject(value, source)
  for (const classification of Object.keys(value)) {
    validateDrillFailureClassification(classification, source, {
      label: "failure classification",
    })
  }
}

function validateArtifactEvidenceRepoCountObject(value, source) {
  validateCountObject(value, source)
  for (const repo of Object.keys(value)) {
    validateDrillArtifactEvidenceRepo(repo, source)
  }
}

function validateArtifactKindCountObject(value, source) {
  validateCountObject(value, source)
  for (const kind of Object.keys(value)) {
    validateDrillArtifactKind(kind, source)
  }
}

function validateExitCriterionStatusCountObject(value, source) {
  validateCountObject(value, source)
  for (const status of Object.keys(value)) {
    if (!["satisfied", "failed", "skipped", "dry-run"].includes(status)) {
      throw new Error(`${source} has unknown exit criterion status ${JSON.stringify(status)}`)
    }
  }
}

function validateProviderCountObject(value, source) {
  validateCountObject(value, source)
  for (const provider of Object.keys(value)) {
    validateDrillProvider(provider, source)
  }
}

function validateProviderAccountAliasCountObject(value, source) {
  validateCountObject(value, source)
  for (const alias of Object.keys(value)) {
    const { provider } = parseProviderAccountAlias(alias)
    validateDrillProvider(provider, source, {
      label: "provider account alias provider",
    })
  }
}

function validateArtifactValidationPresetCountObject(value, source) {
  validateCountObject(value, source)
  for (const preset of Object.keys(value)) {
    if (!isKnownDrillArtifactValidationPreset(preset)) {
      throw new Error(`${source} has unknown artifact validation preset ${JSON.stringify(preset)}`)
    }
  }
}

function validatePresetCountObject(value, source) {
  validateCountObject(value, source)
  for (const preset of Object.keys(value)) {
    if (!isKnownDrillValidationGatePreset(preset)) {
      throw new Error(`${source} has unknown validation gate preset ${JSON.stringify(preset)}`)
    }
  }
}

function validateDeploymentPresetCountObject(value, source) {
  validateCountObject(value, source)
  for (const preset of Object.keys(value)) {
    if (!isKnownDrillDeploymentPreset(preset)) {
      throw new Error(`${source} has unknown deployment preset ${JSON.stringify(preset)}`)
    }
  }
}

function validateGeneratedEvidenceKindCountObject(value, source) {
  validateCountObject(value, source)
  for (const kind of Object.keys(value)) {
    validateDrillGeneratedEvidenceKind(kind, source)
  }
}

function validateGeneratedMatrixLimitationCountObject(value, source) {
  validateCountObject(value, source)
  for (const limitation of Object.keys(value)) {
    validateDrillGeneratedMatrixLimitation(limitation, source)
  }
}

function validateGeneratedEvidencePathCountObject(value, source) {
  validateCountObject(value, source)
  for (const key of Object.keys(value)) {
    validateGeneratedEvidencePathText(key, `${source}.${key}`)
  }
}

function validateRuntimeSignalOwnerCountsMatch(runtimeSignals, runtimeSignalOwners, source) {
  validateRuntimeSignalCountObject(runtimeSignals, source.replace(/\.runtimeSignalOwners$/, ".runtimeSignals"))
  validateRuntimeSignalOwnerCountObject(runtimeSignalOwners, source)
  const expected = drillRuntimeSignalOwnerCounts(runtimeSignals)
  if (JSON.stringify(runtimeSignalOwners) !== JSON.stringify(expected)) {
    throw new Error(`${source} must match runtimeSignals`)
  }
}

function validateCountObject(value, source) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is not an object`)
  }
  for (const [key, count] of Object.entries(value)) {
    if (!nonEmptyString(key) || !Number.isSafeInteger(count) || count < 0) {
      throw new Error(`${source} has invalid count for ${JSON.stringify(key)}`)
    }
  }
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}
