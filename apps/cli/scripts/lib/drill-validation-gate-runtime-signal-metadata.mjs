export function runtimeSignalMetadataForValidationGateReport(report) {
  const signals = new Set([
    ...Object.keys(report.checks?.artifacts?.aggregate?.runtimeSignals ?? {}),
    ...Object.keys(report.checks?.failures?.aggregate?.runtimeSignals ?? {}),
    ...Object.keys(report.checks?.matrices?.aggregate?.runtimeSignals ?? {}),
    ...Object.keys(report.checks?.matrices?.aggregate?.runtimeSignalScenarios ?? {}),
  ])
  return signals.size > 0
    ? { runtimeSignals: [...signals].sort().join(",") }
    : {}
}

export function runtimeSignalMetadataForValidationGateAggregate(aggregate) {
  const signals = new Set([
    ...Object.keys(aggregate.coverage?.artifactRuntimeSignals ?? {}),
    ...Object.keys(aggregate.coverage?.failureRuntimeSignals ?? {}),
    ...Object.keys(aggregate.matrixRuntimeSignalSources ?? {}),
  ])
  return signals.size > 0
    ? { runtimeSignals: [...signals].sort().join(",") }
    : {}
}
