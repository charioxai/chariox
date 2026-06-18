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
  const requiredSignals = new Set([
    ...(report.checks?.platformBundle?.requiredRuntimeSignals ?? []),
    ...(report.checks?.artifacts?.requiredArtifactRuntimeSignals ?? []),
    ...Object.keys(report.checks?.artifacts?.aggregate?.requiredRuntimeSignals ?? {}),
    ...(report.checks?.matrices?.requiredMatrixRuntimeSignals ?? []),
  ].filter(nonEmptyString))
  const missingSignals = new Set([
    ...(report.checks?.platformBundle?.missingRuntimeSignals ?? []),
    ...(report.checks?.artifacts?.missingArtifactRuntimeSignals ?? []),
    ...Object.keys(report.checks?.artifacts?.aggregate?.missingRuntimeSignals ?? {}),
    ...(report.checks?.matrices?.missingMatrixRuntimeSignals ?? []),
  ].filter(nonEmptyString))
  const signalOwners = new Set([
    ...drillRuntimeSignalOwnersFor([...signals]),
    ...platformRuntimeSignalOwners(report),
    ...Object.keys(report.checks?.artifacts?.aggregate?.runtimeSignalOwners ?? {}),
  ])
  const requiredSignalOwners = new Set([
    ...drillRuntimeSignalOwnersFor([...requiredSignals]),
    ...(report.checks?.platformBundle?.requiredRuntimeSignalOwners ?? []),
  ])
  const missingSignalOwners = new Set([
    ...drillRuntimeSignalOwnersFor([...missingSignals]),
    ...(report.checks?.platformBundle?.missingRuntimeSignalOwners ?? []),
  ])
  return signals.size > 0 || requiredSignals.size > 0 || requiredSignalOwners.size > 0 || missingSignals.size > 0 || missingSignalOwners.size > 0
    ? {
      ...(signals.size > 0 ? { runtimeSignals: [...signals].sort().join(",") } : {}),
      ...(requiredSignals.size > 0 ? { requiredRuntimeSignals: [...requiredSignals].sort().join(",") } : {}),
      ...(requiredSignalOwners.size > 0 ? { requiredRuntimeSignalOwners: [...requiredSignalOwners].sort().join(",") } : {}),
      ...(missingSignals.size > 0 ? { missingRuntimeSignals: [...missingSignals].sort().join(",") } : {}),
      ...(missingSignalOwners.size > 0 ? { missingRuntimeSignalOwners: [...missingSignalOwners].sort().join(",") } : {}),
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
  const requiredFailureClassifications = new Set([
    ...(report.checks?.platformBundle?.requiredFailureClassifications ?? []),
    ...(report.checks?.artifacts?.requiredArtifactFailureClassifications ?? []),
    ...Object.keys(report.checks?.artifacts?.aggregate?.requiredFailureClassifications ?? {}),
    ...(report.checks?.matrices?.requiredMatrixClassifications ?? []),
  ].filter(nonEmptyString))
  const missingFailureClassifications = new Set([
    ...(report.checks?.platformBundle?.missingFailureClassifications ?? []),
    ...(report.checks?.artifacts?.missingArtifactFailureClassifications ?? []),
    ...(report.checks?.matrices?.missingMatrixClassifications ?? []),
  ].filter(nonEmptyString))
  const generatedEvidenceKinds = new Set(generatedEvidenceKindsFor(report.generatedEvidence))
  const generatedMatrixArtifactIndexes = new Set(generatedMatrixArtifactIndexesFor(report.generatedEvidence))
  const generatedMatrixLimitations = new Set(generatedMatrixLimitationsFor(report.generatedEvidence))
  const generatedValidationSuiteArtifactIndexes = new Set(generatedValidationSuiteArtifactIndexesFor(report.generatedEvidence))
  const generatedValidationSuiteFailureRoots = new Set(generatedValidationSuiteFailureRootsFor(report.generatedEvidence))
  const exitCriterionStatuses = new Set(Object.keys(report.checks?.artifacts?.aggregate?.exitCriterionStatuses ?? {}))
  const incompleteExitCriterionStatuses = new Set(Object.keys(report.checks?.artifacts?.aggregate?.incompleteExitCriterionStatuses ?? {}))
  const providerAccountAliases = new Set(Object.keys(report.checks?.artifacts?.aggregate?.providerAccountAliases ?? {}))
  return {
    ...runtimeSignalMetadataForValidationGateReport(report),
    ...(coverageAreas.size > 0 ? { coverageAreas: [...coverageAreas].sort().join(",") } : {}),
    ...(owners.size > 0 ? { owners: [...owners].sort().join(",") } : {}),
    ...(classifications.size > 0 ? { classifications: [...classifications].sort().join(",") } : {}),
    ...(requiredFailureClassifications.size > 0 ? { requiredFailureClassifications: [...requiredFailureClassifications].sort().join(",") } : {}),
    ...(missingFailureClassifications.size > 0 ? { missingFailureClassifications: [...missingFailureClassifications].sort().join(",") } : {}),
    ...(exitCriterionStatuses.size > 0 ? { exitCriterionStatuses: [...exitCriterionStatuses].sort().join(",") } : {}),
    ...(incompleteExitCriterionStatuses.size > 0 ? { incompleteExitCriterionStatuses: [...incompleteExitCriterionStatuses].sort().join(",") } : {}),
    ...(providerAccountAliases.size > 0 ? { providerAccountAliases: [...providerAccountAliases].sort().join(",") } : {}),
    ...(generatedEvidenceKinds.size > 0 ? { generatedEvidenceKinds: [...generatedEvidenceKinds].sort().join(",") } : {}),
    ...(generatedMatrixArtifactIndexes.size > 0 ? { generatedMatrixArtifactIndexes: [...generatedMatrixArtifactIndexes].sort().join(",") } : {}),
    ...(generatedMatrixLimitations.size > 0 ? { generatedMatrixLimitations: [...generatedMatrixLimitations].sort().join(",") } : {}),
    ...(generatedValidationSuiteArtifactIndexes.size > 0 ? { generatedValidationSuiteArtifactIndexes: [...generatedValidationSuiteArtifactIndexes].sort().join(",") } : {}),
    ...(generatedValidationSuiteFailureRoots.size > 0 ? { generatedValidationSuiteFailureRoots: [...generatedValidationSuiteFailureRoots].sort().join(",") } : {}),
  }
}

export function runtimeSignalMetadataForValidationGateAggregate(aggregate) {
  const signals = new Set([
    ...Object.keys(aggregate.coverage?.artifactRuntimeSignals ?? {}),
    ...Object.keys(aggregate.coverage?.failureRuntimeSignals ?? {}),
    ...Object.keys(aggregate.coverage?.matrixRuntimeSignals ?? {}),
    ...Object.keys(aggregate.matrixRuntimeSignalSources ?? {}),
  ])
  const requiredSignals = new Set([
    ...Object.keys(aggregate.coverage?.requiredRuntimeSignals ?? {}),
    ...Object.keys(aggregate.coverage?.requiredArtifactRuntimeSignals ?? {}),
    ...Object.keys(aggregate.coverage?.requiredMatrixRuntimeSignals ?? {}),
    ...(aggregate.requiredRuntimeSignals ?? []),
    ...(aggregate.requiredArtifactRuntimeSignals ?? []),
    ...(aggregate.requiredMatrixRuntimeSignals ?? []),
  ].filter(nonEmptyString))
  const missingSignals = new Set([
    ...Object.keys(aggregate.coverage?.missingRuntimeSignals ?? {}),
    ...Object.keys(aggregate.coverage?.missingArtifactRuntimeSignals ?? {}),
    ...Object.keys(aggregate.coverage?.missingMatrixRuntimeSignals ?? {}),
    ...(aggregate.missingRuntimeSignals ?? []),
    ...(aggregate.missingArtifactRuntimeSignals ?? []),
    ...(aggregate.missingMatrixRuntimeSignals ?? []),
  ].filter(nonEmptyString))
  const signalOwners = new Set([
    ...drillRuntimeSignalOwnersFor([...signals]),
    ...Object.keys(aggregate.coverage?.artifactRuntimeSignalOwners ?? {}),
    ...Object.keys(aggregate.coverage?.failureRuntimeSignalOwners ?? {}),
    ...Object.keys(aggregate.coverage?.matrixRuntimeSignalOwners ?? {}),
  ])
  const requiredSignalOwners = new Set([
    ...drillRuntimeSignalOwnersFor([...requiredSignals]),
    ...Object.keys(aggregate.coverage?.requiredRuntimeSignalOwners ?? {}),
    ...(aggregate.requiredRuntimeSignalOwners ?? []),
  ])
  const missingSignalOwners = new Set([
    ...drillRuntimeSignalOwnersFor([...missingSignals]),
    ...Object.keys(aggregate.coverage?.missingRuntimeSignalOwners ?? {}),
    ...(aggregate.missingRuntimeSignalOwners ?? []),
  ])
  return signals.size > 0 || requiredSignals.size > 0 || requiredSignalOwners.size > 0 || missingSignals.size > 0 || missingSignalOwners.size > 0
    ? {
      ...(signals.size > 0 ? { runtimeSignals: [...signals].sort().join(",") } : {}),
      ...(requiredSignals.size > 0 ? { requiredRuntimeSignals: [...requiredSignals].sort().join(",") } : {}),
      ...(requiredSignalOwners.size > 0 ? { requiredRuntimeSignalOwners: [...requiredSignalOwners].sort().join(",") } : {}),
      ...(missingSignals.size > 0 ? { missingRuntimeSignals: [...missingSignals].sort().join(",") } : {}),
      ...(missingSignalOwners.size > 0 ? { missingRuntimeSignalOwners: [...missingSignalOwners].sort().join(",") } : {}),
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
  const requiredFailureClassifications = new Set([
    ...Object.keys(aggregate.coverage?.artifactFailureClassifications ?? {}),
    ...Object.keys(aggregate.coverage?.requiredArtifactFailureClassifications ?? {}),
    ...Object.keys(aggregate.coverage?.requiredFailureClassifications ?? {}),
    ...Object.keys(aggregate.coverage?.requiredMatrixClassifications ?? {}),
    ...(aggregate.requiredArtifactFailureClassifications ?? []).filter(nonEmptyString),
    ...(aggregate.requiredFailureClassifications ?? []).filter(nonEmptyString),
    ...(aggregate.requiredMatrixClassifications ?? []).filter(nonEmptyString),
  ])
  const missingFailureClassifications = new Set([
    ...Object.keys(aggregate.coverage?.missingArtifactFailureClassifications ?? {}),
    ...Object.keys(aggregate.coverage?.missingFailureClassifications ?? {}),
    ...Object.keys(aggregate.coverage?.missingMatrixClassifications ?? {}),
    ...(aggregate.missingArtifactFailureClassifications ?? []).filter(nonEmptyString),
    ...(aggregate.missingFailureClassifications ?? []).filter(nonEmptyString),
    ...(aggregate.missingMatrixClassifications ?? []).filter(nonEmptyString),
  ])
  const generatedEvidenceKinds = new Set([
    ...Object.keys(aggregate.coverage?.artifactGeneratedEvidenceKinds ?? {}),
    ...Object.keys(aggregate.coverage?.generatedEvidenceKinds ?? {}),
  ])
  const generatedMatrixArtifactIndexes = new Set([
    ...Object.keys(aggregate.coverage?.artifactGeneratedMatrixArtifactIndexes ?? {}),
    ...(aggregate.reports ?? [])
      .flatMap((report) => generatedMatrixArtifactIndexesFor(report?.generatedEvidence)),
  ])
  const generatedMatrixLimitations = new Set([
    ...Object.keys(aggregate.coverage?.artifactGeneratedMatrixLimitations ?? {}),
    ...Object.keys(aggregate.coverage?.generatedMatrixLimitations ?? {}),
  ])
  const generatedMatrixNames = new Set([
    ...Object.keys(aggregate.coverage?.artifactGeneratedMatrixNames ?? {}),
  ])
  const generatedMatrixRepos = new Set([
    ...Object.keys(aggregate.coverage?.artifactGeneratedMatrixRepos ?? {}),
  ])
  const generatedValidationSuiteArtifactIndexes = new Set(
    [
      ...Object.keys(aggregate.coverage?.generatedValidationSuiteArtifactIndexes ?? {}),
      ...(aggregate.reports ?? [])
      .flatMap((report) => generatedValidationSuiteArtifactIndexesFor(report?.generatedEvidence)),
    ].filter(nonEmptyString),
  )
  const generatedValidationSuiteFailureRoots = new Set(
    [
      ...Object.keys(aggregate.coverage?.artifactGeneratedValidationSuiteFailureRoots ?? {}),
      ...Object.keys(aggregate.coverage?.generatedValidationSuiteFailureRoots ?? {}),
      ...(aggregate.reports ?? [])
      .flatMap((report) => generatedValidationSuiteFailureRootsFor(report?.generatedEvidence)),
    ].filter(nonEmptyString),
  )
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
  const requiredGeneratedMatrixNames = new Set([
    ...Object.keys(aggregate.coverage?.requiredArtifactGeneratedMatrixNames ?? {}),
    ...(aggregate.requiredArtifactGeneratedMatrixNames ?? []).filter(nonEmptyString),
  ])
  const missingGeneratedMatrixNames = new Set([
    ...Object.keys(aggregate.coverage?.missingArtifactGeneratedMatrixNames ?? {}),
    ...(aggregate.missingArtifactGeneratedMatrixNames ?? []).filter(nonEmptyString),
  ])
  const requiredGeneratedMatrixRepos = new Set([
    ...Object.keys(aggregate.coverage?.requiredArtifactGeneratedMatrixRepos ?? {}),
    ...(aggregate.requiredArtifactGeneratedMatrixRepos ?? []).filter(nonEmptyString),
  ])
  const missingGeneratedMatrixRepos = new Set([
    ...Object.keys(aggregate.coverage?.missingArtifactGeneratedMatrixRepos ?? {}),
    ...(aggregate.missingArtifactGeneratedMatrixRepos ?? []).filter(nonEmptyString),
  ])
  const requiredGeneratedMatrixArtifactIndexes = new Set([
    ...Object.keys(aggregate.coverage?.requiredArtifactGeneratedMatrixArtifactIndexes ?? {}),
    ...Object.keys(aggregate.coverage?.requiredGeneratedMatrixArtifactIndexes ?? {}),
    ...(aggregate.requiredArtifactGeneratedMatrixArtifactIndexes ?? []).filter(nonEmptyString),
    ...(aggregate.requiredGeneratedMatrixArtifactIndexes ?? []).filter(nonEmptyString),
  ])
  const missingGeneratedMatrixArtifactIndexes = new Set([
    ...Object.keys(aggregate.coverage?.missingArtifactGeneratedMatrixArtifactIndexes ?? {}),
    ...Object.keys(aggregate.coverage?.missingGeneratedMatrixArtifactIndexes ?? {}),
    ...(aggregate.missingArtifactGeneratedMatrixArtifactIndexes ?? []).filter(nonEmptyString),
    ...(aggregate.missingGeneratedMatrixArtifactIndexes ?? []).filter(nonEmptyString),
  ])
  const requiredGeneratedValidationSuiteArtifactIndexes = new Set([
    ...Object.keys(aggregate.coverage?.requiredArtifactGeneratedValidationSuiteArtifactIndexes ?? {}),
    ...Object.keys(aggregate.coverage?.requiredGeneratedValidationSuiteArtifactIndexes ?? {}),
    ...(aggregate.requiredArtifactGeneratedValidationSuiteArtifactIndexes ?? []).filter(nonEmptyString),
    ...(aggregate.requiredGeneratedValidationSuiteArtifactIndexes ?? []).filter(nonEmptyString),
  ])
  const missingGeneratedValidationSuiteArtifactIndexes = new Set([
    ...Object.keys(aggregate.coverage?.missingArtifactGeneratedValidationSuiteArtifactIndexes ?? {}),
    ...Object.keys(aggregate.coverage?.missingGeneratedValidationSuiteArtifactIndexes ?? {}),
    ...(aggregate.missingArtifactGeneratedValidationSuiteArtifactIndexes ?? []).filter(nonEmptyString),
    ...(aggregate.missingGeneratedValidationSuiteArtifactIndexes ?? []).filter(nonEmptyString),
  ])
  const requiredGeneratedValidationSuiteFailureRoots = new Set([
    ...Object.keys(aggregate.coverage?.requiredArtifactGeneratedValidationSuiteFailureRoots ?? {}),
    ...Object.keys(aggregate.coverage?.requiredGeneratedValidationSuiteFailureRoots ?? {}),
    ...(aggregate.requiredArtifactGeneratedValidationSuiteFailureRoots ?? []).filter(nonEmptyString),
    ...(aggregate.requiredGeneratedValidationSuiteFailureRoots ?? []).filter(nonEmptyString),
  ])
  const missingGeneratedValidationSuiteFailureRoots = new Set([
    ...Object.keys(aggregate.coverage?.missingArtifactGeneratedValidationSuiteFailureRoots ?? {}),
    ...Object.keys(aggregate.coverage?.missingGeneratedValidationSuiteFailureRoots ?? {}),
    ...(aggregate.missingArtifactGeneratedValidationSuiteFailureRoots ?? []).filter(nonEmptyString),
    ...(aggregate.missingGeneratedValidationSuiteFailureRoots ?? []).filter(nonEmptyString),
  ])
  const providerAccountAliases = new Set([
    ...Object.keys(aggregate.coverage?.artifactProviderAccountAliases ?? {}),
  ])
  const validationPresets = new Set([
    ...Object.keys(aggregate.coverage?.artifactValidationPresets ?? {}),
  ])
  const requiredProviderAccountAliases = new Set([
    ...Object.keys(aggregate.coverage?.requiredArtifactProviderAccountAliases ?? {}),
    ...(aggregate.requiredArtifactProviderAccountAliases ?? []).filter(nonEmptyString),
  ])
  const missingProviderAccountAliases = new Set([
    ...Object.keys(aggregate.coverage?.missingArtifactProviderAccountAliases ?? {}),
    ...(aggregate.missingArtifactProviderAccountAliases ?? []).filter(nonEmptyString),
  ])
  const requiredValidationPresets = new Set([
    ...Object.keys(aggregate.coverage?.requiredArtifactValidationPresets ?? {}),
    ...(aggregate.requiredArtifactValidationPresets ?? []).filter(nonEmptyString),
  ])
  const missingValidationPresets = new Set([
    ...Object.keys(aggregate.coverage?.missingArtifactValidationPresets ?? {}),
    ...(aggregate.missingArtifactValidationPresets ?? []).filter(nonEmptyString),
  ])
  return {
    ...runtimeSignalMetadataForValidationGateAggregate(aggregate),
    ...(coverageAreas.size > 0 ? { coverageAreas: [...coverageAreas].sort().join(",") } : {}),
    ...(owners.size > 0 ? { owners: [...owners].sort().join(",") } : {}),
    ...(classifications.size > 0 ? { classifications: [...classifications].sort().join(",") } : {}),
    ...(requiredFailureClassifications.size > 0 ? { requiredFailureClassifications: [...requiredFailureClassifications].sort().join(",") } : {}),
    ...(missingFailureClassifications.size > 0 ? { missingFailureClassifications: [...missingFailureClassifications].sort().join(",") } : {}),
    ...(exitCriterionStatuses.size > 0 ? { exitCriterionStatuses: [...exitCriterionStatuses].sort().join(",") } : {}),
    ...(incompleteExitCriterionStatuses.size > 0 ? { incompleteExitCriterionStatuses: [...incompleteExitCriterionStatuses].sort().join(",") } : {}),
    ...(providerAccountAliases.size > 0 ? { providerAccountAliases: [...providerAccountAliases].sort().join(",") } : {}),
    ...(requiredProviderAccountAliases.size > 0 ? { requiredProviderAccountAliases: [...requiredProviderAccountAliases].sort().join(",") } : {}),
    ...(missingProviderAccountAliases.size > 0 ? { missingProviderAccountAliases: [...missingProviderAccountAliases].sort().join(",") } : {}),
    ...(validationPresets.size > 0 ? { validationPresets: [...validationPresets].sort().join(",") } : {}),
    ...(requiredValidationPresets.size > 0 ? { requiredValidationPresets: [...requiredValidationPresets].sort().join(",") } : {}),
    ...(missingValidationPresets.size > 0 ? { missingValidationPresets: [...missingValidationPresets].sort().join(",") } : {}),
    ...(generatedEvidenceKinds.size > 0 ? { generatedEvidenceKinds: [...generatedEvidenceKinds].sort().join(",") } : {}),
    ...(generatedMatrixArtifactIndexes.size > 0 ? { generatedMatrixArtifactIndexes: [...generatedMatrixArtifactIndexes].sort().join(",") } : {}),
    ...(generatedMatrixLimitations.size > 0 ? { generatedMatrixLimitations: [...generatedMatrixLimitations].sort().join(",") } : {}),
    ...(generatedMatrixNames.size > 0 ? { generatedMatrixNames: [...generatedMatrixNames].sort().join(",") } : {}),
    ...(generatedMatrixRepos.size > 0 ? { generatedMatrixRepos: [...generatedMatrixRepos].sort().join(",") } : {}),
    ...(generatedValidationSuiteArtifactIndexes.size > 0 ? { generatedValidationSuiteArtifactIndexes: [...generatedValidationSuiteArtifactIndexes].sort().join(",") } : {}),
    ...(generatedValidationSuiteFailureRoots.size > 0 ? { generatedValidationSuiteFailureRoots: [...generatedValidationSuiteFailureRoots].sort().join(",") } : {}),
    ...(requiredGeneratedEvidenceKinds.size > 0 ? { requiredGeneratedEvidenceKinds: [...requiredGeneratedEvidenceKinds].sort().join(",") } : {}),
    ...(missingGeneratedEvidenceKinds.size > 0 ? { missingGeneratedEvidenceKinds: [...missingGeneratedEvidenceKinds].sort().join(",") } : {}),
    ...(requiredGeneratedMatrixArtifactIndexes.size > 0 ? { requiredGeneratedMatrixArtifactIndexes: [...requiredGeneratedMatrixArtifactIndexes].sort().join(",") } : {}),
    ...(missingGeneratedMatrixArtifactIndexes.size > 0 ? { missingGeneratedMatrixArtifactIndexes: [...missingGeneratedMatrixArtifactIndexes].sort().join(",") } : {}),
    ...(requiredGeneratedMatrixLimitations.size > 0 ? { requiredGeneratedMatrixLimitations: [...requiredGeneratedMatrixLimitations].sort().join(",") } : {}),
    ...(missingGeneratedMatrixLimitations.size > 0 ? { missingGeneratedMatrixLimitations: [...missingGeneratedMatrixLimitations].sort().join(",") } : {}),
    ...(requiredGeneratedMatrixNames.size > 0 ? { requiredGeneratedMatrixNames: [...requiredGeneratedMatrixNames].sort().join(",") } : {}),
    ...(missingGeneratedMatrixNames.size > 0 ? { missingGeneratedMatrixNames: [...missingGeneratedMatrixNames].sort().join(",") } : {}),
    ...(requiredGeneratedMatrixRepos.size > 0 ? { requiredGeneratedMatrixRepos: [...requiredGeneratedMatrixRepos].sort().join(",") } : {}),
    ...(missingGeneratedMatrixRepos.size > 0 ? { missingGeneratedMatrixRepos: [...missingGeneratedMatrixRepos].sort().join(",") } : {}),
    ...(requiredGeneratedValidationSuiteArtifactIndexes.size > 0 ? { requiredGeneratedValidationSuiteArtifactIndexes: [...requiredGeneratedValidationSuiteArtifactIndexes].sort().join(",") } : {}),
    ...(missingGeneratedValidationSuiteArtifactIndexes.size > 0 ? { missingGeneratedValidationSuiteArtifactIndexes: [...missingGeneratedValidationSuiteArtifactIndexes].sort().join(",") } : {}),
    ...(requiredGeneratedValidationSuiteFailureRoots.size > 0 ? { requiredGeneratedValidationSuiteFailureRoots: [...requiredGeneratedValidationSuiteFailureRoots].sort().join(",") } : {}),
    ...(missingGeneratedValidationSuiteFailureRoots.size > 0 ? { missingGeneratedValidationSuiteFailureRoots: [...missingGeneratedValidationSuiteFailureRoots].sort().join(",") } : {}),
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

function generatedMatrixArtifactIndexesFor(generatedEvidence) {
  const matrixReports = generatedEvidence?.matrixReports
  if (!matrixReports || typeof matrixReports !== "object") return []
  return sortedUnique((matrixReports.artifactIndexes ?? []).filter(nonEmptyString))
}

function generatedValidationSuiteArtifactIndexesFor(generatedEvidence) {
  const validationSuites = generatedEvidence?.validationSuites
  if (!validationSuites || typeof validationSuites !== "object") return []
  const explicitArtifactIndexes = Array.isArray(validationSuites.artifactIndexes)
    ? validationSuites.artifactIndexes
    : []
  const commandArtifactIndexes = Array.isArray(validationSuites.commands)
    ? validationSuites.commands.map((command) => command?.artifactIndexPath)
    : []
  return sortedUnique([...explicitArtifactIndexes, ...commandArtifactIndexes].filter(nonEmptyString))
}

function generatedMatrixLimitationsFor(generatedEvidence) {
  if (!generatedEvidence?.matrixReports || typeof generatedEvidence.matrixReports !== "object") return []
  return (generatedEvidence.matrixReports.limitations ?? [])
    .map((limitation) => limitation?.kind)
    .filter(nonEmptyString)
    .sort()
}

function generatedValidationSuiteFailureRootsFor(generatedEvidence) {
  const validationSuites = generatedEvidence?.validationSuites
  if (!validationSuites || typeof validationSuites !== "object") return []
  const explicitFailureRoots = Array.isArray(validationSuites.failureRoots)
    ? validationSuites.failureRoots
    : []
  const commandFailureRoots = Array.isArray(validationSuites.commands)
    ? validationSuites.commands.map((command) => command?.failureRoot)
    : []
  return sortedUnique([...explicitFailureRoots, ...commandFailureRoots].filter(nonEmptyString))
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
