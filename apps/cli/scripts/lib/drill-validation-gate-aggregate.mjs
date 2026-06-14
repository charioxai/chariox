import {
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  validateDrillAggregateNextAction,
} from "./drill-aggregate-actions.mjs"
import { isKnownDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import { isKnownDrillGeneratedEvidenceKind } from "./drill-generated-evidence-kinds.mjs"
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
    requiredFailureClassifications: new Map(),
    missingFailureClassifications: new Map(),
    requiredArtifactSchemas: new Map(),
    missingArtifactSchemas: new Map(),
    requiredArtifactKinds: new Map(),
    missingArtifactKinds: new Map(),
    requiredArtifactGeneratedEvidenceKinds: new Map(),
    missingArtifactGeneratedEvidenceKinds: new Map(),
    requiredArtifactEvidenceRepos: new Map(),
    missingArtifactEvidenceRepos: new Map(),
    requiredArtifactRuntimeSignals: new Map(),
    missingArtifactRuntimeSignals: new Map(),
    requiredArtifactRuntimeSignalOwners: new Map(),
    missingArtifactRuntimeSignalOwners: new Map(),
    requiredArtifactOwners: new Map(),
    missingArtifactOwners: new Map(),
    requiredArtifactClassifications: new Map(),
    missingArtifactClassifications: new Map(),
    requiredArtifactCoverageAreas: new Map(),
    missingArtifactCoverageAreas: new Map(),
    artifactSchemas: new Map(),
    artifactCoverageAreas: new Map(),
    artifactRuntimeSignals: new Map(),
    artifactRuntimeSignalOwners: new Map(),
    artifactOwners: new Map(),
    artifactClassifications: new Map(),
    artifactKinds: new Map(),
    artifactGeneratedEvidenceKinds: new Map(),
    artifactEvidenceRepos: new Map(),
    failureRuntimeSignals: new Map(),
    failureRuntimeSignalOwners: new Map(),
    failureOwners: new Map(),
    failureClassifications: new Map(),
    matrixRuntimeSignals: new Map(),
    matrixRuntimeSignalOwners: new Map(),
    matrixOwners: new Map(),
    matrixClassifications: new Map(),
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
    requiredGeneratedEvidenceKinds: new Map(),
    missingGeneratedEvidenceKinds: new Map(),
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
    countStringValues(coverage.requiredFailureClassifications, platformCoverage.requiredFailureClassifications)
    countStringValues(coverage.missingFailureClassifications, platformCoverage.missingFailureClassifications)
    const artifactCoverage = validationGateReportArtifactCoverage(report)
    countStringValues(coverage.requiredArtifactSchemas, artifactCoverage.requiredArtifactSchemas)
    countStringValues(coverage.missingArtifactSchemas, artifactCoverage.missingArtifactSchemas)
    countStringValues(coverage.requiredArtifactKinds, artifactCoverage.requiredArtifactKinds)
    countStringValues(coverage.missingArtifactKinds, artifactCoverage.missingArtifactKinds)
    countStringValues(coverage.requiredArtifactGeneratedEvidenceKinds, artifactCoverage.requiredArtifactGeneratedEvidenceKinds)
    countStringValues(coverage.missingArtifactGeneratedEvidenceKinds, artifactCoverage.missingArtifactGeneratedEvidenceKinds)
    countStringValues(coverage.requiredArtifactEvidenceRepos, artifactCoverage.requiredArtifactEvidenceRepos)
    countStringValues(coverage.missingArtifactEvidenceRepos, artifactCoverage.missingArtifactEvidenceRepos)
    countStringValues(coverage.requiredArtifactRuntimeSignals, artifactCoverage.requiredArtifactRuntimeSignals)
    countStringValues(coverage.missingArtifactRuntimeSignals, artifactCoverage.missingArtifactRuntimeSignals)
    countStringValues(coverage.requiredArtifactRuntimeSignalOwners, artifactCoverage.requiredArtifactRuntimeSignalOwners)
    countStringValues(coverage.missingArtifactRuntimeSignalOwners, artifactCoverage.missingArtifactRuntimeSignalOwners)
    countStringValues(coverage.requiredArtifactOwners, artifactCoverage.requiredArtifactOwners)
    countStringValues(coverage.missingArtifactOwners, artifactCoverage.missingArtifactOwners)
    countStringValues(coverage.requiredArtifactClassifications, artifactCoverage.requiredArtifactClassifications)
    countStringValues(coverage.missingArtifactClassifications, artifactCoverage.missingArtifactClassifications)
    countStringValues(coverage.requiredArtifactCoverageAreas, artifactCoverage.requiredArtifactCoverageAreas)
    countStringValues(coverage.missingArtifactCoverageAreas, artifactCoverage.missingArtifactCoverageAreas)
    countObjectValues(coverage.artifactSchemas, artifactCoverage.schemas)
    countObjectValues(coverage.artifactCoverageAreas, artifactCoverage.coverageAreas)
    countObjectValues(coverage.artifactRuntimeSignals, artifactCoverage.runtimeSignals)
    countObjectValues(coverage.artifactRuntimeSignalOwners, artifactCoverage.runtimeSignalOwners)
    countObjectValues(coverage.artifactOwners, artifactCoverage.owners)
    countObjectValues(coverage.artifactClassifications, artifactCoverage.classifications)
    countObjectValues(coverage.artifactKinds, artifactCoverage.artifactKinds)
    countObjectValues(coverage.artifactGeneratedEvidenceKinds, artifactCoverage.generatedEvidenceKinds)
    countObjectValues(coverage.artifactEvidenceRepos, artifactCoverage.evidenceRepos)
    const failureCoverage = validationGateReportFailureCoverage(report)
    countObjectValues(coverage.failureRuntimeSignals, failureCoverage.runtimeSignals)
    countObjectValues(coverage.failureRuntimeSignalOwners, failureCoverage.runtimeSignalOwners)
    countObjectValues(coverage.failureOwners, failureCoverage.owners)
    countObjectValues(coverage.failureClassifications, failureCoverage.classifications)
    const matrixCoverage = validationGateReportMatrixCoverage(report)
    countObjectValues(coverage.matrixRuntimeSignals, matrixCoverage.runtimeSignals)
    countObjectValues(coverage.matrixRuntimeSignalOwners, matrixCoverage.runtimeSignalOwners)
    countObjectValues(coverage.matrixOwners, matrixCoverage.owners)
    countObjectValues(coverage.matrixClassifications, matrixCoverage.classifications)
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
  countStringValues(coverage.requiredGeneratedEvidenceKinds, normalizedAggregateRequirements.requiredGeneratedEvidenceKinds ?? [])
  countStringValues(
    coverage.missingGeneratedEvidenceKinds,
    missingCoverageRequirements(
      countMapToObject(coverage.generatedEvidenceKinds),
      normalizedAggregateRequirements.requiredGeneratedEvidenceKinds ?? [],
    ),
  )
  const coverageCounts = formatValidationGateCoverageCounts(coverage)
  const missingRequirements = missingValidationGateAggregateRequirements(coverageCounts, {
    ...normalizedAggregateRequirements,
    requiredPresets: normalizedRequiredPresets,
  })
  appendMissingValidationGateAggregateNextActions(nextActions, missingRequirements)
  const hasMissingRequirements = Object.values(missingRequirements).some((missing) => missing.length > 0)
  if (missingRequirements.missingPresets.length > 0) {
    countDrillAggregateNextAction(nextActions, {
      owner: "validation-harness",
      classification: "validation-gate",
      nextAction: `provide validation gate reports for presets: ${missingRequirements.missingPresets.join(", ")}`,
    })
  }
  const aggregate = {
    schema: DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
    status: totals.failed > 0 || hasMissingRequirements ? "failed" : "passed",
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
    requiredArtifactEvidenceRepos: normalizedAggregateRequirements.requiredArtifactEvidenceRepos,
    missingArtifactEvidenceRepos: missingRequirements.missingArtifactEvidenceRepos,
    requiredArtifactRuntimeSignals: normalizedAggregateRequirements.requiredArtifactRuntimeSignals,
    missingArtifactRuntimeSignals: missingRequirements.missingArtifactRuntimeSignals,
    requiredArtifactRuntimeSignalOwners: normalizedAggregateRequirements.requiredArtifactRuntimeSignalOwners,
    missingArtifactRuntimeSignalOwners: missingRequirements.missingArtifactRuntimeSignalOwners,
    requiredArtifactOwners: normalizedAggregateRequirements.requiredArtifactOwners,
    missingArtifactOwners: missingRequirements.missingArtifactOwners,
    requiredArtifactClassifications: normalizedAggregateRequirements.requiredArtifactClassifications,
    missingArtifactClassifications: missingRequirements.missingArtifactClassifications,
    requiredRuntimeSignals: normalizedAggregateRequirements.requiredRuntimeSignals,
    missingRuntimeSignals: missingRequirements.missingRuntimeSignals,
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
    matrixRuntimeSignalSources: formatMatrixRuntimeSignalSources(matrixRuntimeSignalSources),
    coverage: coverageCounts,
    nextActions: formatDrillAggregateNextActionCounts(nextActions),
    reports: summaries,
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
  appendAggregateRequirementLine(lines, "required_artifact_evidence_repos", aggregate.requiredArtifactEvidenceRepos, aggregate.missingArtifactEvidenceRepos)
  appendAggregateRequirementLine(lines, "required_artifact_runtime_signals", aggregate.requiredArtifactRuntimeSignals, aggregate.missingArtifactRuntimeSignals)
  appendAggregateRequirementLine(lines, "required_artifact_runtime_signal_owners", aggregate.requiredArtifactRuntimeSignalOwners, aggregate.missingArtifactRuntimeSignalOwners)
  appendAggregateRequirementLine(lines, "required_artifact_owners", aggregate.requiredArtifactOwners, aggregate.missingArtifactOwners)
  appendAggregateRequirementLine(lines, "required_artifact_classifications", aggregate.requiredArtifactClassifications, aggregate.missingArtifactClassifications)
  appendAggregateRequirementLine(lines, "required_runtime_signals", aggregate.requiredRuntimeSignals, aggregate.missingRuntimeSignals)
  appendAggregateRequirementLine(lines, "required_failure_classifications", aggregate.requiredFailureClassifications, aggregate.missingFailureClassifications)
  appendAggregateRequirementLine(lines, "required_matrices", aggregate.requiredMatrices, aggregate.missingMatrices)
  appendAggregateRequirementLine(lines, "required_matrix_classifications", aggregate.requiredMatrixClassifications, aggregate.missingMatrixClassifications)
  appendAggregateRequirementLine(lines, "required_matrix_runtime_signals", aggregate.requiredMatrixRuntimeSignals, aggregate.missingMatrixRuntimeSignals)
  appendAggregateMatrixRuntimeSignalSources(lines, aggregate.matrixRuntimeSignalSources, aggregate.requiredMatrixRuntimeSignals)
  appendAggregateRequirementLine(lines, "required_deployment_presets", aggregate.requiredDeploymentPresets, aggregate.missingDeploymentPresets)
  appendAggregateRequirementLine(lines, "required_providers", aggregate.requiredProviders, aggregate.missingProviders)
  appendAggregateRequirementLine(lines, "required_scenarios", aggregate.requiredScenarios, aggregate.missingScenarios)
  appendAggregateRequirementLine(lines, "required_generated_evidence_kinds", aggregate.requiredGeneratedEvidenceKinds, aggregate.missingGeneratedEvidenceKinds)
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
  validateStringArray(aggregate.requiredPresets ?? [], `${source}.requiredPresets`)
  validateStringArray(aggregate.missingPresets ?? [], `${source}.missingPresets`)
  validateStringArray(aggregate.requiredPlatformCoverageAreas ?? [], `${source}.requiredPlatformCoverageAreas`)
  validateStringArray(aggregate.missingPlatformCoverageAreas ?? [], `${source}.missingPlatformCoverageAreas`)
  validateStringArray(aggregate.requiredArtifactCoverageAreas ?? [], `${source}.requiredArtifactCoverageAreas`)
  validateStringArray(aggregate.missingArtifactCoverageAreas ?? [], `${source}.missingArtifactCoverageAreas`)
  validateStringArray(aggregate.requiredArtifactSchemas ?? [], `${source}.requiredArtifactSchemas`)
  validateStringArray(aggregate.missingArtifactSchemas ?? [], `${source}.missingArtifactSchemas`)
  validateStringArray(aggregate.requiredArtifactKinds ?? [], `${source}.requiredArtifactKinds`)
  validateStringArray(aggregate.missingArtifactKinds ?? [], `${source}.missingArtifactKinds`)
  validateGeneratedEvidenceKindArray(aggregate.requiredArtifactGeneratedEvidenceKinds ?? [], `${source}.requiredArtifactGeneratedEvidenceKinds`)
  validateGeneratedEvidenceKindArray(aggregate.missingArtifactGeneratedEvidenceKinds ?? [], `${source}.missingArtifactGeneratedEvidenceKinds`)
  validateArtifactEvidenceRepoArray(aggregate.requiredArtifactEvidenceRepos ?? [], `${source}.requiredArtifactEvidenceRepos`)
  validateArtifactEvidenceRepoArray(aggregate.missingArtifactEvidenceRepos ?? [], `${source}.missingArtifactEvidenceRepos`)
  validateRuntimeSignalArray(aggregate.requiredArtifactRuntimeSignals ?? [], `${source}.requiredArtifactRuntimeSignals`)
  validateRuntimeSignalArray(aggregate.missingArtifactRuntimeSignals ?? [], `${source}.missingArtifactRuntimeSignals`)
  validateStringArray(aggregate.requiredArtifactRuntimeSignalOwners ?? [], `${source}.requiredArtifactRuntimeSignalOwners`)
  validateStringArray(aggregate.missingArtifactRuntimeSignalOwners ?? [], `${source}.missingArtifactRuntimeSignalOwners`)
  validateStringArray(aggregate.requiredArtifactOwners ?? [], `${source}.requiredArtifactOwners`)
  validateStringArray(aggregate.missingArtifactOwners ?? [], `${source}.missingArtifactOwners`)
  validateStringArray(aggregate.requiredArtifactClassifications ?? [], `${source}.requiredArtifactClassifications`)
  validateStringArray(aggregate.missingArtifactClassifications ?? [], `${source}.missingArtifactClassifications`)
  validateRuntimeSignalArray(aggregate.requiredRuntimeSignals ?? [], `${source}.requiredRuntimeSignals`)
  validateRuntimeSignalArray(aggregate.missingRuntimeSignals ?? [], `${source}.missingRuntimeSignals`)
  validateStringArray(aggregate.requiredFailureClassifications ?? [], `${source}.requiredFailureClassifications`)
  validateStringArray(aggregate.missingFailureClassifications ?? [], `${source}.missingFailureClassifications`)
  validateStringArray(aggregate.requiredMatrices ?? [], `${source}.requiredMatrices`)
  validateStringArray(aggregate.missingMatrices ?? [], `${source}.missingMatrices`)
  validateStringArray(aggregate.requiredMatrixClassifications ?? [], `${source}.requiredMatrixClassifications`)
  validateStringArray(aggregate.missingMatrixClassifications ?? [], `${source}.missingMatrixClassifications`)
  validateRuntimeSignalArray(aggregate.requiredMatrixRuntimeSignals ?? [], `${source}.requiredMatrixRuntimeSignals`)
  validateRuntimeSignalArray(aggregate.missingMatrixRuntimeSignals ?? [], `${source}.missingMatrixRuntimeSignals`)
  if (aggregate.matrixRuntimeSignalSources !== undefined) {
    validateMatrixRuntimeSignalSources(aggregate.matrixRuntimeSignalSources, `${source}.matrixRuntimeSignalSources`)
  }
  validateStringArray(aggregate.requiredDeploymentPresets ?? [], `${source}.requiredDeploymentPresets`)
  validateStringArray(aggregate.missingDeploymentPresets ?? [], `${source}.missingDeploymentPresets`)
  validateStringArray(aggregate.requiredProviders ?? [], `${source}.requiredProviders`)
  validateStringArray(aggregate.missingProviders ?? [], `${source}.missingProviders`)
  validateStringArray(aggregate.requiredScenarios ?? [], `${source}.requiredScenarios`)
  validateStringArray(aggregate.missingScenarios ?? [], `${source}.missingScenarios`)
  validateGeneratedEvidenceKindArray(aggregate.requiredGeneratedEvidenceKinds ?? [], `${source}.requiredGeneratedEvidenceKinds`)
  validateGeneratedEvidenceKindArray(aggregate.missingGeneratedEvidenceKinds ?? [], `${source}.missingGeneratedEvidenceKinds`)
  for (const [index, action] of aggregate.nextActions.entries()) {
    validateDrillAggregateNextAction(action, `${source}.nextActions[${index}]`)
  }
  if (!Array.isArray(aggregate.reports)) {
    throw new Error(`${source} has invalid reports`)
  }
  for (const [index, report] of aggregate.reports.entries()) {
    validateGateAggregateReportSummary(report, `${source}.reports[${index}]`)
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
    requiredArtifactEvidenceRepos: aggregate.requiredArtifactEvidenceRepos ?? [],
    requiredArtifactRuntimeSignals: aggregate.requiredArtifactRuntimeSignals ?? [],
    requiredArtifactRuntimeSignalOwners: aggregate.requiredArtifactRuntimeSignalOwners ?? [],
    requiredArtifactOwners: aggregate.requiredArtifactOwners ?? [],
    requiredArtifactClassifications: aggregate.requiredArtifactClassifications ?? [],
    requiredRuntimeSignals: aggregate.requiredRuntimeSignals ?? [],
    requiredFailureClassifications: aggregate.requiredFailureClassifications ?? [],
    requiredMatrices: aggregate.requiredMatrices ?? [],
    requiredMatrixClassifications: aggregate.requiredMatrixClassifications ?? [],
    requiredMatrixRuntimeSignals: aggregate.requiredMatrixRuntimeSignals ?? [],
    requiredDeploymentPresets: aggregate.requiredDeploymentPresets ?? [],
    requiredProviders: aggregate.requiredProviders ?? [],
    requiredScenarios: aggregate.requiredScenarios ?? [],
    requiredGeneratedEvidenceKinds: aggregate.requiredGeneratedEvidenceKinds ?? [],
  })
  assertValidationGateAggregateMissingRequirementsMatch(aggregate, expectedMissingRequirements, source)
  const hasMissingRequirements = Object.values(expectedMissingRequirements).some((missing) => missing.length > 0)
  const expectedStatus = aggregate.totals.failed > 0 || hasMissingRequirements ? "failed" : "passed"
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
    requiredArtifactEvidenceRepos: [...(report.checks.artifacts.requiredArtifactEvidenceRepos ?? [])],
    missingArtifactEvidenceRepos: [...(report.checks.artifacts.missingArtifactEvidenceRepos ?? [])],
    requiredArtifactRuntimeSignals: [...(report.checks.artifacts.requiredArtifactRuntimeSignals ?? [])],
    missingArtifactRuntimeSignals: [...(report.checks.artifacts.missingArtifactRuntimeSignals ?? [])],
    requiredArtifactRuntimeSignalOwners: [...(report.checks.artifacts.requiredArtifactRuntimeSignalOwners ?? [])],
    missingArtifactRuntimeSignalOwners: [...(report.checks.artifacts.missingArtifactRuntimeSignalOwners ?? [])],
    requiredArtifactOwners: [...(report.checks.artifacts.requiredArtifactOwners ?? [])],
    missingArtifactOwners: [...(report.checks.artifacts.missingArtifactOwners ?? [])],
    requiredArtifactClassifications: [...(report.checks.artifacts.requiredArtifactClassifications ?? [])],
    missingArtifactClassifications: [...(report.checks.artifacts.missingArtifactClassifications ?? [])],
    schemas: { ...(report.checks.artifacts.aggregate?.schemas ?? {}) },
    coverageAreas: { ...(report.checks.artifacts.aggregate?.coverageAreas ?? {}) },
    runtimeSignals: { ...(report.checks.artifacts.aggregate?.runtimeSignals ?? {}) },
    runtimeSignalOwners: { ...(report.checks.artifacts.aggregate?.runtimeSignalOwners ?? {}) },
    owners: { ...(report.checks.artifacts.aggregate?.owners ?? {}) },
    classifications: { ...(report.checks.artifacts.aggregate?.classifications ?? {}) },
    artifactKinds: { ...(report.checks.artifacts.aggregate?.artifactKinds ?? {}) },
    generatedEvidenceKinds: { ...(report.checks.artifacts.aggregate?.generatedEvidenceKinds ?? {}) },
    evidenceRepos: { ...(report.checks.artifacts.aggregate?.evidenceRepos ?? {}) },
  }
}

function validationGateReportFailureCoverage(report) {
  const runtimeSignals = { ...(report.checks.failures.aggregate?.runtimeSignals ?? {}) }
  return {
    runtimeSignals,
    runtimeSignalOwners: drillRuntimeSignalOwnerCounts(runtimeSignals),
    owners: { ...(report.checks.failures.aggregate?.owners ?? {}) },
    classifications: { ...(report.checks.failures.aggregate?.classifications ?? {}) },
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

function missingValidationGateAggregateRequirements(coverage, requirements) {
  return {
    missingPresets: missingCoverageRequirements(coverage.presets, requirements.requiredPresets ?? []),
    missingPlatformCoverageAreas: missingCoverageRequirements(coverage.requiredPlatformCoverageAreas, requirements.requiredPlatformCoverageAreas ?? []),
    missingArtifactCoverageAreas: missingCoverageRequirements(coverage.artifactCoverageAreas, requirements.requiredArtifactCoverageAreas ?? []),
    missingArtifactSchemas: missingCoverageRequirements(coverage.artifactSchemas, requirements.requiredArtifactSchemas ?? []),
    missingArtifactKinds: missingCoverageRequirements(coverage.artifactKinds, requirements.requiredArtifactKinds ?? []),
    missingArtifactGeneratedEvidenceKinds: missingCoverageRequirements(coverage.artifactGeneratedEvidenceKinds, requirements.requiredArtifactGeneratedEvidenceKinds ?? []),
    missingArtifactEvidenceRepos: missingCoverageRequirements(coverage.artifactEvidenceRepos, requirements.requiredArtifactEvidenceRepos ?? []),
    missingArtifactRuntimeSignals: missingCoverageRequirements(coverage.artifactRuntimeSignals, requirements.requiredArtifactRuntimeSignals ?? []),
    missingArtifactRuntimeSignalOwners: missingCoverageRequirements(coverage.artifactRuntimeSignalOwners, requirements.requiredArtifactRuntimeSignalOwners ?? []),
    missingArtifactOwners: missingCoverageRequirements(coverage.artifactOwners, requirements.requiredArtifactOwners ?? []),
    missingArtifactClassifications: missingCoverageRequirements(coverage.artifactClassifications, requirements.requiredArtifactClassifications ?? []),
    missingRuntimeSignals: missingCoverageRequirements(coverage.requiredRuntimeSignals, requirements.requiredRuntimeSignals ?? []),
    missingFailureClassifications: missingCoverageRequirements(coverage.requiredFailureClassifications, requirements.requiredFailureClassifications ?? []),
    missingMatrices: missingCoverageRequirements(coverage.requiredMatrices, requirements.requiredMatrices ?? []),
    missingMatrixClassifications: missingCoverageRequirements(coverage.requiredMatrixClassifications, requirements.requiredMatrixClassifications ?? []),
    missingMatrixRuntimeSignals: missingCoverageRequirements(coverage.requiredMatrixRuntimeSignals, requirements.requiredMatrixRuntimeSignals ?? []),
    missingDeploymentPresets: missingCoverageRequirements(coverage.requiredDeploymentPresets, requirements.requiredDeploymentPresets ?? []),
    missingProviders: missingCoverageRequirements(coverage.requiredProviders, requirements.requiredProviders ?? []),
    missingScenarios: missingCoverageRequirements(coverage.requiredScenarios, requirements.requiredScenarios ?? []),
    missingGeneratedEvidenceKinds: missingCoverageRequirements(coverage.generatedEvidenceKinds, requirements.requiredGeneratedEvidenceKinds ?? []),
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
    ["missingArtifactEvidenceRepos", "artifact-coverage", "provide validation gate reports with artifact evidence repos"],
    ["missingArtifactRuntimeSignals", "artifact-coverage", "provide validation gate reports with artifact runtime signals"],
    ["missingArtifactRuntimeSignalOwners", "artifact-coverage", "provide validation gate reports with artifact runtime signal owners"],
    ["missingArtifactOwners", "artifact-coverage", "provide validation gate reports with artifact owners"],
    ["missingArtifactClassifications", "artifact-coverage", "provide validation gate reports with artifact classifications"],
    ["missingRuntimeSignals", "platform-bundle", "provide validation gate reports requiring runtime signals"],
    ["missingFailureClassifications", "platform-bundle", "provide validation gate reports requiring failure classifications"],
    ["missingMatrices", "matrix-coverage", "provide validation gate reports requiring matrices"],
    ["missingMatrixClassifications", "matrix-coverage", "provide validation gate reports requiring matrix classifications"],
    ["missingMatrixRuntimeSignals", "matrix-coverage", "provide validation gate reports requiring matrix runtime signals"],
    ["missingDeploymentPresets", "matrix-coverage", "provide validation gate reports requiring deployment presets"],
    ["missingProviders", "matrix-coverage", "provide validation gate reports requiring providers"],
    ["missingScenarios", "matrix-coverage", "provide validation gate reports requiring scenarios"],
    ["missingGeneratedEvidenceKinds", "generated-evidence", "provide validation gate reports with generated evidence kinds"],
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
    "missingArtifactEvidenceRepos",
    "missingArtifactRuntimeSignals",
    "missingArtifactRuntimeSignalOwners",
    "missingArtifactOwners",
    "missingArtifactClassifications",
    "missingRuntimeSignals",
    "missingFailureClassifications",
    "missingMatrices",
    "missingMatrixClassifications",
    "missingMatrixRuntimeSignals",
    "missingDeploymentPresets",
    "missingProviders",
    "missingScenarios",
    "missingGeneratedEvidenceKinds",
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
    requiredArtifactEvidenceRepos: countMapToObject(coverage.requiredArtifactEvidenceRepos),
    missingArtifactEvidenceRepos: countMapToObject(coverage.missingArtifactEvidenceRepos),
    requiredArtifactRuntimeSignals: countMapToObject(coverage.requiredArtifactRuntimeSignals),
    missingArtifactRuntimeSignals: countMapToObject(coverage.missingArtifactRuntimeSignals),
    requiredArtifactRuntimeSignalOwners: countMapToObject(coverage.requiredArtifactRuntimeSignalOwners),
    missingArtifactRuntimeSignalOwners: countMapToObject(coverage.missingArtifactRuntimeSignalOwners),
    requiredArtifactOwners: countMapToObject(coverage.requiredArtifactOwners),
    missingArtifactOwners: countMapToObject(coverage.missingArtifactOwners),
    requiredArtifactClassifications: countMapToObject(coverage.requiredArtifactClassifications),
    missingArtifactClassifications: countMapToObject(coverage.missingArtifactClassifications),
    artifactSchemas: countMapToObject(coverage.artifactSchemas),
    artifactCoverageAreas: countMapToObject(coverage.artifactCoverageAreas),
    requiredRuntimeSignals: countMapToObject(coverage.requiredRuntimeSignals),
    missingRuntimeSignals: countMapToObject(coverage.missingRuntimeSignals),
    requiredFailureClassifications: countMapToObject(coverage.requiredFailureClassifications),
    missingFailureClassifications: countMapToObject(coverage.missingFailureClassifications),
    artifactRuntimeSignals: countMapToObject(coverage.artifactRuntimeSignals),
    artifactRuntimeSignalOwners: countMapToObject(coverage.artifactRuntimeSignalOwners),
    artifactOwners: countMapToObject(coverage.artifactOwners),
    artifactClassifications: countMapToObject(coverage.artifactClassifications),
    artifactKinds: countMapToObject(coverage.artifactKinds),
    artifactGeneratedEvidenceKinds: countMapToObject(coverage.artifactGeneratedEvidenceKinds),
    artifactEvidenceRepos: countMapToObject(coverage.artifactEvidenceRepos),
    failureRuntimeSignals: countMapToObject(coverage.failureRuntimeSignals),
    failureRuntimeSignalOwners: countMapToObject(coverage.failureRuntimeSignalOwners),
    failureOwners: countMapToObject(coverage.failureOwners),
    failureClassifications: countMapToObject(coverage.failureClassifications),
    matrixRuntimeSignals: countMapToObject(coverage.matrixRuntimeSignals),
    matrixRuntimeSignalOwners: countMapToObject(coverage.matrixRuntimeSignalOwners),
    matrixOwners: countMapToObject(coverage.matrixOwners),
    matrixClassifications: countMapToObject(coverage.matrixClassifications),
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
    requiredGeneratedEvidenceKinds: countMapToObject(coverage.requiredGeneratedEvidenceKinds),
    missingGeneratedEvidenceKinds: countMapToObject(coverage.missingGeneratedEvidenceKinds),
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
  appendCoverageLine(lines, "required_artifact_evidence_repos", coverage.requiredArtifactEvidenceRepos)
  appendCoverageLine(lines, "missing_artifact_evidence_repos", coverage.missingArtifactEvidenceRepos)
  appendCoverageLine(lines, "required_artifact_runtime_signals", coverage.requiredArtifactRuntimeSignals)
  appendCoverageLine(lines, "missing_artifact_runtime_signals", coverage.missingArtifactRuntimeSignals)
  appendCoverageLine(lines, "required_artifact_runtime_signal_owners", coverage.requiredArtifactRuntimeSignalOwners)
  appendCoverageLine(lines, "missing_artifact_runtime_signal_owners", coverage.missingArtifactRuntimeSignalOwners)
  appendCoverageLine(lines, "required_artifact_owners", coverage.requiredArtifactOwners)
  appendCoverageLine(lines, "missing_artifact_owners", coverage.missingArtifactOwners)
  appendCoverageLine(lines, "required_artifact_classifications", coverage.requiredArtifactClassifications)
  appendCoverageLine(lines, "missing_artifact_classifications", coverage.missingArtifactClassifications)
  appendCoverageLine(lines, "artifact_schemas", coverage.artifactSchemas)
  appendCoverageLine(lines, "artifact_coverage_areas", coverage.artifactCoverageAreas)
  appendCoverageLine(lines, "required_runtime_signals", coverage.requiredRuntimeSignals)
  appendCoverageLine(lines, "missing_runtime_signals", coverage.missingRuntimeSignals)
  appendCoverageLine(lines, "required_failure_classifications", coverage.requiredFailureClassifications)
  appendCoverageLine(lines, "missing_failure_classifications", coverage.missingFailureClassifications)
  appendCoverageLine(lines, "artifact_runtime_signals", coverage.artifactRuntimeSignals)
  appendCoverageLine(lines, "artifact_runtime_signal_owners", coverage.artifactRuntimeSignalOwners)
  appendCoverageLine(lines, "artifact_owners", coverage.artifactOwners)
  appendCoverageLine(lines, "artifact_classifications", coverage.artifactClassifications)
  appendCoverageLine(lines, "artifact_kinds", coverage.artifactKinds)
  appendCoverageLine(lines, "artifact_generated_evidence_kinds", coverage.artifactGeneratedEvidenceKinds)
  appendCoverageLine(lines, "artifact_evidence_repos", coverage.artifactEvidenceRepos)
  appendCoverageLine(lines, "failure_runtime_signals", coverage.failureRuntimeSignals)
  appendCoverageLine(lines, "failure_runtime_signal_owners", coverage.failureRuntimeSignalOwners)
  appendCoverageLine(lines, "failure_owners", coverage.failureOwners)
  appendCoverageLine(lines, "failure_classifications", coverage.failureClassifications)
  appendCoverageLine(lines, "matrix_runtime_signals", coverage.matrixRuntimeSignals)
  appendCoverageLine(lines, "matrix_runtime_signal_owners", coverage.matrixRuntimeSignalOwners)
  appendCoverageLine(lines, "matrix_owners", coverage.matrixOwners)
  appendCoverageLine(lines, "matrix_classifications", coverage.matrixClassifications)
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
  appendCoverageLine(lines, "required_generated_evidence_kinds", coverage.requiredGeneratedEvidenceKinds)
  appendCoverageLine(lines, "missing_generated_evidence_kinds", coverage.missingGeneratedEvidenceKinds)
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
  validateCountObject(coverage.presets ?? {}, `${source}.presets`)
  validateCountObject(coverage.requiredPlatformCoverageAreas ?? {}, `${source}.requiredPlatformCoverageAreas`)
  validateCountObject(coverage.missingPlatformCoverageAreas ?? {}, `${source}.missingPlatformCoverageAreas`)
  validateCountObject(coverage.requiredArtifactCoverageAreas ?? {}, `${source}.requiredArtifactCoverageAreas`)
  validateCountObject(coverage.missingArtifactCoverageAreas ?? {}, `${source}.missingArtifactCoverageAreas`)
  validateCountObject(coverage.requiredArtifactSchemas ?? {}, `${source}.requiredArtifactSchemas`)
  validateCountObject(coverage.missingArtifactSchemas ?? {}, `${source}.missingArtifactSchemas`)
  validateCountObject(coverage.requiredArtifactKinds ?? {}, `${source}.requiredArtifactKinds`)
  validateCountObject(coverage.missingArtifactKinds ?? {}, `${source}.missingArtifactKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.requiredArtifactGeneratedEvidenceKinds ?? {}, `${source}.requiredArtifactGeneratedEvidenceKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.missingArtifactGeneratedEvidenceKinds ?? {}, `${source}.missingArtifactGeneratedEvidenceKinds`)
  validateArtifactEvidenceRepoCountObject(coverage.requiredArtifactEvidenceRepos ?? {}, `${source}.requiredArtifactEvidenceRepos`)
  validateArtifactEvidenceRepoCountObject(coverage.missingArtifactEvidenceRepos ?? {}, `${source}.missingArtifactEvidenceRepos`)
  validateRuntimeSignalCountObject(coverage.requiredArtifactRuntimeSignals ?? {}, `${source}.requiredArtifactRuntimeSignals`)
  validateRuntimeSignalCountObject(coverage.missingArtifactRuntimeSignals ?? {}, `${source}.missingArtifactRuntimeSignals`)
  validateRuntimeSignalOwnerCountObject(coverage.requiredArtifactRuntimeSignalOwners ?? {}, `${source}.requiredArtifactRuntimeSignalOwners`)
  validateRuntimeSignalOwnerCountObject(coverage.missingArtifactRuntimeSignalOwners ?? {}, `${source}.missingArtifactRuntimeSignalOwners`)
  validateCountObject(coverage.requiredArtifactOwners ?? {}, `${source}.requiredArtifactOwners`)
  validateCountObject(coverage.missingArtifactOwners ?? {}, `${source}.missingArtifactOwners`)
  validateCountObject(coverage.requiredArtifactClassifications ?? {}, `${source}.requiredArtifactClassifications`)
  validateCountObject(coverage.missingArtifactClassifications ?? {}, `${source}.missingArtifactClassifications`)
  validateCountObject(coverage.artifactSchemas ?? {}, `${source}.artifactSchemas`)
  validateCountObject(coverage.artifactCoverageAreas ?? {}, `${source}.artifactCoverageAreas`)
  validateRuntimeSignalCountObject(coverage.requiredRuntimeSignals ?? {}, `${source}.requiredRuntimeSignals`)
  validateRuntimeSignalCountObject(coverage.missingRuntimeSignals ?? {}, `${source}.missingRuntimeSignals`)
  validateCountObject(coverage.requiredFailureClassifications ?? {}, `${source}.requiredFailureClassifications`)
  validateCountObject(coverage.missingFailureClassifications ?? {}, `${source}.missingFailureClassifications`)
  validateRuntimeSignalCountObject(coverage.artifactRuntimeSignals ?? {}, `${source}.artifactRuntimeSignals`)
  validateRuntimeSignalOwnerCountObject(coverage.artifactRuntimeSignalOwners ?? {}, `${source}.artifactRuntimeSignalOwners`)
  validateCountObject(coverage.artifactOwners ?? {}, `${source}.artifactOwners`)
  validateCountObject(coverage.artifactClassifications ?? {}, `${source}.artifactClassifications`)
  validateCountObject(coverage.artifactKinds ?? {}, `${source}.artifactKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.artifactGeneratedEvidenceKinds ?? {}, `${source}.artifactGeneratedEvidenceKinds`)
  validateArtifactEvidenceRepoCountObject(coverage.artifactEvidenceRepos ?? {}, `${source}.artifactEvidenceRepos`)
  validateRuntimeSignalCountObject(coverage.failureRuntimeSignals ?? {}, `${source}.failureRuntimeSignals`)
  validateRuntimeSignalOwnerCountsMatch(coverage.failureRuntimeSignals ?? {}, coverage.failureRuntimeSignalOwners ?? {}, `${source}.failureRuntimeSignalOwners`)
  validateCountObject(coverage.failureOwners ?? {}, `${source}.failureOwners`)
  validateCountObject(coverage.failureClassifications ?? {}, `${source}.failureClassifications`)
  validateRuntimeSignalCountObject(coverage.matrixRuntimeSignals ?? {}, `${source}.matrixRuntimeSignals`)
  validateRuntimeSignalOwnerCountsMatch(coverage.matrixRuntimeSignals ?? {}, coverage.matrixRuntimeSignalOwners ?? {}, `${source}.matrixRuntimeSignalOwners`)
  validateCountObject(coverage.matrixOwners ?? {}, `${source}.matrixOwners`)
  validateCountObject(coverage.matrixClassifications ?? {}, `${source}.matrixClassifications`)
  validateCountObject(coverage.requiredMatrices ?? {}, `${source}.requiredMatrices`)
  validateCountObject(coverage.missingMatrices ?? {}, `${source}.missingMatrices`)
  validateCountObject(coverage.requiredMatrixClassifications ?? {}, `${source}.requiredMatrixClassifications`)
  validateCountObject(coverage.missingMatrixClassifications ?? {}, `${source}.missingMatrixClassifications`)
  validateRuntimeSignalCountObject(coverage.requiredMatrixRuntimeSignals ?? {}, `${source}.requiredMatrixRuntimeSignals`)
  validateRuntimeSignalCountObject(coverage.missingMatrixRuntimeSignals ?? {}, `${source}.missingMatrixRuntimeSignals`)
  validateCountObject(coverage.requiredDeploymentPresets ?? {}, `${source}.requiredDeploymentPresets`)
  validateCountObject(coverage.missingDeploymentPresets ?? {}, `${source}.missingDeploymentPresets`)
  validateCountObject(coverage.requiredProviders ?? {}, `${source}.requiredProviders`)
  validateCountObject(coverage.missingProviders ?? {}, `${source}.missingProviders`)
  validateCountObject(coverage.requiredScenarios ?? {}, `${source}.requiredScenarios`)
  validateCountObject(coverage.missingScenarios ?? {}, `${source}.missingScenarios`)
  validateGeneratedEvidenceKindCountObject(coverage.generatedEvidenceKinds ?? {}, `${source}.generatedEvidenceKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.requiredGeneratedEvidenceKinds ?? {}, `${source}.requiredGeneratedEvidenceKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.missingGeneratedEvidenceKinds ?? {}, `${source}.missingGeneratedEvidenceKinds`)
}

function validateValidationGateMatrixCoverage(coverage, source) {
  if (!coverage || typeof coverage !== "object" || Array.isArray(coverage)) {
    throw new Error(`${source} is not an object`)
  }
  validateRuntimeSignalCountObject(coverage.runtimeSignals ?? {}, `${source}.runtimeSignals`)
  validateRuntimeSignalOwnerCountsMatch(coverage.runtimeSignals ?? {}, coverage.runtimeSignalOwners ?? {}, `${source}.runtimeSignalOwners`)
  validateCountObject(coverage.owners ?? {}, `${source}.owners`)
  validateCountObject(coverage.classifications ?? {}, `${source}.classifications`)
  validateStringArray(coverage.requiredMatrices ?? [], `${source}.requiredMatrices`)
  validateStringArray(coverage.missingMatrices ?? [], `${source}.missingMatrices`)
  validateStringArray(coverage.requiredMatrixClassifications ?? [], `${source}.requiredMatrixClassifications`)
  validateStringArray(coverage.missingMatrixClassifications ?? [], `${source}.missingMatrixClassifications`)
  validateRuntimeSignalArray(coverage.requiredMatrixRuntimeSignals ?? [], `${source}.requiredMatrixRuntimeSignals`)
  validateRuntimeSignalArray(coverage.missingMatrixRuntimeSignals ?? [], `${source}.missingMatrixRuntimeSignals`)
  validateStringArray(coverage.requiredDeploymentPresets ?? [], `${source}.requiredDeploymentPresets`)
  validateStringArray(coverage.missingDeploymentPresets ?? [], `${source}.missingDeploymentPresets`)
  validateStringArray(coverage.requiredProviders ?? [], `${source}.requiredProviders`)
  validateStringArray(coverage.missingProviders ?? [], `${source}.missingProviders`)
  validateStringArray(coverage.requiredScenarios ?? [], `${source}.requiredScenarios`)
  validateStringArray(coverage.missingScenarios ?? [], `${source}.missingScenarios`)
  if (coverage.runtimeSignalScenarios !== undefined) {
    validateRuntimeSignalScenarioMap(coverage.runtimeSignalScenarios, `${source}.runtimeSignalScenarios`, { reportSource: false })
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
  validateStringArray(coverage.requiredFailureClassifications ?? [], `${source}.requiredFailureClassifications`)
  validateStringArray(coverage.missingFailureClassifications ?? [], `${source}.missingFailureClassifications`)
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
    requiredArtifactEvidenceRepos: new Map(),
    missingArtifactEvidenceRepos: new Map(),
    requiredArtifactRuntimeSignals: new Map(),
    missingArtifactRuntimeSignals: new Map(),
    requiredArtifactRuntimeSignalOwners: new Map(),
    missingArtifactRuntimeSignalOwners: new Map(),
    requiredArtifactOwners: new Map(),
    missingArtifactOwners: new Map(),
    requiredArtifactClassifications: new Map(),
    missingArtifactClassifications: new Map(),
    artifactSchemas: new Map(),
    artifactCoverageAreas: new Map(),
    requiredRuntimeSignals: new Map(),
    missingRuntimeSignals: new Map(),
    requiredFailureClassifications: new Map(),
    missingFailureClassifications: new Map(),
    artifactRuntimeSignals: new Map(),
    artifactRuntimeSignalOwners: new Map(),
    artifactOwners: new Map(),
    artifactClassifications: new Map(),
    artifactKinds: new Map(),
    artifactGeneratedEvidenceKinds: new Map(),
    artifactEvidenceRepos: new Map(),
    failureRuntimeSignals: new Map(),
    failureRuntimeSignalOwners: new Map(),
    failureOwners: new Map(),
    failureClassifications: new Map(),
    matrixRuntimeSignals: new Map(),
    matrixRuntimeSignalOwners: new Map(),
    matrixOwners: new Map(),
    matrixClassifications: new Map(),
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
    requiredGeneratedEvidenceKinds: new Map(),
    missingGeneratedEvidenceKinds: new Map(),
  }
  for (const report of aggregate.reports) {
    countStringValues(expected.presets, report.presets ?? [])
    const platformCoverage = report.platformCoverage ?? {
      requiredCoverageAreas: [],
      missingCoverageAreas: [],
      requiredRuntimeSignals: [],
      missingRuntimeSignals: [],
      requiredFailureClassifications: [],
      missingFailureClassifications: [],
    }
    countStringValues(expected.requiredPlatformCoverageAreas, platformCoverage.requiredCoverageAreas ?? [])
    countStringValues(expected.missingPlatformCoverageAreas, platformCoverage.missingCoverageAreas ?? [])
    countStringValues(expected.requiredRuntimeSignals, platformCoverage.requiredRuntimeSignals ?? [])
    countStringValues(expected.missingRuntimeSignals, platformCoverage.missingRuntimeSignals ?? [])
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
    countStringValues(expected.requiredArtifactEvidenceRepos, report.artifactCoverage?.requiredArtifactEvidenceRepos ?? [])
    countStringValues(expected.missingArtifactEvidenceRepos, report.artifactCoverage?.missingArtifactEvidenceRepos ?? [])
    countStringValues(expected.requiredArtifactRuntimeSignals, report.artifactCoverage?.requiredArtifactRuntimeSignals ?? [])
    countStringValues(expected.missingArtifactRuntimeSignals, report.artifactCoverage?.missingArtifactRuntimeSignals ?? [])
    countStringValues(expected.requiredArtifactRuntimeSignalOwners, report.artifactCoverage?.requiredArtifactRuntimeSignalOwners ?? [])
    countStringValues(expected.missingArtifactRuntimeSignalOwners, report.artifactCoverage?.missingArtifactRuntimeSignalOwners ?? [])
    countStringValues(expected.requiredArtifactOwners, report.artifactCoverage?.requiredArtifactOwners ?? [])
    countStringValues(expected.missingArtifactOwners, report.artifactCoverage?.missingArtifactOwners ?? [])
    countStringValues(expected.requiredArtifactClassifications, report.artifactCoverage?.requiredArtifactClassifications ?? [])
    countStringValues(expected.missingArtifactClassifications, report.artifactCoverage?.missingArtifactClassifications ?? [])
    countObjectValues(expected.artifactSchemas, report.artifactCoverage?.schemas)
    countObjectValues(expected.artifactCoverageAreas, report.artifactCoverage?.coverageAreas)
    countObjectValues(expected.artifactRuntimeSignals, report.artifactCoverage?.runtimeSignals)
    countObjectValues(expected.artifactRuntimeSignalOwners, report.artifactCoverage?.runtimeSignalOwners)
    countObjectValues(expected.artifactOwners, report.artifactCoverage?.owners)
    countObjectValues(expected.artifactClassifications, report.artifactCoverage?.classifications)
    countObjectValues(expected.artifactKinds, report.artifactCoverage?.artifactKinds)
    countObjectValues(expected.artifactGeneratedEvidenceKinds, report.artifactCoverage?.generatedEvidenceKinds)
    countObjectValues(expected.artifactEvidenceRepos, report.artifactCoverage?.evidenceRepos)
    countObjectValues(expected.failureRuntimeSignals, report.failureCoverage?.runtimeSignals)
    countObjectValues(expected.failureRuntimeSignalOwners, report.failureCoverage?.runtimeSignalOwners)
    countObjectValues(expected.failureOwners, report.failureCoverage?.owners)
    countObjectValues(expected.failureClassifications, report.failureCoverage?.classifications)
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
  }
  countStringValues(expected.requiredGeneratedEvidenceKinds, aggregate.requiredGeneratedEvidenceKinds ?? [])
  countStringValues(expected.missingGeneratedEvidenceKinds, aggregate.missingGeneratedEvidenceKinds ?? [])
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
  validateStringArray(report.presets ?? [], `${source}.presets`)
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
    validateValidationGateArtifactCoverage(report.failureCoverage, `${source}.failureCoverage`)
  }
  if (report.generatedEvidence !== undefined) {
    validateValidationGateGeneratedEvidenceSummary(report.generatedEvidence, `${source}.generatedEvidence`)
  }
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
      artifactIndexes: stringArray(validationSuites.artifactIndexes),
      outputRoots: stringArray(validationSuites.outputRoots),
    },
    matrixReports: {
      enabled: matrixReports.enabled === true,
      roots: stringArray(matrixReports.roots),
      dryRun: matrixReports.dryRun === true,
      continueOnFailure: matrixReports.continueOnFailure === true,
      commands: (Array.isArray(matrixReports.commands) ? matrixReports.commands : []).map((command) => {
        const commandRecord = command && typeof command === "object" && !Array.isArray(command) ? command : {}
        return {
          artifactIndexPath: commandRecord.artifactIndexPath,
          args: stringArray(commandRecord.args),
          cwd: commandRecord.cwd,
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
  validateStringArray(validationSuites.artifactIndexes ?? [], `${source}.artifactIndexes`)
  validateStringArray(validationSuites.outputRoots ?? [], `${source}.outputRoots`)
  if (validationSuites.enabled && ((validationSuites.artifactIndexes ?? []).length === 0 || (validationSuites.outputRoots ?? []).length === 0)) {
    throw new Error(`${source} enabled evidence is missing paths`)
  }
  if (!validationSuites.enabled && ((validationSuites.artifactIndexes ?? []).length > 0 || (validationSuites.outputRoots ?? []).length > 0)) {
    throw new Error(`${source} disabled evidence has paths`)
  }
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
  validateStringArray(matrixReports.roots ?? [], `${source}.roots`)
  if (!Array.isArray(matrixReports.commands)) {
    throw new Error(`${source}.commands is not an array`)
  }
  for (const [index, command] of matrixReports.commands.entries()) {
    validateGeneratedMatrixCommandSummary(command, `${source}.commands[${index}]`)
  }
  if (matrixReports.enabled && ((matrixReports.roots ?? []).length === 0 || matrixReports.commands.length === 0)) {
    throw new Error(`${source} enabled evidence is missing paths`)
  }
  if (!matrixReports.enabled && ((matrixReports.roots ?? []).length > 0 || matrixReports.commands.length > 0)) {
    throw new Error(`${source} disabled evidence has paths`)
  }
}

function validateGeneratedMatrixCommandSummary(command, source) {
  if (!command || typeof command !== "object" || Array.isArray(command)) {
    throw new Error(`${source} is not an object`)
  }
  for (const key of ["artifactIndexPath", "cwd", "reportPath", "scriptPath"]) {
    if (!nonEmptyString(command[key])) {
      throw new Error(`${source} has invalid ${key}`)
    }
  }
  validateStringArray(command.args ?? [], `${source}.args`)
}

function validateValidationGateArtifactCoverage(coverage, source) {
  if (!coverage || typeof coverage !== "object" || Array.isArray(coverage)) {
    throw new Error(`${source} is not an object`)
  }
  validateStringArray(coverage.requiredArtifactCoverageAreas ?? [], `${source}.requiredArtifactCoverageAreas`)
  validateStringArray(coverage.missingArtifactCoverageAreas ?? [], `${source}.missingArtifactCoverageAreas`)
  validateStringArray(coverage.requiredArtifactSchemas ?? [], `${source}.requiredArtifactSchemas`)
  validateStringArray(coverage.missingArtifactSchemas ?? [], `${source}.missingArtifactSchemas`)
  validateStringArray(coverage.requiredArtifactKinds ?? [], `${source}.requiredArtifactKinds`)
  validateStringArray(coverage.missingArtifactKinds ?? [], `${source}.missingArtifactKinds`)
  validateGeneratedEvidenceKindArray(coverage.requiredArtifactGeneratedEvidenceKinds ?? [], `${source}.requiredArtifactGeneratedEvidenceKinds`)
  validateGeneratedEvidenceKindArray(coverage.missingArtifactGeneratedEvidenceKinds ?? [], `${source}.missingArtifactGeneratedEvidenceKinds`)
  validateArtifactEvidenceRepoArray(coverage.requiredArtifactEvidenceRepos ?? [], `${source}.requiredArtifactEvidenceRepos`)
  validateArtifactEvidenceRepoArray(coverage.missingArtifactEvidenceRepos ?? [], `${source}.missingArtifactEvidenceRepos`)
  validateRuntimeSignalArray(coverage.requiredArtifactRuntimeSignals ?? [], `${source}.requiredArtifactRuntimeSignals`)
  validateRuntimeSignalArray(coverage.missingArtifactRuntimeSignals ?? [], `${source}.missingArtifactRuntimeSignals`)
  validateStringArray(coverage.requiredArtifactRuntimeSignalOwners ?? [], `${source}.requiredArtifactRuntimeSignalOwners`)
  validateStringArray(coverage.missingArtifactRuntimeSignalOwners ?? [], `${source}.missingArtifactRuntimeSignalOwners`)
  validateStringArray(coverage.requiredArtifactOwners ?? [], `${source}.requiredArtifactOwners`)
  validateStringArray(coverage.missingArtifactOwners ?? [], `${source}.missingArtifactOwners`)
  validateStringArray(coverage.requiredArtifactClassifications ?? [], `${source}.requiredArtifactClassifications`)
  validateStringArray(coverage.missingArtifactClassifications ?? [], `${source}.missingArtifactClassifications`)
  validateCountObject(coverage.schemas ?? {}, `${source}.schemas`)
  validateCountObject(coverage.coverageAreas ?? {}, `${source}.coverageAreas`)
  validateRuntimeSignalCountObject(coverage.runtimeSignals ?? {}, `${source}.runtimeSignals`)
  validateRuntimeSignalOwnerCountObject(coverage.runtimeSignalOwners ?? {}, `${source}.runtimeSignalOwners`)
  validateCountObject(coverage.owners ?? {}, `${source}.owners`)
  validateCountObject(coverage.classifications ?? {}, `${source}.classifications`)
  validateCountObject(coverage.artifactKinds ?? {}, `${source}.artifactKinds`)
  validateGeneratedEvidenceKindCountObject(coverage.generatedEvidenceKinds ?? {}, `${source}.generatedEvidenceKinds`)
  validateArtifactEvidenceRepoCountObject(coverage.evidenceRepos ?? {}, `${source}.evidenceRepos`)
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

function validateArtifactEvidenceRepoArray(value, source) {
  validateStringArray(value, source)
  for (const [index, repo] of value.entries()) {
    if (!isKnownDrillArtifactEvidenceRepo(repo)) {
      throw new Error(`${source}[${index}] has unknown evidence repo ${JSON.stringify(repo)}`)
    }
  }
}

function validateGeneratedEvidenceKindArray(value, source) {
  validateStringArray(value, source)
  for (const [index, kind] of value.entries()) {
    if (!isKnownDrillGeneratedEvidenceKind(kind)) {
      throw new Error(`${source}[${index}] has unknown generated evidence kind ${JSON.stringify(kind)}`)
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

function validateArtifactEvidenceRepoCountObject(value, source) {
  validateCountObject(value, source)
  for (const repo of Object.keys(value)) {
    if (!isKnownDrillArtifactEvidenceRepo(repo)) {
      throw new Error(`${source} has unknown evidence repo ${JSON.stringify(repo)}`)
    }
  }
}

function validateGeneratedEvidenceKindCountObject(value, source) {
  validateCountObject(value, source)
  for (const kind of Object.keys(value)) {
    if (!isKnownDrillGeneratedEvidenceKind(kind)) {
      throw new Error(`${source} has unknown generated evidence kind ${JSON.stringify(kind)}`)
    }
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
