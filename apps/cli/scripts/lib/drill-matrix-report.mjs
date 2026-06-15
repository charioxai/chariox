import { readFile } from "node:fs/promises"
import {
  countDrillAggregateEntriesBy,
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  validateDrillAggregateNextAction,
} from "./drill-aggregate-actions.mjs"
import {
  drillFailureNextActionForClassification,
  drillFailureOwnerForClassification,
  isKnownDrillFailureClassification,
} from "./drill-failure-taxonomy.mjs"
import { drillRuntimeSignalOwnerCounts, isKnownDrillRuntimeSignal } from "./drill-runtime-signals.mjs"
import {
  isSensitiveDrillKey,
  looksLikeDrillSecretValue,
} from "./drill-secrets.mjs"
import { findDrillJsonArtifactPaths } from "./drill-json-discovery.mjs"
import { validateDrillDeploymentPresets } from "./drill-environment-presets.mjs"
import { validateDrillProviders } from "./drill-provider-profiles.mjs"
import {
  validateDrillDurationMatchesTimestamps,
  validateDrillTimestampOrder,
} from "./drill-time.mjs"

const DRILL_MATRIX_REPORT_SCHEMA = "arroba.drill.matrix.v1"

export async function readDrillMatrixReport(reportPath) {
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  validateDrillMatrixReport(report, reportPath)
  return report
}

export async function findDrillMatrixReportPaths(roots, { maxDepth = 8 } = {}) {
  return await findDrillJsonArtifactPaths(roots, {
    maxDepth,
    schema: DRILL_MATRIX_REPORT_SCHEMA,
  })
}

export function validateDrillMatrixReport(report, source = "report") {
  if (!report || typeof report !== "object") {
    throw new Error(`${source} is not an object`)
  }
  if (report.schema !== DRILL_MATRIX_REPORT_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(report.schema)}`)
  }
  if (!nonEmptyString(report.matrix)) {
    throw new Error(`${source} is missing matrix`)
  }
  if (!["passed", "failed", "dry-run"].includes(report.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(report.status)}`)
  }
  if (typeof report.dryRun !== "boolean") {
    throw new Error(`${source} is missing dryRun`)
  }
  validateDrillTimestampOrder(report, source)
  if (!Number.isSafeInteger(report.durationMs) || report.durationMs < 0) {
    throw new Error(`${source} has invalid durationMs`)
  }
  validateDrillDurationMatchesTimestamps(report, source)
  validateReportMetadata(report.metadata, `${source}.metadata`)
  if (!Array.isArray(report.scenarios)) {
    throw new Error(`${source} is missing scenarios`)
  }
  if (report.scenarios.length === 0) {
    throw new Error(`${source} has no scenarios`)
  }
  for (const [index, scenario] of report.scenarios.entries()) {
    validateDrillMatrixScenario(scenario, `${source}.scenarios[${index}]`)
  }
  validateDrillMatrixReportConsistency(report, source)
  validateScenarioProviderMetadataConsistency(report, source)
}

function validateDrillMatrixReportConsistency(report, source) {
  const counts = countScenarioStatuses(report.scenarios)
  const expectedStatus = counts.failed > 0
    ? "failed"
    : counts.dryRun === report.scenarios.length
      ? "dry-run"
      : "passed"
  if (report.status !== expectedStatus) {
    throw new Error(`${source} status does not match scenario statuses`)
  }
  if (report.dryRun !== (expectedStatus === "dry-run")) {
    throw new Error(`${source} dryRun does not match scenario statuses`)
  }
  if (report.exitCriteria !== undefined) {
    validateExitCriteriaCountObject(report.exitCriteria, `${source}.exitCriteria`)
    if (JSON.stringify(report.exitCriteria) !== JSON.stringify(countExitCriteriaStatuses(report.scenarios))) {
      throw new Error(`${source}.exitCriteria do not match scenario exit criteria evidence`)
    }
  }
  if (report.incompleteExitCriteria !== undefined) {
    validateMatrixReportIncompleteExitCriteria(report.incompleteExitCriteria, `${source}.incompleteExitCriteria`)
    if (!sameIncompleteExitCriteria(report.incompleteExitCriteria, incompleteExitCriteriaForScenarios(report.scenarios))) {
      throw new Error(`${source}.incompleteExitCriteria do not match scenario exit criteria evidence`)
    }
  }
  if (report.runtimeSignals !== undefined) {
    validateRuntimeSignalCountObject(report.runtimeSignals, `${source}.runtimeSignals`)
    if (JSON.stringify(report.runtimeSignals) !== JSON.stringify(runtimeSignalCountsForScenarios(report.scenarios))) {
      throw new Error(`${source}.runtimeSignals do not match scenario runtimeSignals`)
    }
  }
  if (report.runtimeSignalOwners !== undefined) {
    validateCountObject(report.runtimeSignalOwners, `${source}.runtimeSignalOwners`)
    if (report.runtimeSignals === undefined) {
      throw new Error(`${source}.runtimeSignalOwners requires runtimeSignals`)
    }
    if (JSON.stringify(report.runtimeSignalOwners) !== JSON.stringify(drillRuntimeSignalOwnerCounts(report.runtimeSignals))) {
      throw new Error(`${source}.runtimeSignalOwners do not match runtimeSignals`)
    }
  }
  if (report.runtimeSignalScenarios !== undefined) {
    validateRuntimeSignalEvidenceObject(report.runtimeSignalScenarios, `${source}.runtimeSignalScenarios`, { aggregate: false })
    if (report.runtimeSignals === undefined) {
      throw new Error(`${source}.runtimeSignalScenarios requires runtimeSignals`)
    }
    assertRuntimeSignalEvidenceCounts(`${source}.runtimeSignals`, report.runtimeSignals, report.runtimeSignalScenarios)
    const expectedEvidence = runtimeSignalScenariosForReport(report)
    if (JSON.stringify(report.runtimeSignalScenarios) !== JSON.stringify(expectedEvidence)) {
      throw new Error(`${source}.runtimeSignalScenarios do not match scenario runtimeSignals`)
    }
  }
}

export function summarizeDrillMatrixReport(report, { source = null } = {}) {
  validateDrillMatrixReport(report)
  const counts = countScenarioStatuses(report.scenarios)
  const classifications = new Map()
  const runtimeSignals = new Map()
  const runtimeSignalScenarios = new Map()
  for (const scenario of report.scenarios) {
    const classification = scenario.classification
    if (typeof classification === "string" && classification) {
      classifications.set(classification, (classifications.get(classification) ?? 0) + 1)
    }
    for (const signal of runtimeSignalsForScenario(scenario)) {
      runtimeSignals.set(signal, (runtimeSignals.get(signal) ?? 0) + 1)
      appendRuntimeSignalEvidence(runtimeSignalScenarios, signal, {
        id: scenario.id,
        status: scenario.status,
      })
    }
  }
  return {
    matrix: report.matrix,
    source,
    status: report.status,
    durationMs: report.durationMs,
    deploymentPresets: deploymentPresetsForReport(report),
    providers: providersForReport(report),
    scenarioIds: report.scenarios.map((scenario) => scenario.id),
    scenarioCount: report.scenarios.length,
    counts,
    classifications: Object.fromEntries([...classifications.entries()].sort(([left], [right]) => left.localeCompare(right))),
    runtimeSignals: Object.fromEntries([...runtimeSignals.entries()].sort(([left], [right]) => left.localeCompare(right))),
    runtimeSignalScenarios: formatRuntimeSignalEvidence(runtimeSignalScenarios),
    failedScenarios: report.scenarios.filter((scenario) => scenario.status === "failed"),
    skippedScenarios: report.scenarios.filter((scenario) => scenario.status === "skipped"),
    dryRunScenarios: report.scenarios.filter((scenario) => scenario.status === "dry-run"),
    exitCriteria: countExitCriteriaStatuses(report.scenarios),
    incompleteExitCriteria: incompleteExitCriteriaForScenarios(report.scenarios).map((criterion) => ({
      ...criterion,
      matrix: report.matrix,
      source,
    })),
  }
}

export function summarizeDrillMatrixReports(reports, { sources = [] } = {}) {
  const summaries = reports.map((report, index) => summarizeDrillMatrixReport(report, {
    source: sources[index] ?? null,
  }))
  const totals = {
    reports: summaries.length,
    scenarios: 0,
    passed: 0,
    failed: 0,
    skipped: 0,
    dryRun: 0,
    durationMs: 0,
  }
  const classifications = new Map()
  const runtimeSignals = new Map()
  const runtimeSignalScenarios = new Map()
  const owners = new Map()
  const nextActions = new Map()
  const plannedNextActions = new Map()
  const matrixNames = new Map()
  const deploymentPresets = new Map()
  const providers = new Map()
  const scenarioIds = new Map()
  const exitCriteria = new Map()
  const failedScenarios = []
  const skippedScenarios = []
  const incompleteScenarios = []
  const incompleteExitCriteria = []
  for (const summary of summaries) {
    totals.scenarios += summary.scenarioCount
    totals.passed += summary.counts.passed
    totals.failed += summary.counts.failed
    totals.skipped += summary.counts.skipped
    totals.dryRun += summary.counts.dryRun
    totals.durationMs += Number.isFinite(summary.durationMs) ? summary.durationMs : 0
    matrixNames.set(summary.matrix, (matrixNames.get(summary.matrix) ?? 0) + 1)
    for (const preset of summary.deploymentPresets) {
      deploymentPresets.set(preset, (deploymentPresets.get(preset) ?? 0) + 1)
    }
    for (const provider of summary.providers) {
      providers.set(provider, (providers.get(provider) ?? 0) + 1)
    }
    for (const scenarioId of summary.scenarioIds) {
      scenarioIds.set(scenarioId, (scenarioIds.get(scenarioId) ?? 0) + 1)
    }
    for (const [status, count] of Object.entries(summary.exitCriteria)) {
      exitCriteria.set(status, (exitCriteria.get(status) ?? 0) + count)
    }
    incompleteExitCriteria.push(...summary.incompleteExitCriteria)
    for (const [classification, count] of Object.entries(summary.classifications)) {
      classifications.set(classification, (classifications.get(classification) ?? 0) + count)
    }
    for (const [signal, count] of Object.entries(summary.runtimeSignals)) {
      runtimeSignals.set(signal, (runtimeSignals.get(signal) ?? 0) + count)
    }
    for (const [signal, scenarios] of Object.entries(summary.runtimeSignalScenarios)) {
      for (const scenario of scenarios) {
        appendRuntimeSignalEvidence(runtimeSignalScenarios, signal, {
          matrix: summary.matrix,
          source: summary.source,
          id: scenario.id,
          status: scenario.status,
        })
      }
    }
    for (const scenario of summary.failedScenarios) {
      failedScenarios.push({
        matrix: summary.matrix,
        source: summary.source,
        id: scenario.id,
        classification: scenario.classification ?? null,
        owner: ownerForScenario(scenario),
        reason: scenario.reason ?? null,
        artifactHints: artifactHintsForScenario(scenario),
        nextAction: nextActionForScenario(scenario),
      })
      const owner = ownerForScenario(scenario)
      owners.set(owner, (owners.get(owner) ?? 0) + 1)
      countDrillAggregateNextAction(nextActions, {
        owner,
        classification: scenario.classification ?? "child-process",
        nextAction: nextActionForScenario(scenario),
      })
    }
    for (const scenario of summary.skippedScenarios) {
      skippedScenarios.push({ matrix: summary.matrix, source: summary.source, id: scenario.id, reason: scenario.reason ?? null })
      incompleteScenarios.push({ matrix: summary.matrix, source: summary.source, id: scenario.id, status: "skipped", reason: scenario.reason ?? null })
    }
    for (const scenario of summary.dryRunScenarios) {
      const planned = plannedDiagnosticsForScenario(scenario)
      incompleteScenarios.push({
        matrix: summary.matrix,
        source: summary.source,
        id: scenario.id,
        status: "dry-run",
        reason: scenario.reason ?? null,
        ...planned,
      })
      if (planned.plannedNextAction) {
        countDrillAggregateNextAction(plannedNextActions, {
          owner: planned.plannedOwner,
          classification: planned.plannedClassification,
          nextAction: planned.plannedNextAction,
        })
      }
    }
  }
  return {
    schema: "arroba.drill.matrix.aggregate.v1",
    status: totals.failed > 0
      ? "failed"
      : totals.reports > 0 && totals.dryRun === totals.scenarios
        ? "dry-run"
        : "passed",
    totals,
    classifications: Object.fromEntries([...classifications.entries()].sort(([left], [right]) => left.localeCompare(right))),
    runtimeSignals: Object.fromEntries([...runtimeSignals.entries()].sort(([left], [right]) => left.localeCompare(right))),
    runtimeSignalOwners: drillRuntimeSignalOwnerCounts(Object.fromEntries(runtimeSignals)),
    runtimeSignalScenarios: formatRuntimeSignalEvidence(runtimeSignalScenarios),
    matrixNames: Object.fromEntries([...matrixNames.entries()].sort(([left], [right]) => left.localeCompare(right))),
    deploymentPresets: Object.fromEntries([...deploymentPresets.entries()].sort(([left], [right]) => left.localeCompare(right))),
    providers: Object.fromEntries([...providers.entries()].sort(([left], [right]) => left.localeCompare(right))),
    scenarioIds: Object.fromEntries([...scenarioIds.entries()].sort(([left], [right]) => left.localeCompare(right))),
    exitCriteria: Object.fromEntries([...exitCriteria.entries()].sort(([left], [right]) => left.localeCompare(right))),
    owners: Object.fromEntries([...owners.entries()].sort(([left], [right]) => left.localeCompare(right))),
    nextActions: formatDrillAggregateNextActionCounts(nextActions),
    plannedNextActions: formatDrillAggregateNextActionCounts(plannedNextActions)
      .map((action) => ({
        owner: action.owner,
        classification: action.classification,
        plannedNextAction: action.nextAction,
        count: action.count,
      })),
    reports: summaries.map((summary) => ({
      matrix: summary.matrix,
      source: summary.source,
      status: summary.status,
      deploymentPresets: summary.deploymentPresets,
      providers: summary.providers,
      scenarioIds: summary.scenarioIds,
      exitCriteria: summary.exitCriteria,
      classifications: summary.classifications,
      runtimeSignals: summary.runtimeSignals,
      runtimeSignalScenarios: summary.runtimeSignalScenarios,
      scenarioCount: summary.scenarioCount,
      counts: summary.counts,
      durationMs: summary.durationMs,
    })),
    failedScenarios,
    skippedScenarios,
    incompleteScenarios,
    incompleteExitCriteria,
  }
}

export function formatDrillMatrixReportSummary(report, { source = null } = {}) {
  const summary = summarizeDrillMatrixReport(report, { source })
  const lines = [
    `matrix report: ${summary.matrix}${source ? ` (${source})` : ""}`,
    `status=${summary.status} scenarios=${summary.scenarioCount} passed=${summary.counts.passed} failed=${summary.counts.failed} skipped=${summary.counts.skipped} dry_run=${summary.counts.dryRun} duration_ms=${summary.durationMs ?? "-"}`,
  ]

  const classifications = Object.entries(summary.classifications)
  if (classifications.length > 0) {
    lines.push(`classifications: ${classifications.map(([kind, count]) => `${kind}=${count}`).join(" ")}`)
  }
  const runtimeSignals = Object.entries(summary.runtimeSignals)
  if (runtimeSignals.length > 0) {
    lines.push(`runtime_signals: ${runtimeSignals.map(([signal, count]) => `${signal}=${count}`).join(" ")}`)
    lines.push(`runtime_signal_owners: ${formatCountObject(drillRuntimeSignalOwnerCounts(summary.runtimeSignals))}`)
  }

  if (summary.failedScenarios.length > 0) {
    lines.push("failed scenarios:")
    for (const scenario of summary.failedScenarios) {
      const classification = scenario.classification ? ` classification=${scenario.classification}` : ""
      const owner = ` owner=${ownerForScenario(scenario)}`
      const reason = scenario.reason ? ` reason=${scenario.reason}` : ""
      lines.push(`- ${scenario.id}${classification}${owner}${reason}`)
      const criteria = exitCriteriaForScenario(scenario)
      if (criteria.length > 0) {
        lines.push(`  criteria: ${criteria.join("; ")}`)
      }
      const artifactHints = artifactHintsForScenario(scenario)
      if (artifactHints.length > 0) {
        lines.push(`  artifacts: ${artifactHints.join(", ")}`)
      }
      lines.push(`  next: ${nextActionForScenario(scenario)}`)
    }
  }

  if (summary.skippedScenarios.length > 0) {
    lines.push(`skipped scenarios: ${summary.skippedScenarios.map((scenario) => scenario.id).join(", ")}`)
  }

  if (summary.incompleteExitCriteria.length > 0) {
    lines.push("incomplete exit criteria:")
    for (const criterion of summary.incompleteExitCriteria) {
      const reason = criterion.reason ? ` reason=${criterion.reason}` : ""
      const owner = criterion.owner ? ` owner=${criterion.owner}` : ""
      const classification = criterion.classification ? ` classification=${criterion.classification}` : ""
      const nextAction = criterion.nextAction ? ` next=${criterion.nextAction}` : ""
      lines.push(`- ${criterion.scenarioId}/${criterion.id} status=${criterion.status}${owner}${classification}${reason}: ${criterion.criterion}${nextAction}`)
    }
  }

  if (summary.status === "dry-run") {
    appendDryRunCriteria(lines, report.scenarios)
  }

  if (summary.failedScenarios.length === 0 && summary.incompleteExitCriteria.length === 0) {
    lines.push(summary.status === "dry-run" ? "next: run without --dry-run to execute selected scenarios" : "next: no failed matrix scenarios")
  } else if (summary.failedScenarios.length === 0) {
    lines.push("next: run or reconcile incomplete criteria before treating this matrix report as complete")
  }

  return lines.join("\n")
}

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
    }
  }
  if (Array.isArray(aggregate.plannedNextActions) && aggregate.plannedNextActions.length > 0) {
    lines.push("planned next actions:")
    for (const action of aggregate.plannedNextActions) {
      lines.push(`- owner=${action.owner} classification=${action.classification} count=${action.count}: ${action.plannedNextAction}`)
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

export function drillMatrixReportExitCode(reports) {
  return reports.some((report) => report.status === "failed") ? 1 : 0
}

export function drillMatrixReportCompletionExitCode(reports) {
  const aggregate = summarizeDrillMatrixReports(reports)
  if (aggregate.status === "failed") return 1
  return aggregate.incompleteScenarios.length > 0 || aggregate.incompleteExitCriteria.length > 0 ? 2 : 0
}

function nextActionForScenario(scenario) {
  return drillFailureNextActionForClassification(scenario.classification, { target: "scenario" })
}

function ownerForScenario(scenario) {
  return drillFailureOwnerForClassification(scenario.classification)
}

function plannedDiagnosticsForScenario(scenario) {
  if (!scenario.plannedClassification) return {}
  return {
    plannedClassification: scenario.plannedClassification,
    plannedOwner: scenario.plannedOwner,
    plannedNextAction: scenario.plannedNextAction,
  }
}

function appendDryRunCriteria(lines, scenarios) {
  const withCriteria = scenarios
    .map((scenario) => ({ scenario, criteria: exitCriteriaForScenario(scenario) }))
    .filter((entry) => entry.criteria.length > 0)
  if (withCriteria.length === 0) return
  lines.push("selected scenario criteria:")
  for (const { scenario, criteria } of withCriteria) {
    lines.push(`- ${scenario.id}: ${criteria.join("; ")}`)
  }
}

function validateDrillMatrixScenario(scenario, source) {
  if (!scenario || typeof scenario !== "object") {
    throw new Error(`${source} is not an object`)
  }
  if (!nonEmptyString(scenario.id)) {
    throw new Error(`${source} is missing id`)
  }
  if (!nonEmptyString(scenario.description)) {
    throw new Error(`${source} is missing description`)
  }
  if (!Array.isArray(scenario.requires) || !scenario.requires.every((value) => typeof value === "string")) {
    throw new Error(`${source} has invalid requires`)
  }
  if (scenario.exitCriteria !== undefined && (
    !Array.isArray(scenario.exitCriteria)
    || !scenario.exitCriteria.every((value) => typeof value === "string")
  )) {
    throw new Error(`${source} has invalid exitCriteria`)
  }
  if (scenario.exitCriteriaEvidence !== undefined) {
    validateExitCriteriaEvidence(scenario, `${source}.exitCriteriaEvidence`)
  }
  if (!["passed", "failed", "skipped", "dry-run"].includes(scenario.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(scenario.status)}`)
  }
  if (typeof scenario.expectedFailure !== "boolean") {
    throw new Error(`${source} is missing expectedFailure`)
  }
  if (scenario.classification !== null && typeof scenario.classification !== "string") {
    throw new Error(`${source} has invalid classification`)
  }
  if (nonEmptyString(scenario.classification) && !isKnownDrillFailureClassification(scenario.classification)) {
    throw new Error(`${source} has unknown classification ${JSON.stringify(scenario.classification)}`)
  }
  if (scenario.owner !== undefined && scenario.owner !== null) {
    if (!nonEmptyString(scenario.owner)) {
      throw new Error(`${source} has invalid owner`)
    }
    if (!nonEmptyString(scenario.classification)) {
      if (scenario.status !== "dry-run") {
        throw new Error(`${source} owner requires classification`)
      }
    } else {
      const expectedOwner = drillFailureOwnerForClassification(scenario.classification)
      if (scenario.owner !== expectedOwner) {
        throw new Error(`${source} owner does not match classification`)
      }
    }
  }
  if (scenario.nextAction !== undefined && scenario.nextAction !== null) {
    if (!nonEmptyString(scenario.nextAction)) {
      throw new Error(`${source} has invalid nextAction`)
    }
    if (!nonEmptyString(scenario.classification)) {
      throw new Error(`${source} nextAction requires classification`)
    }
    const expectedNextAction = drillFailureNextActionForClassification(scenario.classification, { target: "scenario" })
    if (scenario.nextAction !== expectedNextAction) {
      throw new Error(`${source} nextAction does not match classification`)
    }
  }
  validatePlannedScenarioDiagnostics(scenario, source)
  if (!Number.isSafeInteger(scenario.durationMs) || scenario.durationMs < 0) {
    throw new Error(`${source} has invalid durationMs`)
  }
  if (scenario.reason !== null && typeof scenario.reason !== "string") {
    throw new Error(`${source} has invalid reason`)
  }
  if (!nonEmptyString(scenario.command)) {
    throw new Error(`${source} is missing command`)
  }
  if (!Array.isArray(scenario.args) || !scenario.args.every((value) => typeof value === "string")) {
    throw new Error(`${source} has invalid args`)
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
  if (scenario.runtimeSignals !== undefined) {
    validateRuntimeSignals(scenario.runtimeSignals, `${source}.runtimeSignals`)
  }
  if (scenario.provider !== undefined) {
    validateProviderList([scenario.provider], `${source}.provider`)
    if (scenario.providers !== undefined && !scenario.providers.includes(scenario.provider)) {
      throw new Error(`${source}.provider must be included in providers`)
    }
  }
  if (scenario.providers !== undefined) {
    validateProviderList(scenario.providers, `${source}.providers`)
  }
  if (scenario.deployment !== undefined && !nonSecretString(scenario.deployment)) {
    throw new Error(`${source} has invalid deployment`)
  }
  if (scenario.mode !== undefined && !nonSecretString(scenario.mode)) {
    throw new Error(`${source} has invalid mode`)
  }
  validateDrillMatrixScenarioOutcome(scenario, source)
}

function validateDrillMatrixScenarioOutcome(scenario, source) {
  const hasReason = nonEmptyString(scenario.reason)
  const hasClassification = nonEmptyString(scenario.classification)
  if (scenario.status === "failed") {
    if (!hasReason) {
      throw new Error(`${source} failed scenario is missing reason`)
    }
    if (!hasClassification) {
      throw new Error(`${source} failed scenario is missing classification`)
    }
    return
  }
  if (scenario.status === "skipped") {
    if (!hasReason) {
      throw new Error(`${source} skipped scenario is missing reason`)
    }
    if (scenario.durationMs !== 0) {
      throw new Error(`${source} skipped scenario must have zero durationMs`)
    }
    return
  }
  if (scenario.status === "dry-run") {
    if (scenario.durationMs !== 0) {
      throw new Error(`${source} dry-run scenario must have zero durationMs`)
    }
    if (hasReason) {
      throw new Error(`${source} dry-run scenario must not include reason`)
    }
    if (hasClassification) {
      throw new Error(`${source} dry-run scenario must not include classification`)
    }
    return
  }
  if (hasReason) {
    throw new Error(`${source} passed scenario must not include reason`)
  }
}

export function validateDrillMatrixAggregate(aggregate) {
  if (!aggregate || typeof aggregate !== "object") {
    throw new Error("aggregate is not an object")
  }
  if (aggregate.schema !== "arroba.drill.matrix.aggregate.v1") {
    throw new Error(`aggregate has unsupported schema ${JSON.stringify(aggregate.schema)}`)
  }
  if (!["passed", "failed", "dry-run"].includes(aggregate.status)) {
    throw new Error(`aggregate has invalid status ${JSON.stringify(aggregate.status)}`)
  }
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

function validatePlannedScenarioDiagnostics(scenario, source) {
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
  if (!isKnownDrillFailureClassification(scenario.plannedClassification)) {
    throw new Error(`${source} has unknown plannedClassification ${JSON.stringify(scenario.plannedClassification)}`)
  }
  const expectedOwner = drillFailureOwnerForClassification(scenario.plannedClassification)
  if (scenario.plannedOwner !== expectedOwner) {
    throw new Error(`${source} plannedOwner does not match plannedClassification`)
  }
  const expectedNextAction = drillFailureNextActionForClassification(scenario.plannedClassification, { target: "scenario" })
  if (scenario.plannedNextAction !== expectedNextAction) {
    throw new Error(`${source} plannedNextAction does not match plannedClassification`)
  }
}

function validateDrillMatrixAggregateConsistency(aggregate) {
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

function assertObjectCountsMatchEntries(label, counts, entries, key) {
  const expected = countDrillAggregateEntriesBy(
    entries.filter((entry) => typeof entry[key] === "string" && entry[key]),
    (entry) => entry[key],
  )
  if (JSON.stringify(counts) !== JSON.stringify(expected)) {
    throw new Error(`${label} do not match failedScenarios`)
  }
}

function assertMatrixNameCountsMatchReports(aggregate) {
  const expected = new Map()
  for (const report of aggregate.reports) {
    expected.set(report.matrix, (expected.get(report.matrix) ?? 0) + 1)
  }
  const expectedCounts = Object.fromEntries([...expected.entries()].sort(([left], [right]) => left.localeCompare(right)))
  if (JSON.stringify(aggregate.matrixNames ?? {}) !== JSON.stringify(expectedCounts)) {
    throw new Error("aggregate matrixNames do not match reports")
  }
}

function assertClassificationCountsMatchReports(aggregate) {
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

function assertNextActionCountsMatchScenarios(aggregate) {
  const expected = new Map()
  for (const scenario of aggregate.failedScenarios) {
    countDrillAggregateNextAction(expected, {
      owner: scenario.owner,
      classification: scenario.classification ?? "child-process",
      nextAction: scenario.nextAction,
    })
  }
  const expectedActions = formatDrillAggregateNextActionCounts(expected)
  if (JSON.stringify(aggregate.nextActions ?? []) !== JSON.stringify(expectedActions)) {
    throw new Error("aggregate nextActions do not match failedScenarios")
  }
}

function assertPlannedNextActionCountsMatchScenarios(aggregate) {
  const expected = new Map()
  for (const scenario of aggregate.incompleteScenarios) {
    if (!scenario.plannedNextAction) continue
    countDrillAggregateNextAction(expected, {
      owner: scenario.plannedOwner,
      classification: scenario.plannedClassification,
      nextAction: scenario.plannedNextAction,
    })
  }
  const expectedActions = formatDrillAggregateNextActionCounts(expected)
    .map((action) => ({
      owner: action.owner,
      classification: action.classification,
      plannedNextAction: action.nextAction,
      count: action.count,
    }))
  if (JSON.stringify(aggregate.plannedNextActions ?? []) !== JSON.stringify(expectedActions)) {
    throw new Error("aggregate plannedNextActions do not match incompleteScenarios")
  }
}

function assertDeploymentPresetCountsMatchReports(aggregate) {
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

function assertProviderCountsMatchReports(aggregate) {
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

function assertScenarioIdCountsMatchReports(aggregate) {
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

function assertExitCriteriaCountsMatchReports(aggregate) {
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

function assertRuntimeSignalCountsMatchReports(aggregate) {
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

function assertRuntimeSignalOwnerCountsMatchSignals(aggregate) {
  const expectedOwners = drillRuntimeSignalOwnerCounts(aggregate.runtimeSignals ?? {})
  if (JSON.stringify(aggregate.runtimeSignalOwners ?? {}) !== JSON.stringify(expectedOwners)) {
    throw new Error("aggregate runtimeSignalOwners do not match runtimeSignals")
  }
}

function assertRuntimeSignalScenariosMatchReports(aggregate) {
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

function assertRuntimeSignalScenarioStatusesMatchDiagnostics(aggregate) {
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

function setExpectedRuntimeSignalScenarioStatus(expectedStatuses, scenario, status) {
  const key = runtimeSignalScenarioStatusKey(scenario)
  const existing = expectedStatuses.get(key)
  if (existing !== undefined && existing !== status) {
    throw new Error(`aggregate scenario diagnostics have conflicting status for ${scenario.matrix}/${scenario.id}`)
  }
  expectedStatuses.set(key, status)
}

function runtimeSignalScenarioStatusKey(scenario) {
  return JSON.stringify([scenario.matrix, scenario.source ?? null, scenario.id])
}

function validateMatrixAggregateReport(report, source) {
  if (!report || typeof report !== "object" || Array.isArray(report)) {
    throw new Error(`${source} is not an object`)
  }
  if (!nonEmptyString(report.matrix)) {
    throw new Error(`${source} is missing matrix`)
  }
  if (report.source !== null && report.source !== undefined && !nonEmptyString(report.source)) {
    throw new Error(`${source} has invalid source`)
  }
  if (!["passed", "failed", "dry-run"].includes(report.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(report.status)}`)
  }
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

function validateMatrixAggregateExitCriterion(criterion, source) {
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

function validateMatrixReportIncompleteExitCriteria(criteria, source) {
  if (!Array.isArray(criteria)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, criterion] of criteria.entries()) {
    const criterionSource = `${source}[${index}]`
    if (!criterion || typeof criterion !== "object" || Array.isArray(criterion)) {
      throw new Error(`${criterionSource} is not an object`)
    }
    if (!nonEmptyString(criterion.scenarioId)) {
      throw new Error(`${criterionSource} is missing scenarioId`)
    }
    validateExitCriterionEvidence(criterion, criterionSource)
    if (criterion.status === "satisfied") {
      throw new Error(`${criterionSource} must not be satisfied`)
    }
    validateOptionalCriterionDiagnostics(criterion, criterionSource)
  }
}

function validateOptionalCriterionDiagnostics(criterion, source) {
  if (criterion.owner !== undefined && criterion.owner !== null && !nonEmptyString(criterion.owner)) {
    throw new Error(`${source} has invalid owner`)
  }
  if (criterion.classification !== undefined && criterion.classification !== null) {
    if (!nonEmptyString(criterion.classification)) {
      throw new Error(`${source} has invalid classification`)
    }
    if (!isKnownDrillFailureClassification(criterion.classification)) {
      throw new Error(`${source} has unknown classification ${JSON.stringify(criterion.classification)}`)
    }
    const expectedOwner = drillFailureOwnerForClassification(criterion.classification)
    if (criterion.owner !== expectedOwner) {
      throw new Error(`${source} owner does not match classification`)
    }
    const expectedNextAction = drillFailureNextActionForClassification(criterion.classification, { target: "scenario" })
    if (criterion.nextAction !== expectedNextAction) {
      throw new Error(`${source} nextAction does not match classification`)
    }
  } else if (criterion.nextAction !== undefined && criterion.nextAction !== null) {
    throw new Error(`${source} nextAction requires classification`)
  }
}

function validateMatrixAggregateScenario(scenario, source) {
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

function validateMatrixAggregatePlannedScenarioDiagnostics(scenario, source) {
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

function validateMatrixAggregatePlannedNextAction(action, source) {
  validateDrillAggregateNextAction({
    owner: action?.owner,
    classification: action?.classification,
    nextAction: action?.plannedNextAction,
    count: action?.count,
  }, source)
}

function validateMatrixAggregateFailedScenario(scenario, source) {
  if (!nonEmptyString(scenario.classification)) {
    throw new Error(`${source} is missing classification`)
  }
  if (!isKnownDrillFailureClassification(scenario.classification)) {
    throw new Error(`${source} has unknown classification ${JSON.stringify(scenario.classification)}`)
  }
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

function validateReportMetadata(value, source) {
  if (!value || typeof value !== "object") {
    throw new Error(`${source} must be an object`)
  }
  validateReportMetadataValue(value, source)
  validateDeploymentPresetMetadata(value, source)
  validateProviderMetadata(value, source)
}

function validateDeploymentPresetMetadata(metadata, source) {
  const deploymentPresets = deploymentPresetsForReport({ metadata })
  if (deploymentPresets.length > 0) validateDeploymentPresetList(deploymentPresets, `${source}.deploymentPresets`)
  if (metadata.deploymentPresetCount !== undefined) {
    if (!Number.isInteger(metadata.deploymentPresetCount) || metadata.deploymentPresetCount < 0) {
      throw new Error(`${source}.deploymentPresetCount is invalid`)
    }
    if (metadata.deploymentPresetCount !== deploymentPresets.length) {
      throw new Error(`${source}.deploymentPresetCount does not match deploymentPresets`)
    }
  }
}

function validateProviderMetadata(metadata, source) {
  const providers = metadataListValue(metadata.providers)
  if (providers.length > 0) validateProviderList(providers, `${source}.providers`)
  if (metadata.providerCount !== undefined) {
    if (!Number.isInteger(metadata.providerCount) || metadata.providerCount < 0) {
      throw new Error(`${source}.providerCount is invalid`)
    }
    if (metadata.providerCount !== providers.length) {
      throw new Error(`${source}.providerCount does not match providers`)
    }
  }
  if (metadata.providerModelOverrides !== undefined) {
    if (typeof metadata.providerModelOverrides !== "string") {
      throw new Error(`${source}.providerModelOverrides is invalid`)
    }
    const providerSet = new Set(providers)
    for (const provider of metadataListValue(metadata.providerModelOverrides)) {
      if (!providerSet.has(provider)) {
        throw new Error(`${source}.providerModelOverrides includes provider not in providers`)
      }
    }
  }
  if (metadata.providerAccountAliases !== undefined) {
    if (typeof metadata.providerAccountAliases !== "string") {
      throw new Error(`${source}.providerAccountAliases is invalid`)
    }
    const providerSet = new Set(providers)
    for (const entry of metadataListValue(metadata.providerAccountAliases)) {
      const [provider, alias] = entry.split("=", 2).map((part) => part.trim())
      if (!providerSet.has(provider)) {
        throw new Error(`${source}.providerAccountAliases includes provider not in providers`)
      }
      if (!validProviderAccountAlias(alias)) {
        throw new Error(`${source}.providerAccountAliases includes invalid account alias`)
      }
    }
  }
}

function validProviderAccountAlias(alias) {
  return typeof alias === "string"
    && /^[a-zA-Z0-9._-]{1,64}$/.test(alias)
    && !looksLikeDrillSecretValue(alias)
}

function validateScenarioProviderMetadataConsistency(report, source) {
  if (!report.scenarios.some((scenario) => scenario.providers !== undefined)) return
  const metadataProviders = providersForReport(report)
  if (metadataProviders.length === 0) return
  const scenarioProviders = [...new Set(report.scenarios.flatMap((scenario) => scenario.providers ?? []))].sort()
  if (JSON.stringify(metadataProviders) !== JSON.stringify(scenarioProviders)) {
    throw new Error(`${source}.metadata.providers do not match scenario providers`)
  }
}

function validateMatrixAggregateReportCounts(counts, source) {
  if (!counts || typeof counts !== "object" || Array.isArray(counts)) {
    throw new Error(`${source} is missing`)
  }
  for (const key of ["passed", "failed", "skipped", "dryRun"]) {
    if (!Number.isSafeInteger(counts[key]) || counts[key] < 0) {
      throw new Error(`${source} has invalid ${key}`)
    }
  }
}

function sumMatrixAggregateReportEntries(reports) {
  return reports.reduce((totals, report) => {
    totals.scenarios += report.scenarioCount
    totals.passed += report.counts.passed
    totals.failed += report.counts.failed
    totals.skipped += report.counts.skipped
    totals.dryRun += report.counts.dryRun
    totals.durationMs += report.durationMs
    return totals
  }, {
    scenarios: 0,
    passed: 0,
    failed: 0,
    skipped: 0,
    dryRun: 0,
    durationMs: 0,
  })
}

function validateCountObject(value, source) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is missing`)
  }
  for (const [key, count] of Object.entries(value)) {
    if (!nonEmptyString(key) || !Number.isSafeInteger(count) || count < 0) {
      throw new Error(`${source} has invalid count for ${JSON.stringify(key)}`)
    }
  }
}

function validateExitCriteriaCountObject(value, source) {
  validateCountObject(value, source)
  for (const status of Object.keys(value)) {
    if (!["satisfied", "failed", "skipped", "dry-run"].includes(status)) {
      throw new Error(`${source} has invalid status ${JSON.stringify(status)}`)
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

function validateFailureClassificationCountObject(value, source) {
  validateCountObject(value, source)
  for (const classification of Object.keys(value)) {
    if (!isKnownDrillFailureClassification(classification)) {
      throw new Error(`${source} has unknown classification ${JSON.stringify(classification)}`)
    }
  }
}

function validateRuntimeSignalEvidenceObject(value, source, { aggregate }) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is missing`)
  }
  for (const [signal, scenarios] of Object.entries(value)) {
    if (!isKnownDrillRuntimeSignal(signal)) {
      throw new Error(`${source} has unknown runtime signal ${JSON.stringify(signal)}`)
    }
    if (!Array.isArray(scenarios) || scenarios.length === 0) {
      throw new Error(`${source}.${signal} has invalid scenarios`)
    }
    for (const [index, scenario] of scenarios.entries()) {
      validateRuntimeSignalEvidenceEntry(scenario, `${source}.${signal}[${index}]`, { aggregate })
    }
  }
}

function validateRuntimeSignalEvidenceEntry(entry, source, { aggregate }) {
  if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
    throw new Error(`${source} is not an object`)
  }
  if (aggregate && !nonEmptyString(entry.matrix)) {
    throw new Error(`${source} is missing matrix`)
  }
  if (aggregate && entry.source !== null && entry.source !== undefined && !nonEmptyString(entry.source)) {
    throw new Error(`${source} has invalid source`)
  }
  if (!nonEmptyString(entry.id)) {
    throw new Error(`${source} is missing id`)
  }
  if (!["passed", "failed", "skipped", "dry-run"].includes(entry.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(entry.status)}`)
  }
}

function assertRuntimeSignalEvidenceCounts(label, counts, evidence) {
  const expected = Object.fromEntries(Object.entries(evidence)
    .map(([signal, scenarios]) => [signal, scenarios.length])
    .sort(([left], [right]) => left.localeCompare(right)))
  if (JSON.stringify(counts) !== JSON.stringify(expected)) {
    throw new Error(`${label} do not match runtimeSignalScenarios`)
  }
}

function assertRuntimeSignalEvidenceScenarioIds(label, scenarioIds, evidence) {
  const knownScenarioIds = new Set(scenarioIds)
  for (const scenarios of Object.values(evidence)) {
    for (const scenario of scenarios) {
      if (!knownScenarioIds.has(scenario.id)) {
        throw new Error(`${label} references unknown scenario ${JSON.stringify(scenario.id)}`)
      }
    }
  }
}

function runtimeSignalCountsForScenarios(scenarios) {
  const counts = new Map()
  for (const scenario of scenarios) {
    for (const signal of runtimeSignalsForScenario(scenario)) {
      counts.set(signal, (counts.get(signal) ?? 0) + 1)
    }
  }
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

function runtimeSignalScenariosForReport(report) {
  const evidence = new Map()
  for (const scenario of report.scenarios) {
    for (const signal of runtimeSignalsForScenario(scenario)) {
      appendRuntimeSignalEvidence(evidence, signal, {
        id: scenario.id,
        status: scenario.status,
      })
    }
  }
  return formatRuntimeSignalEvidence(evidence)
}

function deploymentPresetsForReport(report) {
  const value = report.metadata?.deploymentPresets
  if (!nonEmptyString(value)) return []
  return [...new Set(value.split(",").map((preset) => preset.trim()).filter(Boolean))].sort()
}

function providersForReport(report) {
  return metadataListValue(report.metadata?.providers)
}

function metadataListValue(value) {
  if (!nonEmptyString(value)) return []
  return [...new Set(value.split(",").map((item) => item.trim()).filter(Boolean))].sort()
}

function validateReportMetadataValue(value, source, key = "") {
  if (isSensitiveDrillKey(key)) {
    throw new Error(`${source} includes sensitive metadata key ${JSON.stringify(key)}`)
  }
  if (typeof value === "string") {
    if (looksLikeDrillSecretValue(value)) {
      throw new Error(`${source} includes secret-looking metadata value`)
    }
    return
  }
  if (value === null || typeof value === "number" || typeof value === "boolean") return
  if (Array.isArray(value)) {
    for (const [index, item] of value.entries()) {
      validateReportMetadataValue(item, `${source}[${index}]`)
    }
    return
  }
  if (typeof value !== "object") {
    throw new Error(`${source} has unsupported metadata value`)
  }
  for (const [childKey, childValue] of Object.entries(value)) {
    validateReportMetadataValue(childValue, `${source}.${childKey}`, childKey)
  }
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

function nonSecretString(value) {
  return nonEmptyString(value) && !looksLikeDrillSecretValue(value)
}

function validateRuntimeSignals(value, source) {
  if (!Array.isArray(value) || !value.every(nonEmptyString)) {
    throw new Error(`${source} has invalid runtimeSignals`)
  }
  for (const signal of value) {
    if (!isKnownDrillRuntimeSignal(signal)) {
      throw new Error(`${source} has unknown runtime signal ${JSON.stringify(signal)}`)
    }
  }
}

function validateProviderList(value, source) {
  validateDrillProviders(value, source)
}

function validateProviderCountObject(value, source) {
  validateCountObject(value, source)
  validateDrillProviders(Object.keys(value), source)
}

function validateDeploymentPresetList(value, source) {
  validateDrillDeploymentPresets(value, source)
}

function validateDeploymentPresetCountObject(value, source) {
  validateCountObject(value, source)
  validateDrillDeploymentPresets(Object.keys(value), source)
}

function validateExitCriteriaEvidence(scenario, source) {
  if (!Array.isArray(scenario.exitCriteriaEvidence)) {
    throw new Error(`${source} is not an array`)
  }
  const criteria = exitCriteriaForScenario(scenario)
  if (scenario.exitCriteriaEvidence.length !== criteria.length) {
    throw new Error(`${source} length does not match exitCriteria`)
  }
  for (const [index, criterion] of scenario.exitCriteriaEvidence.entries()) {
    validateExitCriterionEvidence(criterion, `${source}[${index}]`)
    if (criterion.criterion !== criteria[index]) {
      throw new Error(`${source}[${index}] criterion does not match exitCriteria`)
    }
  }
}

function validateExitCriterionEvidence(criterion, source) {
  if (!criterion || typeof criterion !== "object" || Array.isArray(criterion)) {
    throw new Error(`${source} is not an object`)
  }
  if (!nonEmptyString(criterion.id)) {
    throw new Error(`${source} is missing id`)
  }
  if (!nonEmptyString(criterion.criterion)) {
    throw new Error(`${source} is missing criterion`)
  }
  if (!["satisfied", "failed", "skipped", "dry-run"].includes(criterion.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(criterion.status)}`)
  }
  if (criterion.reason !== null && typeof criterion.reason !== "string") {
    throw new Error(`${source} has invalid reason`)
  }
  if (criterion.status === "satisfied" && criterion.reason !== null) {
    throw new Error(`${source} satisfied criterion must not include reason`)
  }
  if (criterion.status !== "satisfied" && !nonEmptyString(criterion.reason)) {
    throw new Error(`${source} incomplete criterion is missing reason`)
  }
}

function countExitCriteriaStatuses(scenarios) {
  const counts = new Map()
  for (const scenario of scenarios) {
    for (const criterion of exitCriteriaEvidenceForScenario(scenario)) {
      counts.set(criterion.status, (counts.get(criterion.status) ?? 0) + 1)
    }
  }
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

function incompleteExitCriteriaForScenarios(scenarios) {
  const incomplete = []
  for (const scenario of scenarios) {
    for (const criterion of exitCriteriaEvidenceForScenario(scenario)) {
      if (criterion.status !== "satisfied") {
        incomplete.push({
          scenarioId: scenario.id,
          id: criterion.id,
          criterion: criterion.criterion,
          status: criterion.status,
          reason: criterion.reason ?? null,
          ...diagnosticsForIncompleteExitCriterion(scenario),
        })
      }
    }
  }
  return incomplete
}

function sameIncompleteExitCriteria(left, right) {
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false
  const leftKeys = left.map(canonicalIncompleteExitCriterionKey).sort()
  const rightKeys = right.map(canonicalIncompleteExitCriterionKey).sort()
  return leftKeys.every((key, index) => key === rightKeys[index])
}

function canonicalIncompleteExitCriterionKey(criterion) {
  return JSON.stringify({
    scenarioId: criterion.scenarioId,
    id: criterion.id,
    criterion: criterion.criterion,
    status: criterion.status,
    reason: criterion.reason ?? null,
    owner: criterion.owner ?? null,
    classification: criterion.classification ?? null,
    nextAction: criterion.nextAction ?? null,
  })
}

function diagnosticsForIncompleteExitCriterion(scenario) {
  const diagnostics = {}
  if (nonEmptyString(scenario.owner)) diagnostics.owner = scenario.owner
  if (nonEmptyString(scenario.classification)) {
    diagnostics.classification = scenario.classification
    diagnostics.owner = drillFailureOwnerForClassification(scenario.classification)
    diagnostics.nextAction = scenario.nextAction ?? drillFailureNextActionForClassification(scenario.classification, { target: "scenario" })
  }
  return diagnostics
}

function incompleteExitCriteriaCount(exitCriteria) {
  return Object.entries(exitCriteria)
    .filter(([status]) => status !== "satisfied")
    .reduce((total, [, count]) => total + count, 0)
}

function exitCriteriaEvidenceForScenario(scenario) {
  if (Array.isArray(scenario.exitCriteriaEvidence)) return scenario.exitCriteriaEvidence
  return exitCriteriaForScenario(scenario).map((criterion, index) => ({
    id: `${scenario.id}:exit-${String(index + 1).padStart(2, "0")}`,
    criterion,
    status: exitCriteriaStatusForScenario(scenario),
    reason: exitCriteriaReasonForScenario(scenario),
  }))
}

function exitCriteriaStatusForScenario(scenario) {
  if (scenario.status === "passed") return "satisfied"
  return scenario.status
}

function exitCriteriaReasonForScenario(scenario) {
  if (scenario.status === "passed") return null
  return scenario.reason ?? (scenario.status === "dry-run"
    ? "scenario command was selected but not executed"
    : "scenario did not complete")
}

function countScenarioStatuses(scenarios) {
  const counts = {
    passed: 0,
    failed: 0,
    skipped: 0,
    dryRun: 0,
  }
  for (const scenario of scenarios) {
    if (scenario.status === "passed") counts.passed += 1
    else if (scenario.status === "failed") counts.failed += 1
    else if (scenario.status === "skipped") counts.skipped += 1
    else if (scenario.status === "dry-run") counts.dryRun += 1
  }
  return counts
}

function exitCriteriaForScenario(scenario) {
  return Array.isArray(scenario.exitCriteria)
    ? scenario.exitCriteria.filter((criterion) => typeof criterion === "string" && criterion.trim().length > 0)
    : []
}

function artifactHintsForScenario(scenario) {
  return Array.isArray(scenario.artifactHints)
    ? scenario.artifactHints.filter(isValidArtifactHint).map(formatArtifactHint)
    : []
}

function isValidArtifactHint(hint) {
  if (typeof hint === "string") return hint.trim().length > 0
  return Boolean(hint)
    && typeof hint === "object"
    && !Array.isArray(hint)
    && nonEmptyString(hint.kind)
    && nonEmptyString(hint.path)
}

function artifactHintLooksSecret(hint) {
  if (typeof hint === "string") return looksLikeDrillSecretValue(hint)
  return looksLikeDrillSecretValue(hint?.kind) || looksLikeDrillSecretValue(hint?.path)
}

function formatArtifactHint(hint) {
  if (typeof hint === "string") return hint
  return `${hint.kind}:${hint.path}`
}

function runtimeSignalsForScenario(scenario) {
  return Array.isArray(scenario.runtimeSignals)
    ? [...new Set(scenario.runtimeSignals.filter(nonEmptyString))].sort()
    : []
}

function appendRuntimeSignalEvidence(evidence, signal, entry) {
  const entries = evidence.get(signal) ?? []
  entries.push(entry)
  evidence.set(signal, entries)
}

function formatRuntimeSignalEvidence(evidence) {
  return Object.fromEntries([...evidence.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([signal, entries]) => [signal, entries
      .map((entry) => ({
        ...(entry.matrix !== undefined ? { matrix: entry.matrix } : {}),
        ...(entry.source !== undefined ? { source: entry.source } : {}),
        id: entry.id,
        status: entry.status,
      }))
      .sort(compareRuntimeSignalEvidenceEntries)]))
}

function compareRuntimeSignalEvidenceEntries(left, right) {
  return String(left.matrix ?? "").localeCompare(String(right.matrix ?? ""))
    || String(left.source ?? "").localeCompare(String(right.source ?? ""))
    || left.id.localeCompare(right.id)
    || left.status.localeCompare(right.status)
}

function formatCountObject(counts) {
  return Object.entries(counts).map(([key, count]) => `${key}=${count}`).join(" ")
}

function formatRuntimeSignalScenarioRef(scenario) {
  const source = scenario.source ? ` source=${scenario.source}` : ""
  return `${scenario.matrix}/${scenario.id}(${scenario.status})${source}`
}
