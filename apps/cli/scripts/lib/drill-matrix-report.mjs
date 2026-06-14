import { opendir, readFile, stat } from "node:fs/promises"
import path from "node:path"
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
import {
  isSensitiveDrillKey,
  looksLikeDrillSecretValue,
} from "./drill-secrets.mjs"
import {
  validateDrillDurationMatchesTimestamps,
  validateDrillTimestampOrder,
} from "./drill-time.mjs"

export async function readDrillMatrixReport(reportPath) {
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  validateDrillMatrixReport(report, reportPath)
  return report
}

export async function findDrillMatrixReportPaths(roots, { maxDepth = 8 } = {}) {
  const discovered = new Set()
  for (const root of roots) {
    await collectDrillMatrixReportPaths(discovered, root, { depth: 0, maxDepth })
  }
  return [...discovered].sort()
}

export function validateDrillMatrixReport(report, source = "report") {
  if (!report || typeof report !== "object") {
    throw new Error(`${source} is not an object`)
  }
  if (report.schema !== "arroba.drill.matrix.v1") {
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
  if (!Number.isFinite(report.durationMs) || report.durationMs < 0) {
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
}

export function summarizeDrillMatrixReport(report, { source = null } = {}) {
  validateDrillMatrixReport(report)
  const counts = countScenarioStatuses(report.scenarios)
  const classifications = new Map()
  for (const scenario of report.scenarios) {
    const classification = scenario.classification
    if (typeof classification === "string" && classification) {
      classifications.set(classification, (classifications.get(classification) ?? 0) + 1)
    }
  }
  return {
    matrix: report.matrix,
    source,
    status: report.status,
    durationMs: report.durationMs,
    deploymentPresets: deploymentPresetsForReport(report),
    scenarioCount: report.scenarios.length,
    counts,
    classifications: Object.fromEntries([...classifications.entries()].sort(([left], [right]) => left.localeCompare(right))),
    failedScenarios: report.scenarios.filter((scenario) => scenario.status === "failed"),
    skippedScenarios: report.scenarios.filter((scenario) => scenario.status === "skipped"),
    dryRunScenarios: report.scenarios.filter((scenario) => scenario.status === "dry-run"),
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
  const owners = new Map()
  const nextActions = new Map()
  const deploymentPresets = new Map()
  const failedScenarios = []
  const skippedScenarios = []
  const incompleteScenarios = []
  for (const summary of summaries) {
    totals.scenarios += summary.scenarioCount
    totals.passed += summary.counts.passed
    totals.failed += summary.counts.failed
    totals.skipped += summary.counts.skipped
    totals.dryRun += summary.counts.dryRun
    totals.durationMs += Number.isFinite(summary.durationMs) ? summary.durationMs : 0
    for (const preset of summary.deploymentPresets) {
      deploymentPresets.set(preset, (deploymentPresets.get(preset) ?? 0) + 1)
    }
    for (const [classification, count] of Object.entries(summary.classifications)) {
      classifications.set(classification, (classifications.get(classification) ?? 0) + count)
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
      incompleteScenarios.push({ matrix: summary.matrix, source: summary.source, id: scenario.id, status: "dry-run", reason: scenario.reason ?? null })
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
    deploymentPresets: Object.fromEntries([...deploymentPresets.entries()].sort(([left], [right]) => left.localeCompare(right))),
    owners: Object.fromEntries([...owners.entries()].sort(([left], [right]) => left.localeCompare(right))),
    nextActions: formatDrillAggregateNextActionCounts(nextActions),
    reports: summaries.map((summary) => ({
      matrix: summary.matrix,
      source: summary.source,
      status: summary.status,
      deploymentPresets: summary.deploymentPresets,
      scenarioCount: summary.scenarioCount,
      counts: summary.counts,
      durationMs: summary.durationMs,
    })),
    failedScenarios,
    skippedScenarios,
    incompleteScenarios,
  }
}

export function formatDrillMatrixReportSummary(report, { source = null } = {}) {
  const summary = summarizeDrillMatrixReport(report)
  const lines = [
    `matrix report: ${summary.matrix}${source ? ` (${source})` : ""}`,
    `status=${summary.status} scenarios=${summary.scenarioCount} passed=${summary.counts.passed} failed=${summary.counts.failed} skipped=${summary.counts.skipped} dry_run=${summary.counts.dryRun} duration_ms=${summary.durationMs ?? "-"}`,
  ]

  const classifications = Object.entries(summary.classifications)
  if (classifications.length > 0) {
    lines.push(`classifications: ${classifications.map(([kind, count]) => `${kind}=${count}`).join(" ")}`)
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

  if (summary.status === "dry-run") {
    appendDryRunCriteria(lines, report.scenarios)
  }

  if (summary.failedScenarios.length === 0) {
    lines.push(summary.status === "dry-run" ? "next: run without --dry-run to execute selected scenarios" : "next: no failed matrix scenarios")
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
  const deploymentPresets = Object.entries(aggregate.deploymentPresets ?? {})
  if (deploymentPresets.length > 0) {
    lines.push(`deployment_presets: ${deploymentPresets.map(([preset, count]) => `${preset}=${count}`).join(" ")}`)
  }
  if (Array.isArray(aggregate.nextActions) && aggregate.nextActions.length > 0) {
    lines.push("next actions:")
    for (const action of aggregate.nextActions) {
      lines.push(`- owner=${action.owner} classification=${action.classification} count=${action.count}: ${action.nextAction}`)
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
      lines.push(`- ${scenario.matrix}/${scenario.id} status=${scenario.status}${reason}${source}`)
    }
  }

  if (aggregate.failedScenarios.length === 0 && aggregate.incompleteScenarios.length === 0) {
    lines.push("next: all selected matrix scenarios completed without failures")
  } else if (aggregate.failedScenarios.length === 0) {
    lines.push("next: run incomplete scenarios before treating this matrix set as complete")
  }

  return lines.join("\n")
}

export function drillMatrixReportExitCode(reports) {
  return reports.some((report) => report.status === "failed") ? 1 : 0
}

export function drillMatrixReportCompletionExitCode(reports) {
  const aggregate = summarizeDrillMatrixReports(reports)
  if (aggregate.status === "failed") return 1
  return aggregate.incompleteScenarios.length > 0 ? 2 : 0
}

async function collectDrillMatrixReportPaths(discovered, entryPath, { depth, maxDepth }) {
  const entry = await stat(entryPath).catch(() => null)
  if (!entry) return
  if (entry.isFile()) {
    await maybeCollectDrillMatrixReportPath(discovered, entryPath)
    return
  }
  if (!entry.isDirectory() || depth > maxDepth) return
  let dir = null
  try {
    dir = await opendir(entryPath)
    for await (const child of dir) {
      const childPath = path.join(entryPath, child.name)
      if (child.isFile()) {
        await maybeCollectDrillMatrixReportPath(discovered, childPath)
        continue
      }
      if (!child.isDirectory() || shouldPruneMatrixReportDirectory(child.name)) continue
      await collectDrillMatrixReportPaths(discovered, childPath, { depth: depth + 1, maxDepth })
    }
  } catch {
    // Ignore unreadable directories in broad artifact roots.
  }
}

async function maybeCollectDrillMatrixReportPath(discovered, entryPath) {
  if (!entryPath.endsWith(".json")) return
  try {
    const parsed = JSON.parse(await readFile(entryPath, "utf8"))
    if (parsed?.schema === "arroba.drill.matrix.v1") discovered.add(entryPath)
  } catch {
    // Ignore unrelated JSON files in broad artifact roots.
  }
}

function shouldPruneMatrixReportDirectory(name) {
  return name === ".git"
    || name === "node_modules"
    || name === ".pnpm-store"
    || name === "debug"
    || name === "release"
}

function nextActionForScenario(scenario) {
  return drillFailureNextActionForClassification(scenario.classification, { target: "scenario" })
}

function ownerForScenario(scenario) {
  return drillFailureOwnerForClassification(scenario.classification)
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
      throw new Error(`${source} owner requires classification`)
    }
    const expectedOwner = drillFailureOwnerForClassification(scenario.classification)
    if (scenario.owner !== expectedOwner) {
      throw new Error(`${source} owner does not match classification`)
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
  if (!Number.isFinite(scenario.durationMs) || scenario.durationMs < 0) {
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
    || !scenario.artifactHints.every((value) => typeof value === "string")
  )) {
    throw new Error(`${source} has invalid artifactHints`)
  }
  if (scenario.artifactHints?.some((value) => looksLikeDrillSecretValue(value))) {
    throw new Error(`${source} includes secret-looking artifactHints`)
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

function validateDrillMatrixAggregate(aggregate) {
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
    if (!Number.isFinite(aggregate.totals[key]) || aggregate.totals[key] < 0) {
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
  validateCountObject(aggregate.deploymentPresets, "aggregate.deploymentPresets")
  if (aggregate.nextActions !== undefined && !Array.isArray(aggregate.nextActions)) {
    throw new Error("aggregate has invalid nextActions")
  }
  for (const [index, action] of (aggregate.nextActions ?? []).entries()) {
    validateDrillAggregateNextAction(action, `aggregate.nextActions[${index}]`)
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
  if (!Array.isArray(aggregate.reports)) {
    throw new Error("aggregate is missing reports")
  }
  for (const [index, report] of aggregate.reports.entries()) {
    validateMatrixAggregateReport(report, `aggregate.reports[${index}]`)
  }
  validateDrillMatrixAggregateConsistency(aggregate)
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
  const expectedStatus = aggregate.totals.failed > 0
    ? "failed"
    : aggregate.totals.reports > 0 && aggregate.totals.dryRun === aggregate.totals.scenarios
      ? "dry-run"
      : "passed"
  if (aggregate.status !== expectedStatus) {
    throw new Error("aggregate status does not match totals")
  }
  assertObjectCountsMatchEntries("aggregate owners", aggregate.owners, aggregate.failedScenarios, "owner")
  assertDeploymentPresetCountsMatchReports(aggregate)
  assertNextActionCountsMatchScenarios(aggregate)
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
  if (!Array.isArray(report.deploymentPresets) || !report.deploymentPresets.every(nonEmptyString)) {
    throw new Error(`${source} has invalid deploymentPresets`)
  }
  if (!Number.isFinite(report.scenarioCount) || report.scenarioCount < 0) {
    throw new Error(`${source} has invalid scenarioCount`)
  }
  validateMatrixAggregateReportCounts(report.counts, `${source}.counts`)
  if (!Number.isFinite(report.durationMs) || report.durationMs < 0) {
    throw new Error(`${source} has invalid durationMs`)
  }
  const scenarioCount = report.counts.passed + report.counts.failed + report.counts.skipped + report.counts.dryRun
  if (report.scenarioCount !== scenarioCount) {
    throw new Error(`${source} scenarioCount does not match counts`)
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
    || !scenario.artifactHints.every((value) => typeof value === "string")
  )) {
    throw new Error(`${source} has invalid artifactHints`)
  }
  if (scenario.artifactHints?.some((value) => looksLikeDrillSecretValue(value))) {
    throw new Error(`${source} includes secret-looking artifactHints`)
  }
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
}

function validateMatrixAggregateReportCounts(counts, source) {
  if (!counts || typeof counts !== "object" || Array.isArray(counts)) {
    throw new Error(`${source} is missing`)
  }
  for (const key of ["passed", "failed", "skipped", "dryRun"]) {
    if (!Number.isFinite(counts[key]) || counts[key] < 0) {
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

function deploymentPresetsForReport(report) {
  const value = report.metadata?.deploymentPresets
  if (!nonEmptyString(value)) return []
  return [...new Set(value.split(",").map((preset) => preset.trim()).filter(Boolean))].sort()
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
    ? scenario.artifactHints.filter((hint) => typeof hint === "string" && hint.trim().length > 0)
    : []
}
