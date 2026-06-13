import { readFile } from "node:fs/promises"

export async function readDrillMatrixReport(reportPath) {
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  validateDrillMatrixReport(report, reportPath)
  return report
}

export function validateDrillMatrixReport(report, source = "report") {
  if (!report || typeof report !== "object") {
    throw new Error(`${source} is not an object`)
  }
  if (report.schema !== "arroba.drill.matrix.v1") {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(report.schema)}`)
  }
  if (!Array.isArray(report.scenarios)) {
    throw new Error(`${source} is missing scenarios`)
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
      const reason = scenario.reason ? ` reason=${scenario.reason}` : ""
      lines.push(`- ${scenario.id}${classification}${reason}`)
      lines.push(`  next: ${nextActionForScenario(scenario)}`)
    }
  }

  if (summary.skippedScenarios.length > 0) {
    lines.push(`skipped scenarios: ${summary.skippedScenarios.map((scenario) => scenario.id).join(", ")}`)
  }

  if (summary.failedScenarios.length === 0) {
    lines.push(summary.status === "dry-run" ? "next: run without --dry-run to execute selected scenarios" : "next: no failed matrix scenarios")
  }

  return lines.join("\n")
}

export function drillMatrixReportExitCode(reports) {
  return reports.some((report) => report.status === "failed") ? 1 : 0
}

function nextActionForScenario(scenario) {
  if (scenario.classification === "provider-auth") {
    return "refresh provider login for the profile used by this drill, then rerun the scenario"
  }
  if (scenario.classification === "provider-account") {
    return "check provider quota or billing for the account used by this drill, then rerun the scenario"
  }
  if (scenario.classification === "expected-failure") {
    return "inspect the expected-failure assertion; the scenario failed differently than planned"
  }
  return "inspect preserved drill artifacts and rerun the command recorded in this report"
}
