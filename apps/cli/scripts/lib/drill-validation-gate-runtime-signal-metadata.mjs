import { drillRuntimeSignalOwnersFor } from "./drill-runtime-signals.mjs"
import { drillFailureOwnerForClassification } from "./drill-failure-taxonomy.mjs"

export function runtimeSignalMetadataForValidationGateReport(report) {
  const signals = new Set([
    ...platformRuntimeSignals(report),
    ...Object.keys(report.checks?.artifacts?.aggregate?.runtimeSignals ?? {}),
    ...Object.keys(report.checks?.failures?.aggregate?.runtimeSignals ?? {}),
    ...Object.keys(report.checks?.matrices?.aggregate?.runtimeSignals ?? {}),
    ...Object.keys(report.checks?.matrices?.aggregate?.runtimeSignalScenarios ?? {}),
  ])
  const signalOwners = new Set([
    ...drillRuntimeSignalOwnersFor([...signals]),
    ...platformRuntimeSignalOwners(report),
    ...Object.keys(report.checks?.artifacts?.aggregate?.runtimeSignalOwners ?? {}),
  ])
  return signals.size > 0
    ? {
      runtimeSignals: [...signals].sort().join(","),
      ...(signalOwners.size > 0 ? { runtimeSignalOwners: [...signalOwners].sort().join(",") } : {}),
    }
    : {}
}

export function diagnosticMetadataForValidationGateReport(report) {
  const coverageAreas = new Set([
    ...platformCoverageAreas(report),
    ...Object.keys(report.checks?.artifacts?.aggregate?.coverageAreas ?? {}),
  ])
  const platformClassifications = platformFailureClassifications(report)
  const owners = new Set([
    ...platformClassifications.map((classification) => drillFailureOwnerForClassification(classification)),
    ...Object.keys(report.checks?.artifacts?.aggregate?.owners ?? {}),
    ...Object.keys(report.checks?.failures?.aggregate?.owners ?? {}),
    ...Object.keys(report.checks?.matrices?.aggregate?.owners ?? {}),
    ...(report.nextActions ?? []).map((action) => action.owner).filter(nonEmptyString),
  ])
  const classifications = new Set([
    ...platformClassifications,
    ...Object.keys(report.checks?.artifacts?.aggregate?.classifications ?? {}),
    ...Object.keys(report.checks?.failures?.aggregate?.classifications ?? {}),
    ...Object.keys(report.checks?.matrices?.aggregate?.classifications ?? {}),
    ...(report.nextActions ?? []).map((action) => action.classification).filter(nonEmptyString),
  ])
  const generatedEvidenceKinds = new Set(generatedEvidenceKindsFor(report.generatedEvidence))
  const generatedMatrixLimitations = new Set(generatedMatrixLimitationsFor(report.generatedEvidence))
  const exitCriterionStatuses = new Set(Object.keys(report.checks?.artifacts?.aggregate?.exitCriterionStatuses ?? {}))
  const incompleteExitCriterionStatuses = new Set(Object.keys(report.checks?.artifacts?.aggregate?.incompleteExitCriterionStatuses ?? {}))
  return {
    ...runtimeSignalMetadataForValidationGateReport(report),
    ...(coverageAreas.size > 0 ? { coverageAreas: [...coverageAreas].sort().join(",") } : {}),
    ...(owners.size > 0 ? { owners: [...owners].sort().join(",") } : {}),
    ...(classifications.size > 0 ? { classifications: [...classifications].sort().join(",") } : {}),
    ...(exitCriterionStatuses.size > 0 ? { exitCriterionStatuses: [...exitCriterionStatuses].sort().join(",") } : {}),
    ...(incompleteExitCriterionStatuses.size > 0 ? { incompleteExitCriterionStatuses: [...incompleteExitCriterionStatuses].sort().join(",") } : {}),
    ...(generatedEvidenceKinds.size > 0 ? { generatedEvidenceKinds: [...generatedEvidenceKinds].sort().join(",") } : {}),
    ...(generatedMatrixLimitations.size > 0 ? { generatedMatrixLimitations: [...generatedMatrixLimitations].sort().join(",") } : {}),
  }
}

export function runtimeSignalMetadataForValidationGateAggregate(aggregate) {
  const signals = new Set([
    ...Object.keys(aggregate.coverage?.artifactRuntimeSignals ?? {}),
    ...Object.keys(aggregate.coverage?.failureRuntimeSignals ?? {}),
    ...Object.keys(aggregate.coverage?.matrixRuntimeSignals ?? {}),
    ...Object.keys(aggregate.matrixRuntimeSignalSources ?? {}),
  ])
  const signalOwners = new Set([
    ...drillRuntimeSignalOwnersFor([...signals]),
    ...Object.keys(aggregate.coverage?.artifactRuntimeSignalOwners ?? {}),
    ...Object.keys(aggregate.coverage?.failureRuntimeSignalOwners ?? {}),
    ...Object.keys(aggregate.coverage?.matrixRuntimeSignalOwners ?? {}),
  ])
  return signals.size > 0
    ? {
      runtimeSignals: [...signals].sort().join(","),
      ...(signalOwners.size > 0 ? { runtimeSignalOwners: [...signalOwners].sort().join(",") } : {}),
    }
    : {}
}

export function diagnosticMetadataForValidationGateAggregate(aggregate) {
  const artifactCoverageInputs = Array.isArray(aggregate.artifactCoverageInputs)
    ? aggregate.artifactCoverageInputs
    : []
  const artifactCoverageInputSources = artifactCoverageInputs
    .map((input) => input?.source)
    .filter(nonEmptyString)
  const artifactCoverageCoverageSources = Object.keys(aggregate.coverage?.artifactCoverageInputSources ?? {})
    .filter(nonEmptyString)
  const coverageAreas = new Set([
    ...Object.keys(aggregate.coverage?.artifactCoverageAreas ?? {}),
  ])
  const owners = new Set([
    ...Object.keys(aggregate.coverage?.artifactRuntimeSignalOwners ?? {}),
    ...Object.keys(aggregate.coverage?.failureRuntimeSignalOwners ?? {}),
    ...Object.keys(aggregate.coverage?.matrixRuntimeSignalOwners ?? {}),
    ...Object.keys(aggregate.coverage?.artifactOwners ?? {}),
    ...Object.keys(aggregate.coverage?.failureOwners ?? {}),
    ...Object.keys(aggregate.coverage?.matrixOwners ?? {}),
    ...(aggregate.nextActions ?? []).map((action) => action.owner).filter(nonEmptyString),
  ])
  const classifications = new Set([
    ...Object.keys(aggregate.coverage?.artifactClassifications ?? {}),
    ...Object.keys(aggregate.coverage?.failureClassifications ?? {}),
    ...Object.keys(aggregate.coverage?.matrixClassifications ?? {}),
    ...Object.keys(aggregate.coverage?.requiredFailureClassifications ?? {}),
    ...Object.keys(aggregate.coverage?.missingFailureClassifications ?? {}),
    ...Object.keys(aggregate.coverage?.requiredMatrixClassifications ?? {}),
    ...Object.keys(aggregate.coverage?.missingMatrixClassifications ?? {}),
    ...(aggregate.requiredFailureClassifications ?? []).filter(nonEmptyString),
    ...(aggregate.missingFailureClassifications ?? []).filter(nonEmptyString),
    ...(aggregate.requiredMatrixClassifications ?? []).filter(nonEmptyString),
    ...(aggregate.missingMatrixClassifications ?? []).filter(nonEmptyString),
    ...(aggregate.nextActions ?? []).map((action) => action.classification).filter(nonEmptyString),
  ])
  const generatedEvidenceKinds = new Set([
    ...Object.keys(aggregate.coverage?.artifactGeneratedEvidenceKinds ?? {}),
    ...Object.keys(aggregate.coverage?.generatedEvidenceKinds ?? {}),
  ])
  const generatedMatrixLimitations = new Set([
    ...Object.keys(aggregate.coverage?.artifactGeneratedMatrixLimitations ?? {}),
    ...Object.keys(aggregate.coverage?.generatedMatrixLimitations ?? {}),
  ])
  const exitCriterionStatuses = new Set(Object.keys(aggregate.coverage?.artifactExitCriterionStatuses ?? {}))
  const incompleteExitCriterionStatuses = new Set(Object.keys(aggregate.coverage?.artifactIncompleteExitCriterionStatuses ?? {}))
  const requiredGeneratedEvidenceKinds = new Set([
    ...Object.keys(aggregate.coverage?.requiredArtifactGeneratedEvidenceKinds ?? {}),
    ...Object.keys(aggregate.coverage?.requiredGeneratedEvidenceKinds ?? {}),
    ...(aggregate.requiredArtifactGeneratedEvidenceKinds ?? []).filter(nonEmptyString),
    ...(aggregate.requiredGeneratedEvidenceKinds ?? []).filter(nonEmptyString),
  ])
  const missingGeneratedEvidenceKinds = new Set([
    ...Object.keys(aggregate.coverage?.missingArtifactGeneratedEvidenceKinds ?? {}),
    ...Object.keys(aggregate.coverage?.missingGeneratedEvidenceKinds ?? {}),
    ...(aggregate.missingArtifactGeneratedEvidenceKinds ?? []).filter(nonEmptyString),
    ...(aggregate.missingGeneratedEvidenceKinds ?? []).filter(nonEmptyString),
  ])
  const requiredGeneratedMatrixLimitations = new Set([
    ...Object.keys(aggregate.coverage?.requiredArtifactGeneratedMatrixLimitations ?? {}),
    ...Object.keys(aggregate.coverage?.requiredGeneratedMatrixLimitations ?? {}),
    ...(aggregate.requiredArtifactGeneratedMatrixLimitations ?? []).filter(nonEmptyString),
    ...(aggregate.requiredGeneratedMatrixLimitations ?? []).filter(nonEmptyString),
  ])
  const missingGeneratedMatrixLimitations = new Set([
    ...Object.keys(aggregate.coverage?.missingArtifactGeneratedMatrixLimitations ?? {}),
    ...Object.keys(aggregate.coverage?.missingGeneratedMatrixLimitations ?? {}),
    ...(aggregate.missingArtifactGeneratedMatrixLimitations ?? []).filter(nonEmptyString),
    ...(aggregate.missingGeneratedMatrixLimitations ?? []).filter(nonEmptyString),
  ])
  return {
    ...runtimeSignalMetadataForValidationGateAggregate(aggregate),
    ...(coverageAreas.size > 0 ? { coverageAreas: [...coverageAreas].sort().join(",") } : {}),
    ...(owners.size > 0 ? { owners: [...owners].sort().join(",") } : {}),
    ...(classifications.size > 0 ? { classifications: [...classifications].sort().join(",") } : {}),
    ...(exitCriterionStatuses.size > 0 ? { exitCriterionStatuses: [...exitCriterionStatuses].sort().join(",") } : {}),
    ...(incompleteExitCriterionStatuses.size > 0 ? { incompleteExitCriterionStatuses: [...incompleteExitCriterionStatuses].sort().join(",") } : {}),
    ...(generatedEvidenceKinds.size > 0 ? { generatedEvidenceKinds: [...generatedEvidenceKinds].sort().join(",") } : {}),
    ...(generatedMatrixLimitations.size > 0 ? { generatedMatrixLimitations: [...generatedMatrixLimitations].sort().join(",") } : {}),
    ...(requiredGeneratedEvidenceKinds.size > 0 ? { requiredGeneratedEvidenceKinds: [...requiredGeneratedEvidenceKinds].sort().join(",") } : {}),
    ...(missingGeneratedEvidenceKinds.size > 0 ? { missingGeneratedEvidenceKinds: [...missingGeneratedEvidenceKinds].sort().join(",") } : {}),
    ...(requiredGeneratedMatrixLimitations.size > 0 ? { requiredGeneratedMatrixLimitations: [...requiredGeneratedMatrixLimitations].sort().join(",") } : {}),
    ...(missingGeneratedMatrixLimitations.size > 0 ? { missingGeneratedMatrixLimitations: [...missingGeneratedMatrixLimitations].sort().join(",") } : {}),
    ...(artifactCoverageInputs.length > 0 ? { artifactCoverageInputCount: String(artifactCoverageInputs.length) } : {}),
    ...(artifactCoverageInputSources.length > 0 || artifactCoverageCoverageSources.length > 0
      ? { artifactCoverageInputSources: sortedUnique([...artifactCoverageInputSources, ...artifactCoverageCoverageSources]).join(",") }
      : {}),
  }
}

function nonEmptyString(value) {
  return typeof value === "string" && value.length > 0
}

function generatedEvidenceKindsFor(generatedEvidence) {
  if (!generatedEvidence || typeof generatedEvidence !== "object") return []
  const kinds = new Set((generatedEvidence.kinds ?? []).filter(nonEmptyString))
  if (generatedEvidence.validationSuites?.enabled) kinds.add("validation-suite-run")
  if (generatedEvidence.matrixReports?.enabled) kinds.add("matrix-report")
  return [...kinds].sort()
}

function generatedMatrixLimitationsFor(generatedEvidence) {
  if (!generatedEvidence?.matrixReports || typeof generatedEvidence.matrixReports !== "object") return []
  return (generatedEvidence.matrixReports.limitations ?? [])
    .map((limitation) => limitation?.kind)
    .filter(nonEmptyString)
    .sort()
}

function platformCoverageAreas(report) {
  return (report.checks?.platformBundle?.validationSuite?.coverageAreas ?? [])
    .map((area) => area?.id)
    .filter(nonEmptyString)
}

function platformRuntimeSignals(report) {
  return (report.checks?.platformBundle?.runtimeSignals ?? [])
    .map((signal) => signal?.id)
    .filter(nonEmptyString)
}

function platformRuntimeSignalOwners(report) {
  return (report.checks?.platformBundle?.runtimeSignals ?? [])
    .map((signal) => signal?.owner)
    .filter(nonEmptyString)
}

function platformFailureClassifications(report) {
  return sortedUnique([
    ...(report.checks?.platformBundle?.failureTaxonomy?.drill ?? []),
    ...(report.checks?.platformBundle?.failureTaxonomy?.scenario ?? []),
  ].filter(nonEmptyString))
}

function sortedUnique(values) {
  return [...new Set(values)].sort()
}
