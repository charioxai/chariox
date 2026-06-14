import {
  findDrillFailureManifestPaths,
  readDrillFailureManifest,
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
  failureRoots = [],
  matrixRoots = [],
  maxDepth = 8,
  platformBundleDir = null,
  requireComplete = false,
} = {}) {
  const checks = {
    platformBundle: await platformBundleCheck(platformBundleDir),
    matrices: await matrixCheck(matrixRoots, { maxDepth, requireComplete }),
    failures: await failureCheck(failureRoots, { maxDepth }),
  }
  return {
    schema: DRILL_VALIDATION_GATE_SCHEMA,
    status: Object.values(checks).some((check) => check.status === "failed") ? "failed" : "passed",
    checks,
  }
}

export function drillValidationGateExitCode(report) {
  return report.status === "failed" ? 1 : 0
}

export function formatDrillValidationGateSummary(report) {
  const lines = [
    "drill validation gate:",
    `status=${report.status}`,
  ]
  const platform = report.checks.platformBundle
  lines.push(`platform_bundle=${platform.status}${platform.dir ? ` dir=${platform.dir}` : ""}${platform.error ? ` error=${platform.error}` : ""}`)

  const matrices = report.checks.matrices
  lines.push(`matrices=${matrices.status} roots=${matrices.roots.length} reports=${matrices.reportPaths.length} require_complete=${matrices.requireComplete}`)
  if (matrices.error) lines.push(`matrix_error=${matrices.error}`)
  if (matrices.aggregate) {
    lines.push(`matrix_status=${matrices.aggregate.status} failed=${matrices.aggregate.totals.failed} skipped=${matrices.aggregate.totals.skipped} dry_run=${matrices.aggregate.totals.dryRun}`)
  }

  const failures = report.checks.failures
  lines.push(`failures=${failures.status} roots=${failures.roots.length} manifests=${failures.manifestPaths.length}`)
  if (failures.error) lines.push(`failure_error=${failures.error}`)
  if (failures.aggregate) {
    lines.push(`failure_total=${failures.aggregate.total}`)
  }

  lines.push(report.status === "passed"
    ? "next: validation artifacts passed configured gates"
    : "next: inspect failed gate checks and rerun the relevant drills")
  return lines.join("\n")
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

async function matrixCheck(matrixRoots, { maxDepth, requireComplete }) {
  if (matrixRoots.length === 0) {
    return {
      status: "skipped",
      roots: [],
      reportPaths: [],
      requireComplete,
    }
  }
  try {
    const reportPaths = await findDrillMatrixReportPaths(matrixRoots, { maxDepth })
    if (reportPaths.length === 0) {
      return {
        status: "failed",
        roots: [...matrixRoots],
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
      reportPaths,
      requireComplete,
      aggregate,
    }
  } catch (error) {
    return {
      status: "failed",
      roots: [...matrixRoots],
      reportPaths: [],
      requireComplete,
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

async function failureCheck(failureRoots, { maxDepth }) {
  if (failureRoots.length === 0) {
    return {
      status: "skipped",
      roots: [],
      manifestPaths: [],
    }
  }
  try {
    const manifestPaths = await findDrillFailureManifestPaths(failureRoots, { maxDepth })
    const manifests = await Promise.all(manifestPaths.map((manifestPath) => readDrillFailureManifest(manifestPath)))
    const aggregate = summarizeDrillFailureManifests(manifests, { sources: manifestPaths })
    return {
      status: aggregate.total === 0 ? "passed" : "failed",
      roots: [...failureRoots],
      manifestPaths,
      aggregate,
    }
  } catch (error) {
    return {
      status: "failed",
      roots: [...failureRoots],
      manifestPaths: [],
      error: error instanceof Error ? error.message : String(error),
    }
  }
}
