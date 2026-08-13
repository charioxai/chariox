import {
  countDrillAggregateEntriesBy,
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  formatDrillAggregateNextActionSourceDetails,
  validateDrillAggregateNextAction,
} from "./drill-aggregate-actions.mjs"
import {
  drillFailureNextActionForClassification,
  drillFailureOwnerForClassification,
  validateDrillFailureClassification,
} from "./drill-failure-taxonomy.mjs"
import { drillRuntimeSignalOwnerCounts } from "./drill-runtime-signals.mjs"
import {
  validateDrillMatrixReportStatus,
  validateDrillMatrixScenarioStatus,
} from "./drill-matrix-statuses.mjs"
import {
  appendRuntimeSignalEvidence,
  artifactHintLooksSecret,
  assertRuntimeSignalEvidenceCounts,
  assertRuntimeSignalEvidenceScenarioIds,
  formatCountObject,
  formatRuntimeSignalEvidence,
  formatRuntimeSignalScenarioRef,
  incompleteExitCriteriaCount,
  isValidArtifactHint,
  nonEmptyString,
  sumMatrixAggregateReportEntries,
  validateCountObject,
  validateDeploymentPresetList,
  validateDeploymentPresetCountObject,
  validateExitCriteriaCountObject,
  validateExitCriterionEvidence,
  validateFailureClassificationCountObject,
  validateMatrixAggregateReportCounts,
  validateOptionalCriterionDiagnostics,
  validateProviderList,
  validateProviderCountObject,
  validateRuntimeSignalCountObject,
  validateRuntimeSignalEvidenceObject,
} from "./drill-matrix-report-shared.mjs"

export function formatDrillMatrixAggregateSummary(aggregate) {
  validateDrillMatrixAggregate(aggregate)
  const lines = [
    "matrix aggregate:",
    `status=${aggregate.status} reports=${aggregate.totals.reports} scenarios=${aggregate.totals.scenarios} passed=${aggregate.totals.passed} failed=${aggregate.totals.failed} skipped=${aggregate.totals.skipped} dry_run=${aggregate.totals.dryRun} duration_ms=${aggregate.totals.durationMs ?? "-"}`,
  ]

  const classifications = Object.entries(aggregate.classifications)
  if (classifications.length > 0) {
    lines.push(`classifications: ${classifications.map(([kind, count]) => `${kind}=${count}`).join(" ")}`)
  }
  const owners = Object.entries(aggregate.owners)
  if (owners.length > 0) {
    lines.push(`owners: ${owners.map(([owner, count]) => `${owner}=${count}`).join(" ")}`)
  }
  const matrixNames = Object.entries(aggregate.matrixNames ?? {})
  if (matrixNames.length > 0) {
    lines.push(`matrix_names: ${matrixNames.map(([name, count]) => `${name}=${count}`).join(" ")}`)
  }
  const deploymentPresets = Object.entries(aggregate.deploymentPresets ?? {})
  if (deploymentPresets.length > 0) {
    lines.push(`deployment_presets: ${deploymentPresets.map(([preset, count]) => `${preset}=${count}`).join(" ")}`)
  }
  const providers = Object.entries(aggregate.providers ?? {})
  if (providers.length > 0) {
    lines.push(`providers: ${providers.map(([provider, count]) => `${provider}=${count}`).join(" ")}`)
  }
  const scenarioIds = Object.entries(aggregate.scenarioIds ?? {})
  if (scenarioIds.length > 0) {
    lines.push(`scenario_ids: ${scenarioIds.map(([id, count]) => `${id}=${count}`).join(" ")}`)
  }
  const exitCriteria = Object.entries(aggregate.exitCriteria ?? {})
  if (exitCriteria.length > 0) {
    lines.push(`exit_criteria: ${exitCriteria.map(([status, count]) => `${status}=${count}`).join(" ")}`)
  }
  const runtimeSignals = Object.entries(aggregate.runtimeSignals ?? {})
  if (runtimeSignals.length > 0) {
    lines.push(`runtime_signals: ${runtimeSignals.map(([signal, count]) => `${signal}=${count}`).join(" ")}`)
    lines.push(`runtime_signal_owners: ${formatCountObject(aggregate.runtimeSignalOwners)}`)
  }
  const runtimeSignalScenarios = Object.entries(aggregate.runtimeSignalScenarios ?? {})
  if (runtimeSignalScenarios.length > 0) {
    lines.push("runtime_signal_sources:")
    for (const [signal, scenarios] of runtimeSignalScenarios) {
      lines.push(`- ${signal}: ${scenarios.map(formatRuntimeSignalScenarioRef).join(", ")}`)
    }
  }
  if (Array.isArray(aggregate.nextActions) && aggregate.nextActions.length > 0) {
    lines.push("next actions:")
    for (const action of aggregate.nextActions) {
      lines.push(`- owner=${action.owner} classification=${action.classification} count=${action.count}: ${action.nextAction}`)
      const sources = formatDrillAggregateNextActionSourceDetails(action.sourceDetails)
      if (sources) {
        lines.push(`  sources: ${sources}`)
      }
    }
  }
  if (Array.isArray(aggregate.plannedNextActions) && aggregate.plannedNextActions.length > 0) {
    lines.push("planned next actions:")
    for (const action of aggregate.plannedNextActions) {
      lines.push(`- owner=${action.owner} classification=${action.classification} count=${action.count}: ${action.plannedNextAction}`)
      const sources = formatDrillAggregateNextActionSourceDetails(action.sourceDetails)
      if (sources) {
        lines.push(`  sources: ${sources}`)
      }
    }
  }

  if (aggregate.failedScenarios.length > 0) {
    lines.push("failed scenarios:")
    for (const scenario of aggregate.failedScenarios) {
      const classification = scenario.classification ? ` classification=${scenario.classification}` : ""
      const reason = scenario.reason ? ` reason=${scenario.reason}` : ""
      const source = scenario.source ? ` source=${scenario.source}` : ""
      lines.push(`- ${scenario.matrix}/${scenario.id}${classification} owner=${scenario.owner}${reason}${source}`)
      if (Array.isArray(scenario.artifactHints) && scenario.artifactHints.length > 0) {
        lines.push(`  artifacts: ${scenario.artifactHints.join(", ")}`)
      }
      lines.push(`  next: ${scenario.nextAction}`)
    }
  }

  if (aggregate.incompleteScenarios.length > 0) {
    lines.push("incomplete scenarios:")
    for (const scenario of aggregate.incompleteScenarios) {
      const reason = scenario.reason ? ` reason=${scenario.reason}` : ""
      const source = scenario.source ? ` source=${scenario.source}` : ""
      const plannedOwner = scenario.plannedOwner ? ` planned_owner=${scenario.plannedOwner}` : ""
      const plannedClassification = scenario.plannedClassification ? ` planned_classification=${scenario.plannedClassification}` : ""
      const plannedNextAction = scenario.plannedNextAction ? ` planned_next=${scenario.plannedNextAction}` : ""
      lines.push(`- ${scenario.matrix}/${scenario.id} status=${scenario.status}${reason}${source}${plannedOwner}${plannedClassification}${plannedNextAction}`)
    }
  }

  if ((aggregate.incompleteExitCriteria ?? []).length > 0) {
    lines.push("incomplete exit criteria:")
    for (const criterion of aggregate.incompleteExitCriteria) {
      const source = criterion.source ? ` source=${criterion.source}` : ""
      const reason = criterion.reason ? ` reason=${criterion.reason}` : ""
      const owner = criterion.owner ? ` owner=${criterion.owner}` : ""
      const classification = criterion.classification ? ` classification=${criterion.classification}` : ""
      const nextAction = criterion.nextAction ? ` next=${criterion.nextAction}` : ""
      lines.push(`- ${criterion.matrix}/${criterion.scenarioId}/${criterion.id} status=${criterion.status}${owner}${classification}${reason}${source}: ${criterion.criterion}${nextAction}`)
    }
  }

  if (aggregate.failedScenarios.length === 0 && aggregate.incompleteScenarios.length === 0 && (aggregate.incompleteExitCriteria ?? []).length === 0) {
    lines.push("next: all selected matrix scenarios completed without failures")
  } else if (aggregate.failedScenarios.length === 0) {
    lines.push("next: run or reconcile incomplete scenarios and criteria before treating this matrix set as complete")
  }

  return lines.join("\n")
}

export function validateDrillMatrixAggregate(aggregate) {
  if (!aggregate || typeof aggregate !== "object") {
    throw new Error("aggregate is not an object")
  }
  if (aggregate.schema !== "chariox.drill.matrix.aggregate.v1") {
    throw new Error(`aggregate has unsupported schema ${JSON.stringify(aggregate.schema)}`)
  }
  validateDrillMatrixReportStatus(aggregate.status, "aggregate")
  if (!aggregate.totals || typeof aggregate.totals !== "object") {
    throw new Error("aggregate is missing totals")
  }
  for (const key of ["reports", "scenarios", "passed", "failed", "skipped", "dryRun", "durationMs"]) {
    if (!Number.isSafeInteger(aggregate.totals[key]) || aggregate.totals[key] < 0) {
      throw new Error(`aggregate totals has invalid ${key}`)
    }
  }
  if (!Array.isArray(aggregate.failedScenarios)) {
    throw new Error("aggregate is missing failedScenarios")
  }
  for (const [index, scenario] of aggregate.failedScenarios.entries()) {
    validateMatrixAggregateScenario(scenario, `aggregate.failedScenarios[${index}]`)
    validateMatrixAggregateFailedScenario(scenario, `aggregate.failedScenarios[${index}]`)
  }
  if (!aggregate.owners || typeof aggregate.owners !== "object" || Array.isArray(aggregate.owners)) {
    throw new Error("aggregate is missing owners")
  }
  validateFailureClassificationCountObject(aggregate.classifications, "aggregate.classifications")
  validateCountObject(aggregate.matrixNames ?? {}, "aggregate.matrixNames")
  validateDeploymentPresetCountObject(aggregate.deploymentPresets, "aggregate.deploymentPresets")
  validateProviderCountObject(aggregate.providers ?? {}, "aggregate.providers")
  validateCountObject(aggregate.scenarioIds ?? {}, "aggregate.scenarioIds")
  validateExitCriteriaCountObject(aggregate.exitCriteria ?? {}, "aggregate.exitCriteria")
  validateRuntimeSignalCountObject(aggregate.runtimeSignals ?? {}, "aggregate.runtimeSignals")
  validateCountObject(aggregate.runtimeSignalOwners ?? {}, "aggregate.runtimeSignalOwners")
  if (aggregate.runtimeSignalScenarios !== undefined) {
    validateRuntimeSignalEvidenceObject(aggregate.runtimeSignalScenarios, "aggregate.runtimeSignalScenarios", { aggregate: true })
    assertRuntimeSignalEvidenceCounts("aggregate runtimeSignals", aggregate.runtimeSignals ?? {}, aggregate.runtimeSignalScenarios)
  }
  if (aggregate.nextActions !== undefined && !Array.isArray(aggregate.nextActions)) {
    throw new Error("aggregate has invalid nextActions")
  }
  for (const [index, action] of (aggregate.nextActions ?? []).entries()) {
    validateDrillAggregateNextAction(action, `aggregate.nextActions[${index}]`)
  }
  if (aggregate.plannedNextActions !== undefined && !Array.isArray(aggregate.plannedNextActions)) {
    throw new Error("aggregate has invalid plannedNextActions")
  }
  for (const [index, action] of (aggregate.plannedNextActions ?? []).entries()) {
    validateMatrixAggregatePlannedNextAction(action, `aggregate.plannedNextActions[${index}]`)
  }
  if (!Array.isArray(aggregate.skippedScenarios)) {
    throw new Error("aggregate is missing skippedScenarios")
  }
  for (const [index, scenario] of aggregate.skippedScenarios.entries()) {
    validateMatrixAggregateScenario(scenario, `aggregate.skippedScenarios[${index}]`)
  }
  if (!Array.isArray(aggregate.incompleteScenarios)) {
    throw new Error("aggregate is missing incompleteScenarios")
  }
  for (const [index, scenario] of aggregate.incompleteScenarios.entries()) {
    validateMatrixAggregateScenario(scenario, `aggregate.incompleteScenarios[${index}]`)
  }
  if (!Array.isArray(aggregate.incompleteExitCriteria ?? [])) {
    throw new Error("aggregate has invalid incompleteExitCriteria")
  }
  for (const [index, criterion] of (aggregate.incompleteExitCriteria ?? []).entries()) {
    validateMatrixAggregateExitCriterion(criterion, `aggregate.incompleteExitCriteria[${index}]`)
  }
  if (!Array.isArray(aggregate.reports)) {
    throw new Error("aggregate is missing reports")
  }
  for (const [index, report] of aggregate.reports.entries()) {
    validateMatrixAggregateReport(report, `aggregate.reports[${index}]`)
  }
  validateDrillMatrixAggregateConsistency(aggregate)
}

export function validateDrillMatrixAggregateConsistency(aggregate) {
  const scenarioTotal = aggregate.totals.passed + aggregate.totals.failed + aggregate.totals.skipped + aggregate.totals.dryRun
  if (aggregate.totals.scenarios !== scenarioTotal) {
    throw new Error("aggregate scenario total does not match status counts")
  }
  if (aggregate.totals.reports !== aggregate.reports.length) {
    throw new Error("aggregate report total does not match reports")
  }
  const reportTotals = sumMatrixAggregateReportEntries(aggregate.reports)
  for (const key of ["scenarios", "passed", "failed", "skipped", "dryRun", "durationMs"]) {
    if (aggregate.totals[key] !== reportTotals[key]) {
      throw new Error(`aggregate totals.${key} does not match reports`)
    }
  }
  if (aggregate.totals.failed !== aggregate.failedScenarios.length) {
    throw new Error("aggregate failed total does not match failedScenarios")
  }
  if (aggregate.totals.skipped + aggregate.totals.dryRun !== aggregate.incompleteScenarios.length) {
    throw new Error("aggregate incomplete total does not match incompleteScenarios")
  }
  if (incompleteExitCriteriaCount(aggregate.exitCriteria ?? {}) !== (aggregate.incompleteExitCriteria ?? []).length) {
    throw new Error("aggregate exitCriteria do not match incompleteExitCriteria")
  }
  const expectedStatus = aggregate.totals.failed > 0
    ? "failed"
    : aggregate.totals.reports > 0 && aggregate.totals.dryRun === aggregate.totals.scenarios
      ? "dry-run"
      : "passed"
  if (aggregate.status !== expectedStatus) {
    throw new Error("aggregate status does not match totals")
  }
  assertObjectCountsMatchEntries("aggregate owners", aggregate.owners, aggregate.failedScenarios, "owner")
  assertClassificationCountsMatchReports(aggregate)
  assertMatrixNameCountsMatchReports(aggregate)
  assertDeploymentPresetCountsMatchReports(aggregate)
  assertProviderCountsMatchReports(aggregate)
  assertScenarioIdCountsMatchReports(aggregate)
  assertExitCriteriaCountsMatchReports(aggregate)
  assertRuntimeSignalCountsMatchReports(aggregate)
  assertRuntimeSignalOwnerCountsMatchSignals(aggregate)
  assertRuntimeSignalScenariosMatchReports(aggregate)
  assertRuntimeSignalScenarioStatusesMatchDiagnostics(aggregate)
  assertNextActionCountsMatchScenarios(aggregate)
  assertPlannedNextActionCountsMatchScenarios(aggregate)
}

export function assertObjectCountsMatchEntries(label, counts, entries, key) {
  const expected = countDrillAggregateEntriesBy(
    entries.filter((entry) => typeof entry[key] === "string" && entry[key]),
    (entry) => entry[key],
  )
  if (JSON.stringify(counts) !== JSON.stringify(expected)) {
    throw new Error(`${label} do not match failedScenarios`)
  }
}

export function assertMatrixNameCountsMatchReports(aggregate) {
  const expected = new Map()
  for (const report of aggregate.reports) {
    expected.set(report.matrix, (expected.get(report.matrix) ?? 0) + 1)
  }
  const expectedCounts = Object.fromEntries([...expected.entries()].sort(([left], [right]) => left.localeCompare(right)))
  if (JSON.stringify(aggregate.matrixNames ?? {}) !== JSON.stringify(expectedCounts)) {
    throw new Error("aggregate matrixNames do not match reports")
  }
}

export function assertClassificationCountsMatchReports(aggregate) {
  const expected = new Map()
  for (const report of aggregate.reports) {
    for (const [classification, count] of Object.entries(report.classifications ?? {})) {
      expected.set(classification, (expected.get(classification) ?? 0) + count)
    }
  }
  const expectedCounts = Object.fromEntries([...expected.entries()].sort(([left], [right]) => left.localeCompare(right)))
  if (JSON.stringify(aggregate.classifications ?? {}) !== JSON.stringify(expectedCounts)) {
    throw new Error("aggregate classifications do not match reports")
  }
}

export function assertNextActionCountsMatchScenarios(aggregate) {
  const expected = new Map()
  for (const scenario of aggregate.failedScenarios) {
    countDrillAggregateNextAction(expected, {
      owner: scenario.owner,
      classification: scenario.classification ?? "child-process",
      nextAction: scenario.nextAction,
      sourceDetails: [nextActionSourceDetailForAggregateScenario(scenario)],
    })
  }
  const expectedActions = formatDrillAggregateNextActionCounts(expected)
  if (JSON.stringify(aggregate.nextActions ?? []) !== JSON.stringify(expectedActions)) {
    throw new Error("aggregate nextActions do not match failedScenarios")
  }
}

export function assertPlannedNextActionCountsMatchScenarios(aggregate) {
  const expected = new Map()
  for (const scenario of aggregate.incompleteScenarios) {
    if (!scenario.plannedNextAction) continue
    countDrillAggregateNextAction(expected, {
      owner: scenario.plannedOwner,
      classification: scenario.plannedClassification,
      nextAction: scenario.plannedNextAction,
      sourceDetails: [nextActionSourceDetailForAggregateScenario(scenario)],
    })
  }
  const expectedActions = formatDrillAggregateNextActionCounts(expected)
    .map((action) => ({
      owner: action.owner,
      classification: action.classification,
      plannedNextAction: action.nextAction,
      count: action.count,
      ...(action.sourceDetails ? { sourceDetails: action.sourceDetails } : {}),
    }))
  if (JSON.stringify(aggregate.plannedNextActions ?? []) !== JSON.stringify(expectedActions)) {
    throw new Error("aggregate plannedNextActions do not match incompleteScenarios")
  }
}

export function nextActionSourceDetailForAggregateScenario(scenario) {
  return {
    source: `${scenario.matrix}/${scenario.id}`,
    matrix: scenario.matrix,
    scenarioId: scenario.id,
    ...(scenario.source ? { reportPath: scenario.source } : {}),
  }
}

export function assertDeploymentPresetCountsMatchReports(aggregate) {
  const expected = new Map()
  for (const report of aggregate.reports) {
    for (const preset of report.deploymentPresets ?? []) {
      expected.set(preset, (expected.get(preset) ?? 0) + 1)
    }
  }
  const expectedCounts = Object.fromEntries([...expected.entries()].sort(([left], [right]) => left.localeCompare(right)))
  if (JSON.stringify(aggregate.deploymentPresets ?? {}) !== JSON.stringify(expectedCounts)) {
    throw new Error("aggregate deploymentPresets do not match reports")
  }
}

export function assertProviderCountsMatchReports(aggregate) {
  const expected = new Map()
  for (const report of aggregate.reports) {
    for (const provider of report.providers ?? []) {
      expected.set(provider, (expected.get(provider) ?? 0) + 1)
    }
  }
  const expectedCounts = Object.fromEntries([...expected.entries()].sort(([left], [right]) => left.localeCompare(right)))
  if (JSON.stringify(aggregate.providers ?? {}) !== JSON.stringify(expectedCounts)) {
    throw new Error("aggregate providers do not match reports")
  }
}

export function assertScenarioIdCountsMatchReports(aggregate) {
  const expected = new Map()
  for (const report of aggregate.reports) {
    for (const scenarioId of report.scenarioIds ?? []) {
      expected.set(scenarioId, (expected.get(scenarioId) ?? 0) + 1)
    }
  }
  const expectedCounts = Object.fromEntries([...expected.entries()].sort(([left], [right]) => left.localeCompare(right)))
  if (JSON.stringify(aggregate.scenarioIds ?? {}) !== JSON.stringify(expectedCounts)) {
    throw new Error("aggregate scenarioIds do not match reports")
  }
}

export function assertExitCriteriaCountsMatchReports(aggregate) {
  const expected = new Map()
  for (const report of aggregate.reports) {
    for (const [status, count] of Object.entries(report.exitCriteria ?? {})) {
      expected.set(status, (expected.get(status) ?? 0) + count)
    }
  }
  const expectedCounts = Object.fromEntries([...expected.entries()].sort(([left], [right]) => left.localeCompare(right)))
  if (JSON.stringify(aggregate.exitCriteria ?? {}) !== JSON.stringify(expectedCounts)) {
    throw new Error("aggregate exitCriteria do not match reports")
  }
}

export function assertRuntimeSignalCountsMatchReports(aggregate) {
  const expected = new Map()
  for (const report of aggregate.reports) {
    for (const [signal, count] of Object.entries(report.runtimeSignals ?? {})) {
      expected.set(signal, (expected.get(signal) ?? 0) + count)
    }
  }
  const expectedCounts = Object.fromEntries([...expected.entries()].sort(([left], [right]) => left.localeCompare(right)))
  if (JSON.stringify(aggregate.runtimeSignals ?? {}) !== JSON.stringify(expectedCounts)) {
    throw new Error("aggregate runtimeSignals do not match reports")
  }
}

export function assertRuntimeSignalOwnerCountsMatchSignals(aggregate) {
  const expectedOwners = drillRuntimeSignalOwnerCounts(aggregate.runtimeSignals ?? {})
  if (JSON.stringify(aggregate.runtimeSignalOwners ?? {}) !== JSON.stringify(expectedOwners)) {
    throw new Error("aggregate runtimeSignalOwners do not match runtimeSignals")
  }
}

export function assertRuntimeSignalScenariosMatchReports(aggregate) {
  if (aggregate.runtimeSignalScenarios === undefined) return
  const expected = new Map()
  for (const report of aggregate.reports) {
    for (const [signal, scenarios] of Object.entries(report.runtimeSignalScenarios ?? {})) {
      for (const scenario of scenarios) {
        appendRuntimeSignalEvidence(expected, signal, {
          matrix: report.matrix,
          source: report.source,
          id: scenario.id,
          status: scenario.status,
        })
      }
    }
  }
  const expectedEvidence = formatRuntimeSignalEvidence(expected)
  if (JSON.stringify(aggregate.runtimeSignalScenarios ?? {}) !== JSON.stringify(expectedEvidence)) {
    throw new Error("aggregate runtimeSignalScenarios do not match reports")
  }
}

export function assertRuntimeSignalScenarioStatusesMatchDiagnostics(aggregate) {
  if (aggregate.runtimeSignalScenarios === undefined) return
  const expectedStatuses = new Map()
  for (const scenario of aggregate.failedScenarios ?? []) {
    setExpectedRuntimeSignalScenarioStatus(expectedStatuses, scenario, "failed")
  }
  for (const scenario of aggregate.incompleteScenarios ?? []) {
    setExpectedRuntimeSignalScenarioStatus(expectedStatuses, scenario, scenario.status)
  }
  for (const scenarios of Object.values(aggregate.runtimeSignalScenarios ?? {})) {
    for (const scenario of scenarios) {
      const expectedStatus = expectedStatuses.get(runtimeSignalScenarioStatusKey(scenario)) ?? "passed"
      if (scenario.status !== expectedStatus) {
        throw new Error(`aggregate runtimeSignalScenarios status does not match scenario diagnostics for ${scenario.matrix}/${scenario.id}`)
      }
    }
  }
}

export function setExpectedRuntimeSignalScenarioStatus(expectedStatuses, scenario, status) {
  const key = runtimeSignalScenarioStatusKey(scenario)
  const existing = expectedStatuses.get(key)
  if (existing !== undefined && existing !== status) {
    throw new Error(`aggregate scenario diagnostics have conflicting status for ${scenario.matrix}/${scenario.id}`)
  }
  expectedStatuses.set(key, status)
}

export function runtimeSignalScenarioStatusKey(scenario) {
  return JSON.stringify([scenario.matrix, scenario.source ?? null, scenario.id])
}

export function validateMatrixAggregateReport(report, source) {
  if (!report || typeof report !== "object" || Array.isArray(report)) {
    throw new Error(`${source} is not an object`)
  }
  if (!nonEmptyString(report.matrix)) {
    throw new Error(`${source} is missing matrix`)
  }
  if (report.source !== null && report.source !== undefined && !nonEmptyString(report.source)) {
    throw new Error(`${source} has invalid source`)
  }
  validateDrillMatrixReportStatus(report.status, source)
  validateFailureClassificationCountObject(report.classifications, `${source}.classifications`)
  validateDeploymentPresetList(report.deploymentPresets, `${source}.deploymentPresets`)
  validateProviderList(report.providers ?? [], `${source}.providers`)
  if (!Array.isArray(report.scenarioIds ?? []) || !(report.scenarioIds ?? []).every(nonEmptyString)) {
    throw new Error(`${source} has invalid scenarioIds`)
  }
  validateExitCriteriaCountObject(report.exitCriteria ?? {}, `${source}.exitCriteria`)
  validateRuntimeSignalCountObject(report.runtimeSignals ?? {}, `${source}.runtimeSignals`)
  if (report.runtimeSignalScenarios !== undefined) {
    validateRuntimeSignalEvidenceObject(report.runtimeSignalScenarios, `${source}.runtimeSignalScenarios`, { aggregate: false })
    assertRuntimeSignalEvidenceCounts(`${source}.runtimeSignals`, report.runtimeSignals ?? {}, report.runtimeSignalScenarios)
  }
  if (!Number.isSafeInteger(report.scenarioCount) || report.scenarioCount < 0) {
    throw new Error(`${source} has invalid scenarioCount`)
  }
  validateMatrixAggregateReportCounts(report.counts, `${source}.counts`)
  if (!Number.isSafeInteger(report.durationMs) || report.durationMs < 0) {
    throw new Error(`${source} has invalid durationMs`)
  }
  const scenarioCount = report.counts.passed + report.counts.failed + report.counts.skipped + report.counts.dryRun
  if (report.scenarioCount !== scenarioCount) {
    throw new Error(`${source} scenarioCount does not match counts`)
  }
  if ((report.scenarioIds ?? []).length !== report.scenarioCount) {
    throw new Error(`${source} scenarioIds do not match scenarioCount`)
  }
  if (report.runtimeSignalScenarios !== undefined) {
    assertRuntimeSignalEvidenceScenarioIds(`${source}.runtimeSignalScenarios`, report.scenarioIds ?? [], report.runtimeSignalScenarios)
  }
  const expectedStatus = report.counts.failed > 0
    ? "failed"
    : report.scenarioCount > 0 && report.counts.dryRun === report.scenarioCount
      ? "dry-run"
      : "passed"
  if (report.status !== expectedStatus) {
    throw new Error(`${source} status does not match counts`)
  }
}

export function validateMatrixAggregateExitCriterion(criterion, source) {
  validateMatrixAggregateScenario({
    matrix: criterion.matrix,
    source: criterion.source,
    id: criterion.scenarioId,
  }, source)
  validateExitCriterionEvidence(criterion, source)
  if (criterion.status === "satisfied") {
    throw new Error(`${source} must not be satisfied`)
  }
  validateOptionalCriterionDiagnostics(criterion, source)
}

export function validateMatrixAggregateScenario(scenario, source) {
  if (!scenario || typeof scenario !== "object" || Array.isArray(scenario)) {
    throw new Error(`${source} is not an object`)
  }
  if (!nonEmptyString(scenario.matrix)) {
    throw new Error(`${source} is missing matrix`)
  }
  if (!nonEmptyString(scenario.id)) {
    throw new Error(`${source} is missing id`)
  }
  if (scenario.source !== null && scenario.source !== undefined && !nonEmptyString(scenario.source)) {
    throw new Error(`${source} has invalid source`)
  }
  if (scenario.artifactHints !== undefined && (
    !Array.isArray(scenario.artifactHints)
    || !scenario.artifactHints.every(isValidArtifactHint)
  )) {
    throw new Error(`${source} has invalid artifactHints`)
  }
  if (scenario.artifactHints?.some(artifactHintLooksSecret)) {
    throw new Error(`${source} includes secret-looking artifactHints`)
  }
  validateMatrixAggregatePlannedScenarioDiagnostics(scenario, source)
}

export function validateMatrixAggregatePlannedScenarioDiagnostics(scenario, source) {
  const hasPlannedClassification = scenario.plannedClassification !== undefined && scenario.plannedClassification !== null
  const hasPlannedOwner = scenario.plannedOwner !== undefined && scenario.plannedOwner !== null
  const hasPlannedNextAction = scenario.plannedNextAction !== undefined && scenario.plannedNextAction !== null
  if (!hasPlannedClassification && !hasPlannedOwner && !hasPlannedNextAction) return
  if (scenario.status !== "dry-run") {
    throw new Error(`${source} planned diagnostics require dry-run status`)
  }
  if (!nonEmptyString(scenario.plannedClassification)) {
    throw new Error(`${source} has invalid plannedClassification`)
  }
  validateMatrixAggregatePlannedNextAction({
    owner: scenario.plannedOwner,
    classification: scenario.plannedClassification,
    plannedNextAction: scenario.plannedNextAction,
    count: 1,
  }, source)
}

export function validateMatrixAggregatePlannedNextAction(action, source) {
  validateDrillAggregateNextAction({
    owner: action?.owner,
    classification: action?.classification,
    nextAction: action?.plannedNextAction,
    count: action?.count,
    ...(action?.sourceDetails !== undefined ? { sourceDetails: action.sourceDetails } : {}),
  }, source)
}

export function validateMatrixAggregateFailedScenario(scenario, source) {
  if (!nonEmptyString(scenario.classification)) {
    throw new Error(`${source} is missing classification`)
  }
  validateDrillFailureClassification(scenario.classification, source)
  if (!nonEmptyString(scenario.owner)) {
    throw new Error(`${source} is missing owner`)
  }
  if (scenario.owner !== drillFailureOwnerForClassification(scenario.classification)) {
    throw new Error(`${source} owner does not match classification`)
  }
  if (!nonEmptyString(scenario.reason)) {
    throw new Error(`${source} is missing reason`)
  }
  const expectedNextAction = drillFailureNextActionForClassification(scenario.classification, { target: "scenario" })
  if (scenario.nextAction !== expectedNextAction) {
    throw new Error(`${source} nextAction does not match classification`)
  }
}
