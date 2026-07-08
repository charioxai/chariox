import { readFile } from "node:fs/promises"
import {
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  formatDrillAggregateNextActionSourceDetails,
} from "./drill-aggregate-actions.mjs"
import {
  drillFailureNextActionForClassification,
  drillFailureOwnerForClassification,
  validateDrillFailureClassification,
} from "./drill-failure-taxonomy.mjs"
import { drillRuntimeSignalOwnerCounts, validateDrillRuntimeSignal } from "./drill-runtime-signals.mjs"
import { findDrillJsonArtifactPaths } from "./drill-json-discovery.mjs"
import {
  validateDrillMatrixReportStatus,
  validateDrillMatrixScenarioStatus,
} from "./drill-matrix-statuses.mjs"
import {
  validateDrillDurationMatchesTimestamps,
  validateDrillTimestampOrder,
} from "./drill-time.mjs"

import {
  appendRuntimeSignalEvidence,
  artifactHintLooksSecret,
  artifactHintsForScenario,
  assertRuntimeSignalEvidenceCounts,
  countExitCriteriaStatuses,
  countScenarioStatuses,
  deploymentPresetsForReport,
  exitCriteriaForScenario,
  formatCountObject,
  formatRuntimeSignalEvidence,
  incompleteExitCriteriaForScenarios,
  isValidArtifactHint,
  nonEmptyString,
  nonSecretString,
  providersForReport,
  runtimeSignalCountsForScenarios,
  runtimeSignalScenariosForReport,
  runtimeSignalsForScenario,
  sameIncompleteExitCriteria,
  validateCountObject,
  validateExitCriteriaCountObject,
  validateExitCriteriaEvidence,
  validateMatrixReportIncompleteExitCriteria,
  validateProviderList,
  validateReportMetadata,
  validateRuntimeSignalCountObject,
  validateRuntimeSignalEvidenceObject,
  validateRuntimeSignals,
  validateScenarioProviderMetadataConsistency,
} from "./drill-matrix-report-shared.mjs"
import {
  formatDrillMatrixAggregateSummary,
  validateDrillMatrixAggregate,
} from "./drill-matrix-report-aggregate.mjs"

export {
  formatDrillMatrixAggregateSummary,
  validateDrillMatrixAggregate,
} from "./drill-matrix-report-aggregate.mjs"

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
  validateDrillMatrixReportStatus(report.status, source)
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
        sourceDetails: [nextActionSourceDetailForScenario(summary, scenario)],
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
          sourceDetails: [nextActionSourceDetailForScenario(summary, scenario)],
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
        ...(action.sourceDetails ? { sourceDetails: action.sourceDetails } : {}),
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

function nextActionSourceDetailForScenario(summary, scenario) {
  return {
    source: `${summary.matrix}/${scenario.id}`,
    matrix: summary.matrix,
    scenarioId: scenario.id,
    ...(summary.source ? { reportPath: summary.source } : {}),
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
  validateDrillMatrixScenarioStatus(scenario.status, source)
  if (typeof scenario.expectedFailure !== "boolean") {
    throw new Error(`${source} is missing expectedFailure`)
  }
  if (scenario.classification !== null && typeof scenario.classification !== "string") {
    throw new Error(`${source} has invalid classification`)
  }
  if (nonEmptyString(scenario.classification)) {
    validateDrillFailureClassification(scenario.classification, source)
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
  validateDrillFailureClassification(scenario.plannedClassification, source, {
    label: "plannedClassification",
  })
  const expectedOwner = drillFailureOwnerForClassification(scenario.plannedClassification)
  if (scenario.plannedOwner !== expectedOwner) {
    throw new Error(`${source} plannedOwner does not match plannedClassification`)
  }
  const expectedNextAction = drillFailureNextActionForClassification(scenario.plannedClassification, { target: "scenario" })
  if (scenario.plannedNextAction !== expectedNextAction) {
    throw new Error(`${source} plannedNextAction does not match plannedClassification`)
  }
}
