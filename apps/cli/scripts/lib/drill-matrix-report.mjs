import { readdir, readFile, stat } from "node:fs/promises"
import path from "node:path"
import {
  drillFailureNextActionForClassification,
  drillFailureOwnerForClassification,
} from "./drill-failure-taxonomy.mjs"

export async function readDrillMatrixReport(reportPath) {
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  validateDrillMatrixReport(report, reportPath)
  return report
}

export async function findDrillMatrixReportPaths(roots) {
  const discovered = new Set()
  for (const root of roots) {
    await collectDrillMatrixReportPaths(discovered, root)
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
  if (!Number.isFinite(report.durationMs) || report.durationMs < 0) {
    throw new Error(`${source} has invalid durationMs`)
  }
  validateReportMetadata(report.metadata, `${source}.metadata`)
  if (!Array.isArray(report.scenarios)) {
    throw new Error(`${source} is missing scenarios`)
  }
  for (const [index, scenario] of report.scenarios.entries()) {
    validateDrillMatrixScenario(scenario, `${source}.scenarios[${index}]`)
  }
}

export function summarizeDrillMatrixReport(report) {
  validateDrillMatrixReport(report)
  const counts = {
    passed: 0,
    failed: 0,
    skipped: 0,
    dryRun: 0,
  }
  const classifications = new Map()
  for (const scenario of report.scenarios) {
    if (scenario.status === "passed") counts.passed += 1
    else if (scenario.status === "failed") counts.failed += 1
    else if (scenario.status === "skipped") counts.skipped += 1
    else if (scenario.status === "dry-run") counts.dryRun += 1
    const classification = scenario.classification
    if (typeof classification === "string" && classification) {
      classifications.set(classification, (classifications.get(classification) ?? 0) + 1)
    }
  }
  return {
    matrix: report.matrix,
    status: report.status,
    durationMs: report.durationMs,
    scenarioCount: report.scenarios.length,
    counts,
    classifications: Object.fromEntries([...classifications.entries()].sort(([left], [right]) => left.localeCompare(right))),
    failedScenarios: report.scenarios.filter((scenario) => scenario.status === "failed"),
    skippedScenarios: report.scenarios.filter((scenario) => scenario.status === "skipped"),
    dryRunScenarios: report.scenarios.filter((scenario) => scenario.status === "dry-run"),
  }
}

export function summarizeDrillMatrixReports(reports) {
  const summaries = reports.map((report) => summarizeDrillMatrixReport(report))
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
    for (const [classification, count] of Object.entries(summary.classifications)) {
      classifications.set(classification, (classifications.get(classification) ?? 0) + count)
    }
    for (const scenario of summary.failedScenarios) {
      failedScenarios.push({
        matrix: summary.matrix,
        id: scenario.id,
        classification: scenario.classification ?? null,
        owner: ownerForScenario(scenario),
        reason: scenario.reason ?? null,
        nextAction: nextActionForScenario(scenario),
      })
      const owner = ownerForScenario(scenario)
      owners.set(owner, (owners.get(owner) ?? 0) + 1)
    }
    for (const scenario of summary.skippedScenarios) {
      skippedScenarios.push({ matrix: summary.matrix, id: scenario.id, reason: scenario.reason ?? null })
      incompleteScenarios.push({ matrix: summary.matrix, id: scenario.id, status: "skipped", reason: scenario.reason ?? null })
    }
    for (const scenario of summary.dryRunScenarios) {
      incompleteScenarios.push({ matrix: summary.matrix, id: scenario.id, status: "dry-run", reason: scenario.reason ?? null })
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
    owners: Object.fromEntries([...owners.entries()].sort(([left], [right]) => left.localeCompare(right))),
    reports: summaries.map((summary) => ({
      matrix: summary.matrix,
      status: summary.status,
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

  if (aggregate.failedScenarios.length > 0) {
    lines.push("failed scenarios:")
    for (const scenario of aggregate.failedScenarios) {
      const classification = scenario.classification ? ` classification=${scenario.classification}` : ""
      const reason = scenario.reason ? ` reason=${scenario.reason}` : ""
      lines.push(`- ${scenario.matrix}/${scenario.id}${classification} owner=${scenario.owner}${reason}`)
      lines.push(`  next: ${scenario.nextAction}`)
    }
  }

  if (aggregate.incompleteScenarios.length > 0) {
    lines.push("incomplete scenarios:")
    for (const scenario of aggregate.incompleteScenarios) {
      const reason = scenario.reason ? ` reason=${scenario.reason}` : ""
      lines.push(`- ${scenario.matrix}/${scenario.id} status=${scenario.status}${reason}`)
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

async function collectDrillMatrixReportPaths(discovered, entryPath) {
  const entry = await stat(entryPath).catch(() => null)
  if (!entry) return
  if (entry.isDirectory()) {
    const children = await readdir(entryPath, { withFileTypes: true }).catch(() => [])
    for (const child of children) {
      await collectDrillMatrixReportPaths(discovered, path.join(entryPath, child.name))
    }
    return
  }
  if (!entry.isFile() || !entryPath.endsWith(".json")) return
  try {
    const parsed = JSON.parse(await readFile(entryPath, "utf8"))
    if (parsed?.schema === "arroba.drill.matrix.v1") discovered.add(entryPath)
  } catch {
    // Ignore unrelated JSON files in broad artifact roots.
  }
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
  if (!aggregate.owners || typeof aggregate.owners !== "object" || Array.isArray(aggregate.owners)) {
    throw new Error("aggregate is missing owners")
  }
  if (!Array.isArray(aggregate.incompleteScenarios)) {
    throw new Error("aggregate is missing incompleteScenarios")
  }
}

function validateReportMetadata(value, source) {
  if (!value || typeof value !== "object") {
    throw new Error(`${source} must be an object`)
  }
  validateReportMetadataValue(value, source)
}

function validateReportMetadataValue(value, source, key = "") {
  if (isSensitiveReportKey(key)) {
    throw new Error(`${source} includes sensitive metadata key ${JSON.stringify(key)}`)
  }
  if (typeof value === "string") {
    if (looksLikeSecretValue(value)) {
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

function isSensitiveReportKey(key) {
  return /token|secret|password|credential|cookie|authorization|api[-_]?key/i.test(key)
}

function looksLikeSecretValue(value) {
  return /\bBearer\s+[A-Za-z0-9._~+/=-]{12,}/i.test(value)
    || /\bsk-[A-Za-z0-9_-]{16,}\b/.test(value)
    || /\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9_]{16,}\b/.test(value)
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
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
