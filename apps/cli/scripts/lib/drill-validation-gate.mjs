import { opendir, readFile, stat } from "node:fs/promises"
import path from "node:path"

import {
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  validateDrillAggregateNextAction,
} from "./drill-aggregate-actions.mjs"
import {
  findDrillArtifactIndexPaths,
  summarizeDrillArtifactIndexes,
  verifyDrillArtifactIndex,
} from "./drill-artifacts.mjs"
import {
  findDrillFailureManifestPaths,
  readDrillFailureManifest,
  resolveFailureManifestPath,
  summarizeDrillFailureManifests,
} from "./drill-failure-manifest.mjs"
import { isKnownDrillFailureClassification } from "./drill-failure-taxonomy.mjs"
import { DRILL_DEPLOYMENT_PRESETS } from "./drill-environment-presets.mjs"
import {
  drillMatrixReportCompletionExitCode,
  drillMatrixReportExitCode,
  findDrillMatrixReportPaths,
  readDrillMatrixReport,
  summarizeDrillMatrixReports,
} from "./drill-matrix-report.mjs"
import { verifyDrillPlatformBundle } from "./drill-platform-bundle.mjs"

export const DRILL_VALIDATION_GATE_SCHEMA = "arroba.drill.validation_gate.v1"
export const DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA = "arroba.drill.validation_gate.aggregate.v1"
export const DRILL_VALIDATION_GATE_PRESETS = Object.freeze({
  "workspace-live-sync": Object.freeze({
    description: "Workspace Live Sync local/remote matrix evidence and distributed sync diagnostics.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredFailureClassifications: Object.freeze(["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"]),
    requiredMatrices: Object.freeze(["workspace-live-sync-matrix"]),
    requiredMatrixClassifications: Object.freeze(["kernel-authority", "relay-target-freshness", "workspace-live-sync-conflict"]),
  }),
  "remote-home-extension": Object.freeze({
    description: "Home-owned extension execution evidence for remote agents and collab authority checks.",
    requiredPlatformCoverageAreas: Object.freeze(["failure-diagnostics", "matrix-validation", "runtime-fixtures"]),
    requiredFailureClassifications: Object.freeze(["kernel-authority", "remote-extension-sync", "worker-execution"]),
    requiredMatrices: Object.freeze(["remote-home-extension-matrix"]),
    requiredMatrixClassifications: Object.freeze(["kernel-authority", "remote-extension-sync", "worker-execution"]),
  }),
})

export function describeDrillValidationGatePresets({ names = null } = {}) {
  const presetNames = names == null
    ? Object.keys(DRILL_VALIDATION_GATE_PRESETS).sort()
    : normalizeRequiredPresets(Array.isArray(names) ? names : [names])
  return presetNames.map((name) => {
    const preset = DRILL_VALIDATION_GATE_PRESETS[name]
    return {
      name,
      description: preset.description,
      requiredPlatformCoverageAreas: [...(preset.requiredPlatformCoverageAreas ?? [])],
      requiredFailureClassifications: [...(preset.requiredFailureClassifications ?? [])],
      requiredMatrices: [...(preset.requiredMatrices ?? [])],
      requiredMatrixClassifications: [...(preset.requiredMatrixClassifications ?? [])],
      requiredDeploymentPresets: [...(preset.requiredDeploymentPresets ?? [])],
      requiredProviders: [...(preset.requiredProviders ?? [])],
      requiredScenarios: [...(preset.requiredScenarios ?? [])],
    }
  })
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

export function summarizeDrillValidationGateReports(reports, { sources = [] } = {}) {
  const totals = {
    reports: reports.length,
    passed: 0,
    failed: 0,
  }
  const nextActions = new Map()
  const coverage = {
    presets: new Map(),
    requiredPlatformCoverageAreas: new Map(),
    missingPlatformCoverageAreas: new Map(),
    requiredFailureClassifications: new Map(),
    missingFailureClassifications: new Map(),
    requiredMatrices: new Map(),
    missingMatrices: new Map(),
    requiredMatrixClassifications: new Map(),
    missingMatrixClassifications: new Map(),
    requiredDeploymentPresets: new Map(),
    missingDeploymentPresets: new Map(),
    requiredProviders: new Map(),
    missingProviders: new Map(),
    requiredScenarios: new Map(),
    missingScenarios: new Map(),
  }
  const summaries = reports.map((report, index) => {
    validateDrillValidationGateReport(report, sources[index] ?? "validation gate report")
    totals[report.status] += 1
    for (const action of report.nextActions) {
      countDrillAggregateNextAction(nextActions, action)
    }
    countStringValues(coverage.presets, report.presets ?? [])
    const platformCoverage = validationGateReportPlatformCoverage(report)
    countStringValues(coverage.requiredPlatformCoverageAreas, platformCoverage.requiredCoverageAreas)
    countStringValues(coverage.missingPlatformCoverageAreas, platformCoverage.missingCoverageAreas)
    countStringValues(coverage.requiredFailureClassifications, platformCoverage.requiredFailureClassifications)
    countStringValues(coverage.missingFailureClassifications, platformCoverage.missingFailureClassifications)
    const matrixCoverage = validationGateReportMatrixCoverage(report)
    countStringValues(coverage.requiredMatrices, matrixCoverage.requiredMatrices)
    countStringValues(coverage.missingMatrices, matrixCoverage.missingMatrices)
    countStringValues(coverage.requiredMatrixClassifications, matrixCoverage.requiredMatrixClassifications)
    countStringValues(coverage.missingMatrixClassifications, matrixCoverage.missingMatrixClassifications)
    countStringValues(coverage.requiredDeploymentPresets, matrixCoverage.requiredDeploymentPresets)
    countStringValues(coverage.missingDeploymentPresets, matrixCoverage.missingDeploymentPresets)
    countStringValues(coverage.requiredProviders, matrixCoverage.requiredProviders)
    countStringValues(coverage.missingProviders, matrixCoverage.missingProviders)
    countStringValues(coverage.requiredScenarios, matrixCoverage.requiredScenarios)
    countStringValues(coverage.missingScenarios, matrixCoverage.missingScenarios)
    return {
      source: sources[index] ?? null,
      status: report.status,
      presets: [...(report.presets ?? [])],
      checks: Object.fromEntries(Object.entries(report.checks).map(([name, check]) => [name, check.status])),
      platformCoverage,
      matrixCoverage,
    }
  })
  const aggregate = {
    schema: DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
    status: totals.failed > 0 ? "failed" : "passed",
    totals,
    coverage: formatValidationGateCoverageCounts(coverage),
    nextActions: formatDrillAggregateNextActionCounts(nextActions),
    reports: summaries,
  }
  validateDrillValidationGateAggregate(aggregate)
  return aggregate
}

export function formatDrillValidationGateAggregateSummary(aggregate) {
  validateDrillValidationGateAggregate(aggregate)
  const lines = [
    "drill validation gate aggregate:",
    `status=${aggregate.status} reports=${aggregate.totals.reports} passed=${aggregate.totals.passed} failed=${aggregate.totals.failed}`,
  ]
  if (aggregate.nextActions.length > 0) {
    lines.push("next actions:")
    for (const action of aggregate.nextActions) {
      lines.push(`- owner=${action.owner} classification=${action.classification} count=${action.count}: ${action.nextAction}`)
    }
  }
  if (aggregate.coverage) {
    const coverageLines = formatValidationGateCoverageSummary(aggregate.coverage)
    if (coverageLines.length > 0) {
      lines.push("coverage:")
      lines.push(...coverageLines)
    }
  }
  lines.push(aggregate.status === "passed"
    ? "next: all validation gate reports passed"
    : "next: inspect failed validation gate reports and rerun the relevant drills")
  return lines.join("\n")
}

export function drillValidationGateAggregateExitCode(aggregate) {
  validateDrillValidationGateAggregate(aggregate)
  return aggregate.status === "failed" ? 1 : 0
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
    configuration: configurationCheck({
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
    platformBundle: await platformBundleCheck(platformBundleDir, {
      requiredCoverageAreas: normalizedRequiredPlatformCoverageAreas,
      requiredFailureClassifications: normalizedRequiredFailureClassifications,
    }),
    artifacts: await artifactIndexCheck({ artifactIndexes, artifactRoots }, { maxDepth }),
    matrices: await matrixCheck({
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
    failures: await failureCheck({ failureInputs, failureRoots }, { maxDepth }),
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

export function validateDrillValidationGateReport(report, source = "validation gate report") {
  if (!report || typeof report !== "object" || Array.isArray(report)) {
    throw new Error(`${source} is not an object`)
  }
  if (report.schema !== DRILL_VALIDATION_GATE_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(report.schema)}`)
  }
  if (!["passed", "failed"].includes(report.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(report.status)}`)
  }
  validateStringArray(report.presets ?? [], `${source}.presets`)
  if (!report.checks || typeof report.checks !== "object" || Array.isArray(report.checks)) {
    throw new Error(`${source} is missing checks`)
  }
  validateConfigurationCheck(report.checks.configuration, `${source}.checks.configuration`)
  validatePlatformBundleCheck(report.checks.platformBundle, `${source}.checks.platformBundle`)
  validateArtifactIndexCheck(report.checks.artifacts, `${source}.checks.artifacts`)
  validateMatrixCheck(report.checks.matrices, `${source}.checks.matrices`)
  validateFailureCheck(report.checks.failures, `${source}.checks.failures`)
  if (!Array.isArray(report.nextActions)) {
    throw new Error(`${source} has invalid nextActions`)
  }
  for (const [index, action] of report.nextActions.entries()) {
    validateDrillAggregateNextAction(action, `${source}.nextActions[${index}]`)
  }
  const expectedStatus = Object.values(report.checks).some((check) => check.status === "failed") ? "failed" : "passed"
  if (report.status !== expectedStatus) {
    throw new Error(`${source} status does not match check statuses`)
  }
}

export function validateDrillValidationGateAggregate(aggregate, source = "validation gate aggregate") {
  if (!aggregate || typeof aggregate !== "object" || Array.isArray(aggregate)) {
    throw new Error(`${source} is not an object`)
  }
  if (aggregate.schema !== DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(aggregate.schema)}`)
  }
  if (!["passed", "failed"].includes(aggregate.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(aggregate.status)}`)
  }
  if (!aggregate.totals || typeof aggregate.totals !== "object" || Array.isArray(aggregate.totals)) {
    throw new Error(`${source} has invalid totals`)
  }
  for (const key of ["reports", "passed", "failed"]) {
    if (!Number.isSafeInteger(aggregate.totals[key]) || aggregate.totals[key] < 0) {
      throw new Error(`${source}.totals has invalid ${key}`)
    }
  }
  if (!Array.isArray(aggregate.nextActions)) {
    throw new Error(`${source} has invalid nextActions`)
  }
  for (const [index, action] of aggregate.nextActions.entries()) {
    validateDrillAggregateNextAction(action, `${source}.nextActions[${index}]`)
  }
  if (!Array.isArray(aggregate.reports)) {
    throw new Error(`${source} has invalid reports`)
  }
  for (const [index, report] of aggregate.reports.entries()) {
    validateGateAggregateReportSummary(report, `${source}.reports[${index}]`)
  }
  if (aggregate.coverage !== undefined) {
    validateValidationGateCoverageAggregate(aggregate.coverage, `${source}.coverage`)
  }
  if (aggregate.totals.reports !== aggregate.reports.length) {
    throw new Error(`${source} totals.reports does not match reports`)
  }
  const passed = aggregate.reports.filter((report) => report.status === "passed").length
  const failed = aggregate.reports.filter((report) => report.status === "failed").length
  if (aggregate.totals.passed !== passed || aggregate.totals.failed !== failed) {
    throw new Error(`${source} totals do not match reports`)
  }
  const expectedStatus = aggregate.totals.failed > 0 ? "failed" : "passed"
  if (aggregate.status !== expectedStatus) {
    throw new Error(`${source} status does not match totals`)
  }
  if (aggregate.coverage !== undefined) {
    assertValidationGateCoverageMatchesReports(aggregate, source)
  }
}

function validateConfigurationCheck(check, source) {
  validateCheckObject(check, source)
  if (check.status === "skipped") {
    throw new Error(`${source} cannot be skipped`)
  }
  if (check.status === "failed" && !nonEmptyString(check.error)) {
    throw new Error(`${source} is missing error`)
  }
}

function validatePlatformBundleCheck(check, source) {
  validateCheckObject(check, source)
  validateStringArray(check.requiredCoverageAreas ?? [], `${source}.requiredCoverageAreas`)
  validateStringArray(check.missingCoverageAreas ?? [], `${source}.missingCoverageAreas`)
  validateStringArray(check.requiredFailureClassifications ?? [], `${source}.requiredFailureClassifications`)
  validateStringArray(check.missingFailureClassifications ?? [], `${source}.missingFailureClassifications`)
  if (check.status === "skipped") {
    if (check.dir !== null) {
      throw new Error(`${source} skipped check has invalid dir`)
    }
    if ((check.requiredCoverageAreas ?? []).length > 0
      || (check.missingCoverageAreas ?? []).length > 0
      || (check.requiredFailureClassifications ?? []).length > 0
      || (check.missingFailureClassifications ?? []).length > 0) {
      throw new Error(`${source} skipped check has invalid coverage requirements`)
    }
    return
  }
  if (check.status === "failed") {
    if (!nonEmptyString(check.error)) {
      throw new Error(`${source} is missing error`)
    }
    if (!check.validationSuite) {
      if (check.dir !== null && check.dir !== undefined && !nonEmptyString(check.dir)) {
        throw new Error(`${source} has invalid dir`)
      }
      return
    }
  }
  if (!nonEmptyString(check.dir)) {
    throw new Error(`${source} is missing dir`)
  }
  if (!Array.isArray(check.artifacts)) {
    throw new Error(`${source} has invalid artifacts`)
  }
  for (const [index, artifact] of check.artifacts.entries()) {
    validatePlatformBundleArtifact(artifact, `${source}.artifacts[${index}]`)
  }
  validatePlatformValidationSuiteSummary(check.validationSuite, `${source}.validationSuite`)
  if (check.failureTaxonomy !== undefined) {
    validatePlatformFailureTaxonomySummary(check.failureTaxonomy, `${source}.failureTaxonomy`)
  }
}

function validateArtifactIndexCheck(check, source) {
  validateCheckObject(check, source)
  validateStringArray(check.roots, `${source}.roots`)
  validateStringArray(check.inputs, `${source}.inputs`)
  validateStringArray(check.indexPaths, `${source}.indexPaths`)
  if (check.status === "failed" && !check.aggregate && !nonEmptyString(check.error)) {
    throw new Error(`${source} is missing error`)
  }
  if (check.aggregate) {
    validateAggregateSchema(check.aggregate, "arroba.drill.artifact_index.aggregate.v1", `${source}.aggregate`)
  }
}

function validateMatrixCheck(check, source) {
  validateCheckObject(check, source)
  validateStringArray(check.roots, `${source}.roots`)
  validateStringArray(check.inputs, `${source}.inputs`)
  validateStringArray(check.reportPaths, `${source}.reportPaths`)
  validateStringArray(check.requiredMatrices ?? [], `${source}.requiredMatrices`)
  validateStringArray(check.missingMatrices ?? [], `${source}.missingMatrices`)
  validateStringArray(check.requiredMatrixClassifications ?? [], `${source}.requiredMatrixClassifications`)
  validateStringArray(check.missingMatrixClassifications ?? [], `${source}.missingMatrixClassifications`)
  validateStringArray(check.requiredDeploymentPresets ?? [], `${source}.requiredDeploymentPresets`)
  validateStringArray(check.missingDeploymentPresets ?? [], `${source}.missingDeploymentPresets`)
  validateStringArray(check.requiredProviders ?? [], `${source}.requiredProviders`)
  validateStringArray(check.missingProviders ?? [], `${source}.missingProviders`)
  validateStringArray(check.requiredScenarios ?? [], `${source}.requiredScenarios`)
  validateStringArray(check.missingScenarios ?? [], `${source}.missingScenarios`)
  if (typeof check.requireComplete !== "boolean") {
    throw new Error(`${source} has invalid requireComplete`)
  }
  if (check.status === "failed" && !check.aggregate && !nonEmptyString(check.error)) {
    throw new Error(`${source} is missing error`)
  }
  if (check.aggregate) {
    validateAggregateSchema(check.aggregate, "arroba.drill.matrix.aggregate.v1", `${source}.aggregate`)
  }
}

function validateFailureCheck(check, source) {
  validateCheckObject(check, source)
  validateStringArray(check.roots, `${source}.roots`)
  validateStringArray(check.inputs, `${source}.inputs`)
  validateStringArray(check.manifestPaths, `${source}.manifestPaths`)
  if (check.status === "failed" && !check.aggregate && !nonEmptyString(check.error)) {
    throw new Error(`${source} is missing error`)
  }
  if (check.aggregate) {
    validateAggregateSchema(check.aggregate, "arroba.drill.failure.aggregate.v1", `${source}.aggregate`)
  }
}

function validateCheckObject(check, source) {
  if (!check || typeof check !== "object" || Array.isArray(check)) {
    throw new Error(`${source} is not an object`)
  }
  if (!["passed", "failed", "skipped"].includes(check.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(check.status)}`)
  }
}

function validatePlatformBundleArtifact(artifact, source) {
  if (!artifact || typeof artifact !== "object" || Array.isArray(artifact)) {
    throw new Error(`${source} is not an object`)
  }
  for (const key of ["path", "schema"]) {
    if (!nonEmptyString(artifact[key])) {
      throw new Error(`${source} is missing ${key}`)
    }
  }
  if (typeof artifact.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(artifact.sha256)) {
    throw new Error(`${source} has invalid sha256`)
  }
  if (!Number.isSafeInteger(artifact.sizeBytes) || artifact.sizeBytes < 0) {
    throw new Error(`${source} has invalid sizeBytes`)
  }
}

function validatePlatformValidationSuiteSummary(summary, source) {
  if (!summary || typeof summary !== "object" || Array.isArray(summary)) {
    throw new Error(`${source} is not an object`)
  }
  if (!Number.isSafeInteger(summary.testCount) || summary.testCount <= 0) {
    throw new Error(`${source} has invalid testCount`)
  }
  if (!Array.isArray(summary.coverageAreas) || summary.coverageAreas.length === 0) {
    throw new Error(`${source} has invalid coverageAreas`)
  }
  let coveredTests = 0
  const areaIds = new Set()
  for (const [index, area] of summary.coverageAreas.entries()) {
    const areaSource = `${source}.coverageAreas[${index}]`
    if (!area || typeof area !== "object" || Array.isArray(area)) {
      throw new Error(`${areaSource} is not an object`)
    }
    if (!nonEmptyString(area.id)) {
      throw new Error(`${areaSource} has invalid id`)
    }
    if (areaIds.has(area.id)) {
      throw new Error(`${source} has duplicate coverage area ${area.id}`)
    }
    areaIds.add(area.id)
    if (!Number.isSafeInteger(area.testCount) || area.testCount <= 0) {
      throw new Error(`${areaSource} has invalid testCount`)
    }
    coveredTests += area.testCount
  }
  if (coveredTests !== summary.testCount) {
    throw new Error(`${source} coverageAreas do not match testCount`)
  }
}

function validatePlatformFailureTaxonomySummary(summary, source) {
  if (!summary || typeof summary !== "object" || Array.isArray(summary)) {
    throw new Error(`${source} is not an object`)
  }
  validateStringArray(summary.drill, `${source}.drill`)
  validateStringArray(summary.scenario, `${source}.scenario`)
}

function validateAggregateSchema(aggregate, schema, source) {
  if (!aggregate || typeof aggregate !== "object" || Array.isArray(aggregate)) {
    throw new Error(`${source} is not an object`)
  }
  if (aggregate.schema !== schema) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(aggregate.schema)}`)
  }
}

function validateStringArray(value, source) {
  if (!Array.isArray(value)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, entry] of value.entries()) {
    if (typeof entry !== "string") {
      throw new Error(`${source}[${index}] is not a string`)
    }
  }
}

function validateCountObject(value, source) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is not an object`)
  }
  for (const [key, count] of Object.entries(value)) {
    if (!nonEmptyString(key) || !Number.isSafeInteger(count) || count < 0) {
      throw new Error(`${source} has invalid count for ${JSON.stringify(key)}`)
    }
  }
}

function validateGateAggregateReportSummary(report, source) {
  if (!report || typeof report !== "object" || Array.isArray(report)) {
    throw new Error(`${source} is not an object`)
  }
  if (report.source !== null && typeof report.source !== "string") {
    throw new Error(`${source} has invalid source`)
  }
  if (!["passed", "failed"].includes(report.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(report.status)}`)
  }
  validateStringArray(report.presets ?? [], `${source}.presets`)
  if (!report.checks || typeof report.checks !== "object" || Array.isArray(report.checks)) {
    throw new Error(`${source} has invalid checks`)
  }
  for (const name of ["configuration", "platformBundle", "artifacts", "matrices", "failures"]) {
    if (!["passed", "failed", "skipped"].includes(report.checks[name])) {
      throw new Error(`${source}.checks has invalid ${name}`)
    }
  }
  if (report.matrixCoverage !== undefined) {
    validateValidationGateMatrixCoverage(report.matrixCoverage, `${source}.matrixCoverage`)
  }
  if (report.platformCoverage !== undefined) {
    validateValidationGatePlatformCoverage(report.platformCoverage, `${source}.platformCoverage`)
  }
}

function validationGateReportPlatformCoverage(report) {
  const platform = report.checks.platformBundle
  return {
    requiredCoverageAreas: [...(platform.requiredCoverageAreas ?? [])],
    missingCoverageAreas: [...(platform.missingCoverageAreas ?? [])],
    requiredFailureClassifications: [...(platform.requiredFailureClassifications ?? [])],
    missingFailureClassifications: [...(platform.missingFailureClassifications ?? [])],
  }
}

function validationGateReportMatrixCoverage(report) {
  const matrices = report.checks.matrices
  return {
    requiredMatrices: [...(matrices.requiredMatrices ?? [])],
    missingMatrices: [...(matrices.missingMatrices ?? [])],
    requiredMatrixClassifications: [...(matrices.requiredMatrixClassifications ?? [])],
    missingMatrixClassifications: [...(matrices.missingMatrixClassifications ?? [])],
    requiredDeploymentPresets: [...(matrices.requiredDeploymentPresets ?? [])],
    missingDeploymentPresets: [...(matrices.missingDeploymentPresets ?? [])],
    requiredProviders: [...(matrices.requiredProviders ?? [])],
    missingProviders: [...(matrices.missingProviders ?? [])],
    requiredScenarios: [...(matrices.requiredScenarios ?? [])],
    missingScenarios: [...(matrices.missingScenarios ?? [])],
  }
}

function countStringValues(counts, values) {
  for (const value of values) {
    counts.set(value, (counts.get(value) ?? 0) + 1)
  }
}

function formatValidationGateCoverageCounts(coverage) {
  return {
    presets: countMapToObject(coverage.presets),
    requiredPlatformCoverageAreas: countMapToObject(coverage.requiredPlatformCoverageAreas),
    missingPlatformCoverageAreas: countMapToObject(coverage.missingPlatformCoverageAreas),
    requiredFailureClassifications: countMapToObject(coverage.requiredFailureClassifications),
    missingFailureClassifications: countMapToObject(coverage.missingFailureClassifications),
    requiredMatrices: countMapToObject(coverage.requiredMatrices),
    missingMatrices: countMapToObject(coverage.missingMatrices),
    requiredMatrixClassifications: countMapToObject(coverage.requiredMatrixClassifications),
    missingMatrixClassifications: countMapToObject(coverage.missingMatrixClassifications),
    requiredDeploymentPresets: countMapToObject(coverage.requiredDeploymentPresets),
    missingDeploymentPresets: countMapToObject(coverage.missingDeploymentPresets),
    requiredProviders: countMapToObject(coverage.requiredProviders),
    missingProviders: countMapToObject(coverage.missingProviders),
    requiredScenarios: countMapToObject(coverage.requiredScenarios),
    missingScenarios: countMapToObject(coverage.missingScenarios),
  }
}

function countMapToObject(counts) {
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

function formatValidationGateCoverageSummary(coverage) {
  const lines = []
  appendCoverageLine(lines, "presets", coverage.presets)
  appendCoverageLine(lines, "required_platform_coverage_areas", coverage.requiredPlatformCoverageAreas)
  appendCoverageLine(lines, "missing_platform_coverage_areas", coverage.missingPlatformCoverageAreas)
  appendCoverageLine(lines, "required_failure_classifications", coverage.requiredFailureClassifications)
  appendCoverageLine(lines, "missing_failure_classifications", coverage.missingFailureClassifications)
  appendCoverageLine(lines, "required_matrices", coverage.requiredMatrices)
  appendCoverageLine(lines, "missing_matrices", coverage.missingMatrices)
  appendCoverageLine(lines, "required_matrix_classifications", coverage.requiredMatrixClassifications)
  appendCoverageLine(lines, "missing_matrix_classifications", coverage.missingMatrixClassifications)
  appendCoverageLine(lines, "required_deployment_presets", coverage.requiredDeploymentPresets)
  appendCoverageLine(lines, "missing_deployment_presets", coverage.missingDeploymentPresets)
  appendCoverageLine(lines, "required_providers", coverage.requiredProviders)
  appendCoverageLine(lines, "missing_providers", coverage.missingProviders)
  appendCoverageLine(lines, "required_scenarios", coverage.requiredScenarios)
  appendCoverageLine(lines, "missing_scenarios", coverage.missingScenarios)
  return lines
}

function appendCoverageLine(lines, label, counts) {
  const entries = Object.entries(counts ?? {})
  if (entries.length > 0) {
    lines.push(`- ${label}: ${entries.map(([key, count]) => `${key}=${count}`).join(" ")}`)
  }
}

function validateValidationGateCoverageAggregate(coverage, source) {
  if (!coverage || typeof coverage !== "object" || Array.isArray(coverage)) {
    throw new Error(`${source} is not an object`)
  }
  validateCountObject(coverage.presets ?? {}, `${source}.presets`)
  validateCountObject(coverage.requiredPlatformCoverageAreas ?? {}, `${source}.requiredPlatformCoverageAreas`)
  validateCountObject(coverage.missingPlatformCoverageAreas ?? {}, `${source}.missingPlatformCoverageAreas`)
  validateCountObject(coverage.requiredFailureClassifications ?? {}, `${source}.requiredFailureClassifications`)
  validateCountObject(coverage.missingFailureClassifications ?? {}, `${source}.missingFailureClassifications`)
  validateCountObject(coverage.requiredMatrices ?? {}, `${source}.requiredMatrices`)
  validateCountObject(coverage.missingMatrices ?? {}, `${source}.missingMatrices`)
  validateCountObject(coverage.requiredMatrixClassifications ?? {}, `${source}.requiredMatrixClassifications`)
  validateCountObject(coverage.missingMatrixClassifications ?? {}, `${source}.missingMatrixClassifications`)
  validateCountObject(coverage.requiredDeploymentPresets ?? {}, `${source}.requiredDeploymentPresets`)
  validateCountObject(coverage.missingDeploymentPresets ?? {}, `${source}.missingDeploymentPresets`)
  validateCountObject(coverage.requiredProviders ?? {}, `${source}.requiredProviders`)
  validateCountObject(coverage.missingProviders ?? {}, `${source}.missingProviders`)
  validateCountObject(coverage.requiredScenarios ?? {}, `${source}.requiredScenarios`)
  validateCountObject(coverage.missingScenarios ?? {}, `${source}.missingScenarios`)
}

function validateValidationGateMatrixCoverage(coverage, source) {
  if (!coverage || typeof coverage !== "object" || Array.isArray(coverage)) {
    throw new Error(`${source} is not an object`)
  }
  validateStringArray(coverage.requiredMatrices ?? [], `${source}.requiredMatrices`)
  validateStringArray(coverage.missingMatrices ?? [], `${source}.missingMatrices`)
  validateStringArray(coverage.requiredMatrixClassifications ?? [], `${source}.requiredMatrixClassifications`)
  validateStringArray(coverage.missingMatrixClassifications ?? [], `${source}.missingMatrixClassifications`)
  validateStringArray(coverage.requiredDeploymentPresets ?? [], `${source}.requiredDeploymentPresets`)
  validateStringArray(coverage.missingDeploymentPresets ?? [], `${source}.missingDeploymentPresets`)
  validateStringArray(coverage.requiredProviders ?? [], `${source}.requiredProviders`)
  validateStringArray(coverage.missingProviders ?? [], `${source}.missingProviders`)
  validateStringArray(coverage.requiredScenarios ?? [], `${source}.requiredScenarios`)
  validateStringArray(coverage.missingScenarios ?? [], `${source}.missingScenarios`)
}

function validateValidationGatePlatformCoverage(coverage, source) {
  if (!coverage || typeof coverage !== "object" || Array.isArray(coverage)) {
    throw new Error(`${source} is not an object`)
  }
  validateStringArray(coverage.requiredCoverageAreas ?? [], `${source}.requiredCoverageAreas`)
  validateStringArray(coverage.missingCoverageAreas ?? [], `${source}.missingCoverageAreas`)
  validateStringArray(coverage.requiredFailureClassifications ?? [], `${source}.requiredFailureClassifications`)
  validateStringArray(coverage.missingFailureClassifications ?? [], `${source}.missingFailureClassifications`)
}

function assertValidationGateCoverageMatchesReports(aggregate, source) {
  const expected = {
    presets: new Map(),
    requiredPlatformCoverageAreas: new Map(),
    missingPlatformCoverageAreas: new Map(),
    requiredFailureClassifications: new Map(),
    missingFailureClassifications: new Map(),
    requiredMatrices: new Map(),
    missingMatrices: new Map(),
    requiredMatrixClassifications: new Map(),
    missingMatrixClassifications: new Map(),
    requiredDeploymentPresets: new Map(),
    missingDeploymentPresets: new Map(),
    requiredProviders: new Map(),
    missingProviders: new Map(),
    requiredScenarios: new Map(),
    missingScenarios: new Map(),
  }
  for (const report of aggregate.reports) {
    countStringValues(expected.presets, report.presets ?? [])
    const platformCoverage = report.platformCoverage ?? {
      requiredCoverageAreas: [],
      missingCoverageAreas: [],
      requiredFailureClassifications: [],
      missingFailureClassifications: [],
    }
    countStringValues(expected.requiredPlatformCoverageAreas, platformCoverage.requiredCoverageAreas ?? [])
    countStringValues(expected.missingPlatformCoverageAreas, platformCoverage.missingCoverageAreas ?? [])
    countStringValues(expected.requiredFailureClassifications, platformCoverage.requiredFailureClassifications ?? [])
    countStringValues(expected.missingFailureClassifications, platformCoverage.missingFailureClassifications ?? [])
    const coverage = report.matrixCoverage ?? {
      requiredMatrices: [],
      missingMatrices: [],
      requiredMatrixClassifications: [],
      missingMatrixClassifications: [],
      requiredDeploymentPresets: [],
      missingDeploymentPresets: [],
      requiredProviders: [],
      missingProviders: [],
      requiredScenarios: [],
      missingScenarios: [],
    }
    countStringValues(expected.requiredMatrices, coverage.requiredMatrices ?? [])
    countStringValues(expected.missingMatrices, coverage.missingMatrices ?? [])
    countStringValues(expected.requiredMatrixClassifications, coverage.requiredMatrixClassifications ?? [])
    countStringValues(expected.missingMatrixClassifications, coverage.missingMatrixClassifications ?? [])
    countStringValues(expected.requiredDeploymentPresets, coverage.requiredDeploymentPresets ?? [])
    countStringValues(expected.missingDeploymentPresets, coverage.missingDeploymentPresets ?? [])
    countStringValues(expected.requiredProviders, coverage.requiredProviders ?? [])
    countStringValues(expected.missingProviders, coverage.missingProviders ?? [])
    countStringValues(expected.requiredScenarios, coverage.requiredScenarios ?? [])
    countStringValues(expected.missingScenarios, coverage.missingScenarios ?? [])
  }
  const expectedCoverage = formatValidationGateCoverageCounts(expected)
  if (JSON.stringify(aggregate.coverage) !== JSON.stringify(expectedCoverage)) {
    throw new Error(`${source} coverage does not match reports`)
  }
}

function validationGateNextActions(checks) {
  const counts = new Map()
  if (checks.configuration.status === "failed") {
    countDrillAggregateNextAction(counts, {
      owner: "validation-harness",
      classification: "validation-gate",
      nextAction: "configure at least one platform bundle, artifact root, matrix root, or failure root before using the validation gate",
    })
  }
  if (checks.platformBundle.status === "failed") {
    countDrillAggregateNextAction(counts, {
      owner: "validation-harness",
      classification: "platform-bundle",
      nextAction: "rebuild the drill platform bundle and verify it before using collected artifacts as evidence",
    })
    const missingCoverageAreas = checks.platformBundle.missingCoverageAreas ?? []
    if (missingCoverageAreas.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: `provide a drill platform bundle covering: ${missingCoverageAreas.join(", ")}`,
      })
    }
    const missingFailureClassifications = checks.platformBundle.missingFailureClassifications ?? []
    if (missingFailureClassifications.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "platform-bundle",
        nextAction: `provide a drill platform bundle covering failure classifications: ${missingFailureClassifications.join(", ")}`,
      })
    }
  }
  if (checks.artifacts.status === "failed") {
    countDrillAggregateNextAction(counts, {
      owner: "validation-harness",
      classification: "artifact-index",
      nextAction: "fix missing, unreadable, or tampered artifact indexes before using collected drill evidence",
    })
  }
  if (checks.matrices.status === "failed") {
    if (checks.matrices.error) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-artifacts",
        nextAction: "produce matrix reports under the configured matrix roots, then rerun the validation gate",
      })
    }
    for (const action of checks.matrices.aggregate?.nextActions ?? []) {
      countDrillAggregateNextAction(counts, action)
    }
    if (checks.matrices.requireComplete && (checks.matrices.aggregate?.incompleteScenarios?.length ?? 0) > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "incomplete-matrix",
        nextAction: "run skipped or dry-run matrix scenarios before treating this validation set as complete",
      })
    }
    const missingMatrices = checks.matrices.missingMatrices ?? []
    if (missingMatrices.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-coverage",
        nextAction: `run missing drill matrices: ${missingMatrices.join(", ")}`,
      })
    }
    const missingMatrixClassifications = checks.matrices.missingMatrixClassifications ?? []
    if (missingMatrixClassifications.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-coverage",
        nextAction: `run matrix reports covering failure classifications: ${missingMatrixClassifications.join(", ")}`,
      })
    }
    const missingDeploymentPresets = checks.matrices.missingDeploymentPresets ?? []
    if (missingDeploymentPresets.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-coverage",
        nextAction: `run matrix reports for missing deployment presets: ${missingDeploymentPresets.join(", ")}`,
      })
    }
    const missingProviders = checks.matrices.missingProviders ?? []
    if (missingProviders.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-coverage",
        nextAction: `run matrix reports for missing providers: ${missingProviders.join(", ")}`,
      })
    }
    const missingScenarios = checks.matrices.missingScenarios ?? []
    if (missingScenarios.length > 0) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "matrix-coverage",
        nextAction: `run matrix reports for missing scenarios: ${missingScenarios.join(", ")}`,
      })
    }
  }
  if (checks.failures.status === "failed") {
    if (checks.failures.error) {
      countDrillAggregateNextAction(counts, {
        owner: "validation-harness",
        classification: "failure-artifacts",
        nextAction: "fix unreadable failure artifacts or discovery configuration, then rerun the validation gate",
      })
    }
    for (const action of checks.failures.aggregate?.nextActions ?? []) {
      countDrillAggregateNextAction(counts, action)
    }
  }
  return formatDrillAggregateNextActionCounts(counts)
}

function configurationCheck({
  artifactIndexes,
  artifactRoots,
  failureInputs,
  failureRoots,
  matrixReports,
  matrixRoots,
  platformBundleDir,
  requiredPlatformCoverageAreas,
  requiredFailureClassifications,
  requiredMatrices,
  requiredMatrixClassifications,
  requiredDeploymentPresets,
  requiredProviders,
  requiredScenarios,
}) {
  const configured = Boolean(platformBundleDir)
    || artifactRoots.length > 0
    || artifactIndexes.length > 0
    || matrixRoots.length > 0
    || matrixReports.length > 0
    || requiredPlatformCoverageAreas.length > 0
    || requiredFailureClassifications.length > 0
    || failureRoots.length > 0
    || failureInputs.length > 0
    || requiredMatrices.length > 0
    || requiredMatrixClassifications.length > 0
    || requiredDeploymentPresets.length > 0
    || requiredProviders.length > 0
    || requiredScenarios.length > 0
  return configured
    ? { status: "passed" }
    : {
        status: "failed",
        error: "no validation checks configured",
      }
}

async function artifactIndexCheck({ artifactIndexes, artifactRoots }, { maxDepth }) {
  if (artifactRoots.length === 0 && artifactIndexes.length === 0) {
    return {
      status: "skipped",
      roots: [],
      inputs: [],
      indexPaths: [],
    }
  }
  try {
    const discovered = artifactRoots.length > 0
      ? await findDrillArtifactIndexPaths(artifactRoots, { maxDepth })
      : []
    const indexPaths = [...new Set([...artifactIndexes, ...discovered])].sort()
    if (indexPaths.length === 0) {
      return {
        status: "failed",
        roots: [...artifactRoots],
        inputs: [...artifactIndexes],
        indexPaths,
        error: "no artifact indexes found",
      }
    }
    const indexes = await Promise.all(indexPaths.map((indexPath) => verifyDrillArtifactIndex(indexPath)))
    const aggregate = summarizeDrillArtifactIndexes(indexes, { sources: indexPaths })
    return {
      status: "passed",
      roots: [...artifactRoots],
      inputs: [...artifactIndexes],
      indexPaths,
      aggregate,
    }
  } catch (error) {
    return {
      status: "failed",
      roots: [...artifactRoots],
      inputs: [...artifactIndexes],
      indexPaths: [],
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

async function platformBundleCheck(platformBundleDir, { requiredCoverageAreas = [], requiredFailureClassifications = [] } = {}) {
  if (!platformBundleDir) {
    if (requiredCoverageAreas.length > 0 || requiredFailureClassifications.length > 0) {
      return {
        status: "failed",
        dir: null,
        requiredCoverageAreas: [...requiredCoverageAreas],
        missingCoverageAreas: [...requiredCoverageAreas],
        requiredFailureClassifications: [...requiredFailureClassifications],
        missingFailureClassifications: [...requiredFailureClassifications],
        error: "no platform bundle provided",
      }
    }
    return {
      status: "skipped",
      dir: null,
      requiredCoverageAreas: [],
      missingCoverageAreas: [],
      requiredFailureClassifications: [],
      missingFailureClassifications: [],
    }
  }
  try {
    const bundle = await verifyDrillPlatformBundle(platformBundleDir)
    const validationSuite = await readPlatformBundleValidationSuite(platformBundleDir)
    const failureTaxonomy = await readPlatformBundleFailureTaxonomy(platformBundleDir)
    const missingCoverageAreas = missingRequiredPlatformCoverageAreas(validationSuite, requiredCoverageAreas)
    const missingFailureClassifications = missingRequiredFailureClassifications(failureTaxonomy, requiredFailureClassifications)
    const errors = [
      ...(missingCoverageAreas.length > 0 ? [`missing platform coverage areas: ${missingCoverageAreas.join(", ")}`] : []),
      ...(missingFailureClassifications.length > 0 ? [`missing failure classifications: ${missingFailureClassifications.join(", ")}`] : []),
    ]
    return {
      status: errors.length === 0 ? "passed" : "failed",
      dir: platformBundleDir,
      requiredCoverageAreas: [...requiredCoverageAreas],
      missingCoverageAreas,
      requiredFailureClassifications: [...requiredFailureClassifications],
      missingFailureClassifications,
      ...(errors.length > 0 ? { error: errors.join("; ") } : {}),
      artifacts: bundle.artifacts.map((artifact) => ({
        path: artifact.path,
        schema: artifact.schema,
        sha256: artifact.sha256,
        sizeBytes: artifact.sizeBytes,
      })),
      validationSuite,
      failureTaxonomy,
    }
  } catch (error) {
    return {
      status: "failed",
      dir: platformBundleDir,
      requiredCoverageAreas: [...requiredCoverageAreas],
      missingCoverageAreas: [...requiredCoverageAreas],
      requiredFailureClassifications: [...requiredFailureClassifications],
      missingFailureClassifications: [...requiredFailureClassifications],
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

async function readPlatformBundleValidationSuite(platformBundleDir) {
  const suite = JSON.parse(await readFile(path.join(platformBundleDir, "validation-suite.json"), "utf8"))
  return {
    testCount: suite.testCount,
    coverageAreas: suite.coverage.map((area) => ({
      id: area.id,
      testCount: area.testCount,
    })),
  }
}

async function readPlatformBundleFailureTaxonomy(platformBundleDir) {
  const [drill, scenario] = await Promise.all([
    readPlatformBundleFailureTaxonomyKinds(platformBundleDir, "drill"),
    readPlatformBundleFailureTaxonomyKinds(platformBundleDir, "scenario"),
  ])
  return { drill, scenario }
}

async function readPlatformBundleFailureTaxonomyKinds(platformBundleDir, target) {
  const taxonomy = JSON.parse(await readFile(path.join(platformBundleDir, `failure-taxonomy-${target}.json`), "utf8"))
  if (taxonomy.schema !== "arroba.drill.failure_taxonomy.v1") {
    throw new Error(`failure taxonomy ${target} has unsupported schema ${JSON.stringify(taxonomy.schema)}`)
  }
  if (taxonomy.target !== target) {
    throw new Error(`failure taxonomy ${target} has invalid target ${JSON.stringify(taxonomy.target)}`)
  }
  if (!Array.isArray(taxonomy.classifications)) {
    throw new Error(`failure taxonomy ${target} has invalid classifications`)
  }
  return [...new Set(taxonomy.classifications
    .map((entry) => entry?.kind)
    .filter((kind) => typeof kind === "string"))].sort()
}

function missingRequiredPlatformCoverageAreas(validationSuite, requiredCoverageAreas) {
  const present = new Set((validationSuite.coverageAreas ?? []).map((area) => area.id))
  return requiredCoverageAreas.filter((area) => !present.has(area))
}

function missingRequiredFailureClassifications(failureTaxonomy, requiredFailureClassifications) {
  const drill = new Set(failureTaxonomy.drill ?? [])
  const scenario = new Set(failureTaxonomy.scenario ?? [])
  return requiredFailureClassifications.filter((classification) => !drill.has(classification) || !scenario.has(classification))
}

function expandValidationGatePresetRequirements({
  presets,
  requiredPlatformCoverageAreas,
  requiredFailureClassifications,
  requiredMatrices,
  requiredMatrixClassifications,
  requiredDeploymentPresets,
  requiredProviders,
  requiredScenarios,
}) {
  const expanded = {
    requiredPlatformCoverageAreas: [...requiredPlatformCoverageAreas],
    requiredFailureClassifications: [...requiredFailureClassifications],
    requiredMatrices: [...requiredMatrices],
    requiredMatrixClassifications: [...requiredMatrixClassifications],
    requiredDeploymentPresets: [...requiredDeploymentPresets],
    requiredProviders: [...requiredProviders],
    requiredScenarios: [...requiredScenarios],
  }
  for (const presetName of presets) {
    const preset = DRILL_VALIDATION_GATE_PRESETS[presetName]
    expanded.requiredPlatformCoverageAreas.push(...(preset.requiredPlatformCoverageAreas ?? []))
    expanded.requiredFailureClassifications.push(...(preset.requiredFailureClassifications ?? []))
    expanded.requiredMatrices.push(...(preset.requiredMatrices ?? []))
    expanded.requiredMatrixClassifications.push(...(preset.requiredMatrixClassifications ?? []))
    expanded.requiredDeploymentPresets.push(...(preset.requiredDeploymentPresets ?? []))
    expanded.requiredProviders.push(...(preset.requiredProviders ?? []))
    expanded.requiredScenarios.push(...(preset.requiredScenarios ?? []))
  }
  return expanded
}

async function matrixCheck({ matrixReports, matrixRoots }, {
  maxDepth,
  requireComplete,
  requiredMatrices,
  requiredMatrixClassifications,
  requiredDeploymentPresets,
  requiredProviders,
  requiredScenarios,
}) {
  if (matrixRoots.length === 0
    && matrixReports.length === 0
    && requiredMatrices.length === 0
    && requiredMatrixClassifications.length === 0
    && requiredDeploymentPresets.length === 0
    && requiredProviders.length === 0
    && requiredScenarios.length === 0) {
    return {
      status: "skipped",
      roots: [],
      inputs: [],
      reportPaths: [],
      requireComplete,
      requiredMatrices: [],
      missingMatrices: [],
      requiredMatrixClassifications: [],
      missingMatrixClassifications: [],
      requiredDeploymentPresets: [],
      missingDeploymentPresets: [],
      requiredProviders: [],
      missingProviders: [],
      requiredScenarios: [],
      missingScenarios: [],
    }
  }
  try {
    const discovered = matrixRoots.length > 0
      ? await findDrillMatrixReportPaths(matrixRoots, { maxDepth })
      : []
    const reportPaths = [...new Set([...matrixReports, ...discovered])].sort()
    if (reportPaths.length === 0) {
      return {
        status: "failed",
        roots: [...matrixRoots],
        inputs: [...matrixReports],
        reportPaths,
        requireComplete,
        requiredMatrices: [...requiredMatrices],
        missingMatrices: [...requiredMatrices],
        requiredMatrixClassifications: [...requiredMatrixClassifications],
        missingMatrixClassifications: [...requiredMatrixClassifications],
        requiredDeploymentPresets: [...requiredDeploymentPresets],
        missingDeploymentPresets: [...requiredDeploymentPresets],
        requiredProviders: [...requiredProviders],
        missingProviders: [...requiredProviders],
        requiredScenarios: [...requiredScenarios],
        missingScenarios: [...requiredScenarios],
        error: "no matrix reports found",
      }
    }
    const reports = await Promise.all(reportPaths.map((reportPath) => readDrillMatrixReport(reportPath)))
    const aggregate = summarizeDrillMatrixReports(reports, { sources: reportPaths })
    const exitCode = requireComplete
      ? drillMatrixReportCompletionExitCode(reports)
      : drillMatrixReportExitCode(reports)
    const missingMatrices = missingRequiredMatrices(aggregate, requiredMatrices)
    const missingMatrixClassifications = missingRequiredMatrixClassifications(aggregate, requiredMatrixClassifications)
    const missingDeploymentPresets = missingRequiredDeploymentPresets(aggregate, requiredDeploymentPresets)
    const missingProviders = missingRequiredProviders(aggregate, requiredProviders)
    const missingScenarios = missingRequiredScenarios(aggregate, requiredScenarios)
    return {
      status: exitCode === 0
        && missingMatrices.length === 0
        && missingMatrixClassifications.length === 0
        && missingDeploymentPresets.length === 0
        && missingProviders.length === 0
        && missingScenarios.length === 0
        ? "passed"
        : "failed",
      roots: [...matrixRoots],
      inputs: [...matrixReports],
      reportPaths,
      requireComplete,
      requiredMatrices: [...requiredMatrices],
      missingMatrices,
      requiredMatrixClassifications: [...requiredMatrixClassifications],
      missingMatrixClassifications,
      requiredDeploymentPresets: [...requiredDeploymentPresets],
      missingDeploymentPresets,
      requiredProviders: [...requiredProviders],
      missingProviders,
      requiredScenarios: [...requiredScenarios],
      missingScenarios,
      aggregate,
    }
  } catch (error) {
    return {
      status: "failed",
      roots: [...matrixRoots],
      inputs: [...matrixReports],
      reportPaths: [],
      requireComplete,
      requiredMatrices: [...requiredMatrices],
      missingMatrices: [...requiredMatrices],
      requiredMatrixClassifications: [...requiredMatrixClassifications],
      missingMatrixClassifications: [...requiredMatrixClassifications],
      requiredDeploymentPresets: [...requiredDeploymentPresets],
      missingDeploymentPresets: [...requiredDeploymentPresets],
      requiredProviders: [...requiredProviders],
      missingProviders: [...requiredProviders],
      requiredScenarios: [...requiredScenarios],
      missingScenarios: [...requiredScenarios],
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

function missingRequiredDeploymentPresets(aggregate, requiredDeploymentPresets) {
  const present = new Set(Object.keys(aggregate.deploymentPresets ?? {}))
  return requiredDeploymentPresets.filter((preset) => !present.has(preset))
}

function missingRequiredProviders(aggregate, requiredProviders) {
  const present = new Set(Object.keys(aggregate.providers ?? {}))
  return requiredProviders.filter((provider) => !present.has(provider))
}

function missingRequiredScenarios(aggregate, requiredScenarios) {
  const present = new Set(Object.keys(aggregate.scenarioIds ?? {}))
  return requiredScenarios.filter((scenario) => !present.has(scenario))
}

function missingRequiredMatrices(aggregate, requiredMatrices) {
  const present = new Set(Object.keys(aggregate.matrixNames ?? {}))
  return requiredMatrices.filter((matrix) => !present.has(matrix))
}

function missingRequiredMatrixClassifications(aggregate, requiredMatrixClassifications) {
  const present = new Set(Object.keys(aggregate.classifications ?? {}))
  return requiredMatrixClassifications.filter((classification) => !present.has(classification))
}

function normalizeRequiredPlatformCoverageAreas(requiredPlatformCoverageAreas) {
  if (!Array.isArray(requiredPlatformCoverageAreas)) {
    throw new Error("requiredPlatformCoverageAreas must be an array")
  }
  const areas = []
  for (const area of requiredPlatformCoverageAreas) {
    if (!nonEmptyString(area)) {
      throw new Error("requiredPlatformCoverageAreas has invalid area")
    }
    for (const value of area.split(",")) {
      const normalized = value.trim()
      if (normalized) areas.push(normalized)
    }
  }
  return [...new Set(areas)].sort()
}

function normalizeRequiredPresets(presets) {
  if (!Array.isArray(presets)) {
    throw new Error("presets must be an array")
  }
  const names = []
  for (const preset of presets) {
    if (!nonEmptyString(preset)) {
      throw new Error("presets has invalid preset")
    }
    for (const value of preset.split(",")) {
      const normalized = value.trim()
      if (normalized) names.push(normalized)
    }
  }
  const normalizedNames = [...new Set(names)].sort()
  for (const preset of normalizedNames) {
    if (!Object.prototype.hasOwnProperty.call(DRILL_VALIDATION_GATE_PRESETS, preset)) {
      throw new Error(`unknown validation gate preset: ${preset}`)
    }
  }
  return normalizedNames
}

function normalizeRequiredFailureClassifications(requiredFailureClassifications) {
  if (!Array.isArray(requiredFailureClassifications)) {
    throw new Error("requiredFailureClassifications must be an array")
  }
  const classifications = []
  for (const classification of requiredFailureClassifications) {
    if (!nonEmptyString(classification)) {
      throw new Error("requiredFailureClassifications has invalid classification")
    }
    for (const value of classification.split(",")) {
      const normalized = value.trim()
      if (normalized) classifications.push(normalized)
    }
  }
  const normalizedClassifications = [...new Set(classifications)].sort()
  for (const classification of normalizedClassifications) {
    if (!isKnownDrillFailureClassification(classification)) {
      throw new Error(`unknown required failure classification: ${classification}`)
    }
  }
  return normalizedClassifications
}

function normalizeRequiredMatrices(requiredMatrices) {
  if (!Array.isArray(requiredMatrices)) {
    throw new Error("requiredMatrices must be an array")
  }
  const matrices = []
  for (const matrix of requiredMatrices) {
    if (!nonEmptyString(matrix)) {
      throw new Error("requiredMatrices has invalid matrix")
    }
    for (const value of matrix.split(",")) {
      const normalized = value.trim()
      if (normalized) matrices.push(normalized)
    }
  }
  return [...new Set(matrices)].sort()
}

function normalizeRequiredMatrixClassifications(requiredMatrixClassifications) {
  if (!Array.isArray(requiredMatrixClassifications)) {
    throw new Error("requiredMatrixClassifications must be an array")
  }
  const classifications = []
  for (const classification of requiredMatrixClassifications) {
    if (!nonEmptyString(classification)) {
      throw new Error("requiredMatrixClassifications has invalid classification")
    }
    for (const value of classification.split(",")) {
      const normalized = value.trim()
      if (normalized) classifications.push(normalized)
    }
  }
  const normalizedClassifications = [...new Set(classifications)].sort()
  for (const classification of normalizedClassifications) {
    if (!isKnownDrillFailureClassification(classification)) {
      throw new Error(`unknown required matrix classification: ${classification}`)
    }
  }
  return normalizedClassifications
}

function normalizeRequiredDeploymentPresets(requiredDeploymentPresets) {
  if (!Array.isArray(requiredDeploymentPresets)) {
    throw new Error("requiredDeploymentPresets must be an array")
  }
  const presets = []
  for (const preset of requiredDeploymentPresets) {
    if (!nonEmptyString(preset)) {
      throw new Error("requiredDeploymentPresets has invalid preset")
    }
    for (const value of preset.split(",")) {
      const normalized = value.trim()
      if (normalized) presets.push(normalized)
    }
  }
  const normalizedPresets = [...new Set(presets)].sort()
  for (const preset of normalizedPresets) {
    if (!DRILL_DEPLOYMENT_PRESETS.includes(preset)) {
      throw new Error(`unknown required deployment preset: ${preset}`)
    }
  }
  return normalizedPresets
}

function normalizeRequiredProviders(requiredProviders) {
  if (!Array.isArray(requiredProviders)) {
    throw new Error("requiredProviders must be an array")
  }
  const providers = []
  for (const provider of requiredProviders) {
    if (!nonEmptyString(provider)) {
      throw new Error("requiredProviders has invalid provider")
    }
    for (const value of provider.split(",")) {
      const normalized = value.trim()
      if (normalized) providers.push(normalized)
    }
  }
  return [...new Set(providers)].sort()
}

function normalizeRequiredScenarios(requiredScenarios) {
  if (!Array.isArray(requiredScenarios)) {
    throw new Error("requiredScenarios must be an array")
  }
  const scenarios = []
  for (const scenario of requiredScenarios) {
    if (!nonEmptyString(scenario)) {
      throw new Error("requiredScenarios has invalid scenario")
    }
    for (const value of scenario.split(",")) {
      const normalized = value.trim()
      if (normalized) scenarios.push(normalized)
    }
  }
  return [...new Set(scenarios)].sort()
}

async function failureCheck({ failureInputs, failureRoots }, { maxDepth }) {
  if (failureRoots.length === 0 && failureInputs.length === 0) {
    return {
      status: "skipped",
      roots: [],
      inputs: [],
      manifestPaths: [],
    }
  }
  try {
    const discovered = failureRoots.length > 0
      ? await findDrillFailureManifestPaths(failureRoots, { maxDepth })
      : []
    const inputManifestPaths = await Promise.all(failureInputs.map((input) => resolveFailureManifestPath(input)))
    const manifestPaths = [...new Set([...inputManifestPaths, ...discovered])].sort()
    const manifests = await Promise.all(manifestPaths.map((manifestPath) => readDrillFailureManifest(manifestPath)))
    const aggregate = summarizeDrillFailureManifests(manifests, { sources: manifestPaths })
    return {
      status: aggregate.total === 0 ? "passed" : "failed",
      roots: [...failureRoots],
      inputs: [...failureInputs],
      manifestPaths,
      aggregate,
    }
  } catch (error) {
    return {
      status: "failed",
      roots: [...failureRoots],
      inputs: [...failureInputs],
      manifestPaths: [],
      error: error instanceof Error ? error.message : String(error),
    }
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

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}
