import { drillRuntimeSignalOwner } from "./drill-runtime-signals.mjs"

export function runtimeSignalMetadataForValidationGateReport(report) {
  const signals = new Set([
    ...Object.keys(report.checks?.artifacts?.aggregate?.runtimeSignals ?? {}),
    ...Object.keys(report.checks?.failures?.aggregate?.runtimeSignals ?? {}),
    ...Object.keys(report.checks?.matrices?.aggregate?.runtimeSignals ?? {}),
    ...Object.keys(report.checks?.matrices?.aggregate?.runtimeSignalScenarios ?? {}),
  ])
  const signalOwners = new Set([
    ...[...signals].map((signal) => drillRuntimeSignalOwner(signal)),
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
  const owners = new Set([
    ...Object.keys(report.checks?.artifacts?.aggregate?.owners ?? {}),
    ...Object.keys(report.checks?.failures?.aggregate?.owners ?? {}),
    ...Object.keys(report.checks?.matrices?.aggregate?.owners ?? {}),
    ...(report.nextActions ?? []).map((action) => action.owner).filter(nonEmptyString),
  ])
  const classifications = new Set([
    ...Object.keys(report.checks?.artifacts?.aggregate?.classifications ?? {}),
    ...Object.keys(report.checks?.failures?.aggregate?.classifications ?? {}),
    ...Object.keys(report.checks?.matrices?.aggregate?.classifications ?? {}),
    ...(report.nextActions ?? []).map((action) => action.classification).filter(nonEmptyString),
  ])
  return {
    ...runtimeSignalMetadataForValidationGateReport(report),
    ...(owners.size > 0 ? { owners: [...owners].sort().join(",") } : {}),
    ...(classifications.size > 0 ? { classifications: [...classifications].sort().join(",") } : {}),
  }
}

export function runtimeSignalMetadataForValidationGateAggregate(aggregate) {
  const signals = new Set([
    ...Object.keys(aggregate.coverage?.artifactRuntimeSignals ?? {}),
    ...Object.keys(aggregate.coverage?.failureRuntimeSignals ?? {}),
    ...Object.keys(aggregate.matrixRuntimeSignalSources ?? {}),
  ])
  const signalOwners = new Set([
    ...[...signals].map((signal) => drillRuntimeSignalOwner(signal)),
    ...Object.keys(aggregate.coverage?.artifactRuntimeSignalOwners ?? {}),
  ])
  return signals.size > 0
    ? {
      runtimeSignals: [...signals].sort().join(","),
      ...(signalOwners.size > 0 ? { runtimeSignalOwners: [...signalOwners].sort().join(",") } : {}),
    }
    : {}
}

export function diagnosticMetadataForValidationGateAggregate(aggregate) {
  const owners = new Set([
    ...Object.keys(aggregate.coverage?.artifactRuntimeSignalOwners ?? {}),
    ...Object.keys(aggregate.coverage?.artifactOwners ?? {}),
    ...(aggregate.nextActions ?? []).map((action) => action.owner).filter(nonEmptyString),
  ])
  const classifications = new Set([
    ...Object.keys(aggregate.coverage?.artifactClassifications ?? {}),
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
  return {
    ...runtimeSignalMetadataForValidationGateAggregate(aggregate),
    ...(owners.size > 0 ? { owners: [...owners].sort().join(",") } : {}),
    ...(classifications.size > 0 ? { classifications: [...classifications].sort().join(",") } : {}),
  }
}

function nonEmptyString(value) {
  return typeof value === "string" && value.length > 0
}
