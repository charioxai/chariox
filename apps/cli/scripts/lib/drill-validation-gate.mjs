import { opendir, readFile, stat } from "node:fs/promises"
import path from "node:path"

import {
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  validateDrillAggregateNextAction,
} from "./drill-aggregate-actions.mjs"
import {
  DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
  drillValidationGateAggregateExitCode,
  formatDrillValidationGateAggregateSummary,
  summarizeValidationGateReportAggregate,
  validateDrillValidationGateAggregate,
} from "./drill-validation-gate-aggregate.mjs"
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
import {
  drillMatrixReportCompletionExitCode,
  drillMatrixReportExitCode,
  findDrillMatrixReportPaths,
  readDrillMatrixReport,
  summarizeDrillMatrixReports,
} from "./drill-matrix-report.mjs"
import { verifyDrillPlatformBundle } from "./drill-platform-bundle.mjs"
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

export const DRILL_VALIDATION_GATE_SCHEMA = "arroba.drill.validation_gate.v1"
export {
  DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
  DRILL_VALIDATION_GATE_PRESETS,
  describeDrillValidationGatePresets,
  drillValidationGateAggregateExitCode,
  formatDrillValidationGateAggregateSummary,
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
