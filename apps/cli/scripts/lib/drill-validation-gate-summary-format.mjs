import { validateDrillValidationGateReport } from "./drill-validation-gate-report.mjs"

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
  const requiredRuntimeSignals = platform.requiredRuntimeSignals ?? []
  const missingRuntimeSignals = platform.missingRuntimeSignals ?? []
  if (requiredRuntimeSignals.length > 0) {
    lines.push(`platform_required_runtime_signals=${requiredRuntimeSignals.join(",")} missing=${missingRuntimeSignals.join(",") || "none"}`)
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
  if (platform.runtimeSignals) {
    lines.push(`platform_runtime_signals=${platform.runtimeSignals.map((signal) => `${signal.id}:${signal.owner}`).join(",")}`)
  }

  const artifacts = report.checks.artifacts
  lines.push(`artifacts=${artifacts.status} roots=${artifacts.roots.length} inputs=${artifacts.inputs.length} indexes=${artifacts.indexPaths.length}`)
  const requiredArtifactSchemas = artifacts.requiredArtifactSchemas ?? []
  const missingArtifactSchemas = artifacts.missingArtifactSchemas ?? []
  if (requiredArtifactSchemas.length > 0) {
    lines.push(`artifact_required_schemas=${requiredArtifactSchemas.join(",")} missing=${missingArtifactSchemas.join(",") || "none"}`)
  }
  if (artifacts.error) lines.push(`artifact_error=${artifacts.error}`)
  if (artifacts.aggregate) {
    lines.push(`artifact_total=${artifacts.aggregate.totals.artifacts} size_bytes=${artifacts.aggregate.totals.sizeBytes}`)
    const artifactSchemas = Object.entries(artifacts.aggregate.schemas ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (artifactSchemas.length > 0) {
      lines.push(`artifact_schemas=${artifactSchemas.map(([schema, count]) => `${schema}:${count}`).join(",")}`)
    }
    const runtimeSignals = Object.entries(artifacts.aggregate.runtimeSignals ?? {})
    if (runtimeSignals.length > 0) {
      lines.push(`artifact_runtime_signals=${runtimeSignals.map(([signal, count]) => `${signal}:${count}`).join(",")}`)
    }
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
  const requiredMatrixRuntimeSignals = matrices.requiredMatrixRuntimeSignals ?? []
  const missingMatrixRuntimeSignals = matrices.missingMatrixRuntimeSignals ?? []
  if (requiredMatrixRuntimeSignals.length > 0) {
    lines.push(`matrix_required_runtime_signals=${requiredMatrixRuntimeSignals.join(",")} missing=${missingMatrixRuntimeSignals.join(",") || "none"}`)
    appendMatrixRuntimeSignalSources(lines, matrices.aggregate?.runtimeSignalScenarios, requiredMatrixRuntimeSignals)
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
    const exitCriteria = Object.entries(matrices.aggregate.exitCriteria ?? {})
    if (exitCriteria.length > 0) {
      lines.push(`matrix_exit_criteria=${exitCriteria.map(([status, count]) => `${status}:${count}`).join(",")}`)
    }
    const incompleteExitCriteria = matrices.aggregate.incompleteExitCriteria ?? []
    if (incompleteExitCriteria.length > 0) {
      lines.push("matrix_incomplete_exit_criteria:")
      for (const criterion of incompleteExitCriteria) {
        lines.push(`- ${formatMatrixExitCriterion(criterion)}`)
      }
    }
  }

  const failures = report.checks.failures
  lines.push(`failures=${failures.status} roots=${failures.roots.length} inputs=${failures.inputs.length} manifests=${failures.manifestPaths.length}`)
  if (failures.error) lines.push(`failure_error=${failures.error}`)
  if (failures.aggregate) {
    lines.push(`failure_total=${failures.aggregate.total}`)
    const runtimeSignals = Object.entries(failures.aggregate.runtimeSignals ?? {})
    if (runtimeSignals.length > 0) {
      lines.push(`failure_runtime_signals=${runtimeSignals.map(([signal, count]) => `${signal}:${count}`).join(",")}`)
    }
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

function appendMatrixRuntimeSignalSources(lines, runtimeSignalScenarios, requiredMatrixRuntimeSignals) {
  const evidence = runtimeSignalScenarios && typeof runtimeSignalScenarios === "object" && !Array.isArray(runtimeSignalScenarios)
    ? runtimeSignalScenarios
    : {}
  lines.push("matrix_runtime_signal_sources:")
  for (const signal of requiredMatrixRuntimeSignals) {
    const scenarios = Array.isArray(evidence[signal]) ? evidence[signal] : []
    lines.push(`- ${signal}: ${scenarios.length > 0 ? scenarios.map(formatMatrixRuntimeSignalScenario).join(", ") : "missing"}`)
  }
}

function formatMatrixRuntimeSignalScenario(scenario) {
  if (!scenario || typeof scenario !== "object" || Array.isArray(scenario)) return "invalid"
  const matrix = typeof scenario.matrix === "string" && scenario.matrix ? scenario.matrix : "unknown-matrix"
  const id = typeof scenario.id === "string" && scenario.id ? scenario.id : "unknown-scenario"
  const status = typeof scenario.status === "string" && scenario.status ? scenario.status : "unknown"
  const source = typeof scenario.source === "string" && scenario.source ? ` source=${scenario.source}` : ""
  return `${matrix}/${id}(${status})${source}`
}

function formatMatrixExitCriterion(criterion) {
  if (!criterion || typeof criterion !== "object" || Array.isArray(criterion)) return "invalid"
  const matrix = typeof criterion.matrix === "string" && criterion.matrix ? criterion.matrix : "unknown-matrix"
  const scenario = typeof criterion.scenarioId === "string" && criterion.scenarioId ? criterion.scenarioId : "unknown-scenario"
  const id = typeof criterion.id === "string" && criterion.id ? criterion.id : "unknown-criterion"
  const status = typeof criterion.status === "string" && criterion.status ? criterion.status : "unknown"
  const reason = typeof criterion.reason === "string" && criterion.reason ? ` reason=${criterion.reason}` : ""
  const source = typeof criterion.source === "string" && criterion.source ? ` source=${criterion.source}` : ""
  const text = typeof criterion.criterion === "string" && criterion.criterion ? `: ${criterion.criterion}` : ""
  return `${matrix}/${scenario}/${id}(${status})${reason}${source}${text}`
}
