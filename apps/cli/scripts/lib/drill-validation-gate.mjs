import {
  countDrillAggregateNextAction,
  formatDrillAggregateNextActionCounts,
  validateDrillAggregateNextAction,
} from "./drill-aggregate-actions.mjs"
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

export const DRILL_VALIDATION_GATE_SCHEMA = "arroba.drill.validation_gate.v1"

export async function runDrillValidationGate({
  failureInputs = [],
  failureRoots = [],
  matrixReports = [],
  matrixRoots = [],
  maxDepth = 8,
  platformBundleDir = null,
  requireComplete = false,
} = {}) {
  const checks = {
    configuration: configurationCheck({ failureInputs, failureRoots, matrixReports, matrixRoots, platformBundleDir }),
    platformBundle: await platformBundleCheck(platformBundleDir),
    matrices: await matrixCheck({ matrixReports, matrixRoots }, { maxDepth, requireComplete }),
    failures: await failureCheck({ failureInputs, failureRoots }, { maxDepth }),
  }
  const nextActions = validationGateNextActions(checks)
  const report = {
    schema: DRILL_VALIDATION_GATE_SCHEMA,
    status: Object.values(checks).some((check) => check.status === "failed") ? "failed" : "passed",
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
  const configuration = report.checks.configuration
  lines.push(`configuration=${configuration.status}${configuration.error ? ` error=${configuration.error}` : ""}`)

  const platform = report.checks.platformBundle
  lines.push(`platform_bundle=${platform.status}${platform.dir ? ` dir=${platform.dir}` : ""}${platform.error ? ` error=${platform.error}` : ""}`)

  const matrices = report.checks.matrices
  lines.push(`matrices=${matrices.status} roots=${matrices.roots.length} inputs=${matrices.inputs.length} reports=${matrices.reportPaths.length} require_complete=${matrices.requireComplete}`)
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
  if (!report.checks || typeof report.checks !== "object" || Array.isArray(report.checks)) {
    throw new Error(`${source} is missing checks`)
  }
  validateConfigurationCheck(report.checks.configuration, `${source}.checks.configuration`)
  validatePlatformBundleCheck(report.checks.platformBundle, `${source}.checks.platformBundle`)
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
  if (check.status === "skipped") {
    if (check.dir !== null) {
      throw new Error(`${source} skipped check has invalid dir`)
    }
    return
  }
  if (!nonEmptyString(check.dir)) {
    throw new Error(`${source} is missing dir`)
  }
  if (check.status === "failed") {
    if (!nonEmptyString(check.error)) {
      throw new Error(`${source} is missing error`)
    }
    return
  }
  if (!Array.isArray(check.artifacts)) {
    throw new Error(`${source} has invalid artifacts`)
  }
  for (const [index, artifact] of check.artifacts.entries()) {
    validatePlatformBundleArtifact(artifact, `${source}.artifacts[${index}]`)
  }
}

function validateMatrixCheck(check, source) {
  validateCheckObject(check, source)
  validateStringArray(check.roots, `${source}.roots`)
  validateStringArray(check.inputs, `${source}.inputs`)
  validateStringArray(check.reportPaths, `${source}.reportPaths`)
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

function validationGateNextActions(checks) {
  const counts = new Map()
  if (checks.configuration.status === "failed") {
    countDrillAggregateNextAction(counts, {
      owner: "validation-harness",
      classification: "validation-gate",
      nextAction: "configure at least one platform bundle, matrix root, or failure root before using the validation gate",
    })
  }
  if (checks.platformBundle.status === "failed") {
    countDrillAggregateNextAction(counts, {
      owner: "validation-harness",
      classification: "platform-bundle",
      nextAction: "rebuild the drill platform bundle and verify it before using collected artifacts as evidence",
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

function configurationCheck({ failureInputs, failureRoots, matrixReports, matrixRoots, platformBundleDir }) {
  const configured = Boolean(platformBundleDir)
    || matrixRoots.length > 0
    || matrixReports.length > 0
    || failureRoots.length > 0
    || failureInputs.length > 0
  return configured
    ? { status: "passed" }
    : {
        status: "failed",
        error: "no validation checks configured",
      }
}

async function platformBundleCheck(platformBundleDir) {
  if (!platformBundleDir) return { status: "skipped", dir: null }
  try {
    const bundle = await verifyDrillPlatformBundle(platformBundleDir)
    return {
      status: "passed",
      dir: platformBundleDir,
      artifacts: bundle.artifacts.map((artifact) => ({
        path: artifact.path,
        schema: artifact.schema,
        sha256: artifact.sha256,
        sizeBytes: artifact.sizeBytes,
      })),
    }
  } catch (error) {
    return {
      status: "failed",
      dir: platformBundleDir,
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

async function matrixCheck({ matrixReports, matrixRoots }, { maxDepth, requireComplete }) {
  if (matrixRoots.length === 0 && matrixReports.length === 0) {
    return {
      status: "skipped",
      roots: [],
      inputs: [],
      reportPaths: [],
      requireComplete,
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
        error: "no matrix reports found",
      }
    }
    const reports = await Promise.all(reportPaths.map((reportPath) => readDrillMatrixReport(reportPath)))
    const aggregate = summarizeDrillMatrixReports(reports, { sources: reportPaths })
    const exitCode = requireComplete
      ? drillMatrixReportCompletionExitCode(reports)
      : drillMatrixReportExitCode(reports)
    return {
      status: exitCode === 0 ? "passed" : "failed",
      roots: [...matrixRoots],
      inputs: [...matrixReports],
      reportPaths,
      requireComplete,
      aggregate,
    }
  } catch (error) {
    return {
      status: "failed",
      roots: [...matrixRoots],
      inputs: [...matrixReports],
      reportPaths: [],
      requireComplete,
      error: error instanceof Error ? error.message : String(error),
    }
  }
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

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}
