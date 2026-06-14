import { opendir, readFile, stat } from "node:fs/promises"
import path from "node:path"

import {
  DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
  drillValidationGateAggregateExitCode,
  formatDrillValidationGateAggregateSummary,
  summarizeValidationGateReportAggregate,
  validateDrillValidationGateAggregate,
} from "./drill-validation-gate-aggregate.mjs"
import { artifactValidationGateCheck } from "./drill-validation-gate-artifact-check.mjs"
import { configurationValidationGateCheck } from "./drill-validation-gate-configuration-check.mjs"
import { failureValidationGateCheck } from "./drill-validation-gate-failure-check.mjs"
import { matrixValidationGateCheck } from "./drill-validation-gate-matrix-check.mjs"
import { validationGateNextActions } from "./drill-validation-gate-next-actions.mjs"
import { platformValidationGateCheck } from "./drill-validation-gate-platform-check.mjs"
import {
  DRILL_VALIDATION_GATE_PRESETS,
  describeDrillValidationGatePresets,
  expandValidationGatePresetRequirements,
  normalizeRequiredDeploymentPresets,
  normalizeRequiredFailureClassifications,
  normalizeRequiredMatrices,
  normalizeRequiredMatrixClassifications,
  normalizeRequiredPlatformCoverageAreas,
  normalizeRequiredPresets,
  normalizeRequiredProviders,
  normalizeRequiredScenarios,
} from "./drill-validation-gate-presets.mjs"
import {
  DRILL_VALIDATION_GATE_SCHEMA,
  validateDrillValidationGateReport,
} from "./drill-validation-gate-report.mjs"

export {
  DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
  DRILL_VALIDATION_GATE_PRESETS,
  DRILL_VALIDATION_GATE_SCHEMA,
  describeDrillValidationGatePresets,
  drillValidationGateAggregateExitCode,
  formatDrillValidationGateAggregateSummary,
  validateDrillValidationGateReport,
  validateDrillValidationGateAggregate,
}

export async function readDrillValidationGateReport(reportPath) {
  const report = JSON.parse(await readFile(reportPath, "utf8"))
  validateDrillValidationGateReport(report, reportPath)
  return report
}

export async function readDrillValidationGateAggregate(aggregatePath) {
  const aggregate = JSON.parse(await readFile(aggregatePath, "utf8"))
  validateDrillValidationGateAggregate(aggregate, aggregatePath)
  return aggregate
}

export async function findDrillValidationGateReportPaths(roots, { maxDepth = 8 } = {}) {
  const discovered = new Set()
  for (const root of roots) {
    await collectDrillValidationGateReportPaths(discovered, root, { depth: 0, maxDepth })
  }
  return [...discovered].sort()
}

export async function findDrillValidationGateAggregatePaths(roots, { maxDepth = 8 } = {}) {
  const discovered = new Set()
  for (const root of roots) {
    await collectDrillValidationGateAggregatePaths(discovered, root, { depth: 0, maxDepth })
  }
  return [...discovered].sort()
}

export function summarizeDrillValidationGateReports(reports, options = {}) {
  const { sources = [], requiredPresets = [] } = options
  const normalizedRequiredPresets = normalizeRequiredPresets(requiredPresets)
  const normalizedAggregateRequirements = normalizeValidationGateAggregateRequirements(options)
  return summarizeValidationGateReportAggregate(reports, {
    sources,
    normalizedRequiredPresets,
    normalizedAggregateRequirements,
    validateReport: validateDrillValidationGateReport,
  })
}

export async function runDrillValidationGate({
  artifactIndexes = [],
  artifactRoots = [],
  failureInputs = [],
  failureRoots = [],
  matrixReports = [],
  matrixRoots = [],
  maxDepth = 8,
  platformBundleDir = null,
  presets = [],
  requireComplete = false,
  requiredPlatformCoverageAreas = [],
  requiredFailureClassifications = [],
  requiredMatrices = [],
  requiredMatrixClassifications = [],
  requiredDeploymentPresets = [],
  requiredProviders = [],
  requiredScenarios = [],
} = {}) {
  const normalizedPresets = normalizeRequiredPresets(presets)
  const expandedRequirements = expandValidationGatePresetRequirements({
    presets: normalizedPresets,
    requiredPlatformCoverageAreas,
    requiredFailureClassifications,
    requiredMatrices,
    requiredMatrixClassifications,
    requiredDeploymentPresets,
    requiredProviders,
    requiredScenarios,
  })
  const normalizedRequiredPlatformCoverageAreas = normalizeRequiredPlatformCoverageAreas(expandedRequirements.requiredPlatformCoverageAreas)
  const normalizedRequiredFailureClassifications = normalizeRequiredFailureClassifications(expandedRequirements.requiredFailureClassifications)
  const normalizedRequiredMatrices = normalizeRequiredMatrices(expandedRequirements.requiredMatrices)
  const normalizedRequiredMatrixClassifications = normalizeRequiredMatrixClassifications(expandedRequirements.requiredMatrixClassifications)
  const normalizedRequiredDeploymentPresets = normalizeRequiredDeploymentPresets(expandedRequirements.requiredDeploymentPresets)
  const normalizedRequiredProviders = normalizeRequiredProviders(expandedRequirements.requiredProviders)
  const normalizedRequiredScenarios = normalizeRequiredScenarios(expandedRequirements.requiredScenarios)
  const checks = {
    configuration: configurationValidationGateCheck({
      artifactIndexes,
      artifactRoots,
      failureInputs,
      failureRoots,
      matrixReports,
      matrixRoots,
      platformBundleDir,
      requiredPlatformCoverageAreas: normalizedRequiredPlatformCoverageAreas,
      requiredFailureClassifications: normalizedRequiredFailureClassifications,
      requiredMatrices: normalizedRequiredMatrices,
      requiredMatrixClassifications: normalizedRequiredMatrixClassifications,
      requiredDeploymentPresets: normalizedRequiredDeploymentPresets,
      requiredProviders: normalizedRequiredProviders,
      requiredScenarios: normalizedRequiredScenarios,
    }),
    platformBundle: await platformValidationGateCheck(platformBundleDir, {
      requiredCoverageAreas: normalizedRequiredPlatformCoverageAreas,
      requiredFailureClassifications: normalizedRequiredFailureClassifications,
    }),
    artifacts: await artifactValidationGateCheck({ artifactIndexes, artifactRoots }, { maxDepth }),
    matrices: await matrixValidationGateCheck({
      matrixReports,
      matrixRoots,
    }, {
      maxDepth,
      requireComplete,
      requiredMatrices: normalizedRequiredMatrices,
      requiredMatrixClassifications: normalizedRequiredMatrixClassifications,
      requiredDeploymentPresets: normalizedRequiredDeploymentPresets,
      requiredProviders: normalizedRequiredProviders,
      requiredScenarios: normalizedRequiredScenarios,
    }),
    failures: await failureValidationGateCheck({ failureInputs, failureRoots }, { maxDepth }),
  }
  const nextActions = validationGateNextActions(checks)
  const report = {
    schema: DRILL_VALIDATION_GATE_SCHEMA,
    status: Object.values(checks).some((check) => check.status === "failed") ? "failed" : "passed",
    presets: normalizedPresets,
    checks,
    nextActions,
  }
  validateDrillValidationGateReport(report)
  return report
}

export function drillValidationGateExitCode(report) {
  validateDrillValidationGateReport(report)
  return report.status === "failed" ? 1 : 0
}

export function formatDrillValidationGateSummary(report) {
  validateDrillValidationGateReport(report)
  const lines = [
    "drill validation gate:",
    `status=${report.status}`,
  ]
  if ((report.presets ?? []).length > 0) {
    lines.push(`presets=${report.presets.join(",")}`)
  }
  const configuration = report.checks.configuration
  lines.push(`configuration=${configuration.status}${configuration.error ? ` error=${configuration.error}` : ""}`)

  const platform = report.checks.platformBundle
  lines.push(`platform_bundle=${platform.status}${platform.dir ? ` dir=${platform.dir}` : ""}${platform.error ? ` error=${platform.error}` : ""}`)
  const requiredPlatformCoverageAreas = platform.requiredCoverageAreas ?? []
  const missingPlatformCoverageAreas = platform.missingCoverageAreas ?? []
  if (requiredPlatformCoverageAreas.length > 0) {
    lines.push(`platform_required_coverage_areas=${requiredPlatformCoverageAreas.join(",")} missing=${missingPlatformCoverageAreas.join(",") || "none"}`)
  }
  const requiredFailureClassifications = platform.requiredFailureClassifications ?? []
  const missingFailureClassifications = platform.missingFailureClassifications ?? []
  if (requiredFailureClassifications.length > 0) {
    lines.push(`platform_required_failure_classifications=${requiredFailureClassifications.join(",")} missing=${missingFailureClassifications.join(",") || "none"}`)
  }
  if (platform.validationSuite) {
    lines.push(`platform_validation_suite_tests=${platform.validationSuite.testCount} coverage=${platform.validationSuite.coverageAreas.map((area) => `${area.id}:${area.testCount}`).join(",")}`)
  }
  if (platform.failureTaxonomy) {
    lines.push(`platform_failure_taxonomy=drill:${platform.failureTaxonomy.drill.length} scenario:${platform.failureTaxonomy.scenario.length}`)
  }

  const artifacts = report.checks.artifacts
  lines.push(`artifacts=${artifacts.status} roots=${artifacts.roots.length} inputs=${artifacts.inputs.length} indexes=${artifacts.indexPaths.length}`)
  if (artifacts.error) lines.push(`artifact_error=${artifacts.error}`)
  if (artifacts.aggregate) {
    lines.push(`artifact_total=${artifacts.aggregate.totals.artifacts} size_bytes=${artifacts.aggregate.totals.sizeBytes}`)
  }

  const matrices = report.checks.matrices
  lines.push(`matrices=${matrices.status} roots=${matrices.roots.length} inputs=${matrices.inputs.length} reports=${matrices.reportPaths.length} require_complete=${matrices.requireComplete}`)
  const requiredMatrices = matrices.requiredMatrices ?? []
  const missingMatrices = matrices.missingMatrices ?? []
  if (requiredMatrices.length > 0) {
    lines.push(`matrix_required_names=${requiredMatrices.join(",")} missing=${missingMatrices.join(",") || "none"}`)
  }
  const requiredMatrixClassifications = matrices.requiredMatrixClassifications ?? []
  const missingMatrixClassifications = matrices.missingMatrixClassifications ?? []
  if (requiredMatrixClassifications.length > 0) {
    lines.push(`matrix_required_classifications=${requiredMatrixClassifications.join(",")} missing=${missingMatrixClassifications.join(",") || "none"}`)
  }
  const requiredDeploymentPresets = matrices.requiredDeploymentPresets ?? []
  const missingDeploymentPresets = matrices.missingDeploymentPresets ?? []
  if (requiredDeploymentPresets.length > 0) {
    lines.push(`matrix_required_deployment_presets=${requiredDeploymentPresets.join(",")} missing=${missingDeploymentPresets.join(",") || "none"}`)
  }
  const requiredProviders = matrices.requiredProviders ?? []
  const missingProviders = matrices.missingProviders ?? []
  if (requiredProviders.length > 0) {
    lines.push(`matrix_required_providers=${requiredProviders.join(",")} missing=${missingProviders.join(",") || "none"}`)
  }
  const requiredScenarios = matrices.requiredScenarios ?? []
  const missingScenarios = matrices.missingScenarios ?? []
  if (requiredScenarios.length > 0) {
    lines.push(`matrix_required_scenarios=${requiredScenarios.join(",")} missing=${missingScenarios.join(",") || "none"}`)
  }
  if (matrices.error) lines.push(`matrix_error=${matrices.error}`)
  if (matrices.aggregate) {
    lines.push(`matrix_status=${matrices.aggregate.status} failed=${matrices.aggregate.totals.failed} skipped=${matrices.aggregate.totals.skipped} dry_run=${matrices.aggregate.totals.dryRun}`)
  }

  const failures = report.checks.failures
  lines.push(`failures=${failures.status} roots=${failures.roots.length} inputs=${failures.inputs.length} manifests=${failures.manifestPaths.length}`)
  if (failures.error) lines.push(`failure_error=${failures.error}`)
  if (failures.aggregate) {
    lines.push(`failure_total=${failures.aggregate.total}`)
  }

  if (report.nextActions.length > 0) {
    lines.push("next actions:")
    for (const action of report.nextActions) {
      lines.push(`- owner=${action.owner} classification=${action.classification} count=${action.count}: ${action.nextAction}`)
    }
  }

  lines.push(report.status === "passed"
    ? "next: validation artifacts passed configured gates"
    : "next: inspect failed gate checks and rerun the relevant drills")
  return lines.join("\n")
}

function normalizeValidationGateAggregateRequirements(options) {
  return {
    requiredPlatformCoverageAreas: normalizeRequiredPlatformCoverageAreas(options.requiredPlatformCoverageAreas ?? []),
    requiredFailureClassifications: normalizeRequiredFailureClassifications(options.requiredFailureClassifications ?? []),
    requiredMatrices: normalizeRequiredMatrices(options.requiredMatrices ?? []),
    requiredMatrixClassifications: normalizeRequiredMatrixClassifications(options.requiredMatrixClassifications ?? []),
    requiredDeploymentPresets: normalizeRequiredDeploymentPresets(options.requiredDeploymentPresets ?? []),
    requiredProviders: normalizeRequiredProviders(options.requiredProviders ?? []),
    requiredScenarios: normalizeRequiredScenarios(options.requiredScenarios ?? []),
  }
}

async function collectDrillValidationGateReportPaths(discovered, entryPath, { depth, maxDepth }) {
  const entry = await stat(entryPath).catch(() => null)
  if (!entry) return
  if (entry.isFile()) {
    await maybeCollectDrillValidationGatePath(discovered, entryPath, DRILL_VALIDATION_GATE_SCHEMA)
    return
  }
  if (!entry.isDirectory() || depth > maxDepth) return
  let dir = null
  try {
    dir = await opendir(entryPath)
    for await (const child of dir) {
      const childPath = path.join(entryPath, child.name)
      if (child.isFile()) {
        await maybeCollectDrillValidationGatePath(discovered, childPath, DRILL_VALIDATION_GATE_SCHEMA)
        continue
      }
      if (!child.isDirectory() || shouldPruneValidationGateDirectory(child.name)) continue
      await collectDrillValidationGateReportPaths(discovered, childPath, { depth: depth + 1, maxDepth })
    }
  } catch {
    // Ignore unreadable directories in broad artifact roots.
  }
}

async function collectDrillValidationGateAggregatePaths(discovered, entryPath, { depth, maxDepth }) {
  const entry = await stat(entryPath).catch(() => null)
  if (!entry) return
  if (entry.isFile()) {
    await maybeCollectDrillValidationGatePath(discovered, entryPath, DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA)
    return
  }
  if (!entry.isDirectory() || depth > maxDepth) return
  let dir = null
  try {
    dir = await opendir(entryPath)
    for await (const child of dir) {
      const childPath = path.join(entryPath, child.name)
      if (child.isFile()) {
        await maybeCollectDrillValidationGatePath(discovered, childPath, DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA)
        continue
      }
      if (!child.isDirectory() || shouldPruneValidationGateDirectory(child.name)) continue
      await collectDrillValidationGateAggregatePaths(discovered, childPath, { depth: depth + 1, maxDepth })
    }
  } catch {
    // Ignore unreadable directories in broad artifact roots.
  }
}

async function maybeCollectDrillValidationGatePath(discovered, entryPath, schema) {
  if (!entryPath.endsWith(".json")) return
  try {
    const parsed = JSON.parse(await readFile(entryPath, "utf8"))
    if (parsed?.schema === schema) discovered.add(entryPath)
  } catch {
    // Ignore unrelated JSON files in broad artifact roots.
  }
}

function shouldPruneValidationGateDirectory(name) {
  return name === ".git"
    || name === "node_modules"
    || name === ".pnpm-store"
    || name === "debug"
    || name === "release"
}
