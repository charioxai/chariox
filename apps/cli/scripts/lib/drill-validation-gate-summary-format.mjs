import { validateDrillValidationGateReport } from "./drill-validation-gate-report.mjs"
import { drillRuntimeSignalOwnerCounts } from "./drill-runtime-signals.mjs"

export function formatDrillValidationGateSummary(report) {
  validateDrillValidationGateReport(report)
  const lines = [
    "drill validation gate:",
    `status=${report.status}`,
  ]
  if ((report.presets ?? []).length > 0) {
    lines.push(`presets=${report.presets.join(",")}`)
  }
  appendGeneratedEvidenceSummary(lines, report.generatedEvidence)
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
  const requiredArtifactCoverageAreas = artifacts.requiredArtifactCoverageAreas ?? []
  const missingArtifactCoverageAreas = artifacts.missingArtifactCoverageAreas ?? []
  if (requiredArtifactCoverageAreas.length > 0) {
    lines.push(`artifact_required_coverage_areas=${requiredArtifactCoverageAreas.join(",")} missing=${missingArtifactCoverageAreas.join(",") || "none"}`)
  }
  const requiredArtifactSchemas = artifacts.requiredArtifactSchemas ?? []
  const missingArtifactSchemas = artifacts.missingArtifactSchemas ?? []
  if (requiredArtifactSchemas.length > 0) {
    lines.push(`artifact_required_schemas=${requiredArtifactSchemas.join(",")} missing=${missingArtifactSchemas.join(",") || "none"}`)
  }
  const requiredArtifactKinds = artifacts.requiredArtifactKinds ?? []
  const missingArtifactKinds = artifacts.missingArtifactKinds ?? []
  if (requiredArtifactKinds.length > 0) {
    lines.push(`artifact_required_kinds=${requiredArtifactKinds.join(",")} missing=${missingArtifactKinds.join(",") || "none"}`)
  }
  const requiredArtifactGeneratedEvidenceKinds = artifacts.requiredArtifactGeneratedEvidenceKinds ?? []
  const missingArtifactGeneratedEvidenceKinds = artifacts.missingArtifactGeneratedEvidenceKinds ?? []
  if (requiredArtifactGeneratedEvidenceKinds.length > 0) {
    lines.push(`artifact_required_generated_evidence_kinds=${requiredArtifactGeneratedEvidenceKinds.join(",")} missing=${missingArtifactGeneratedEvidenceKinds.join(",") || "none"}`)
  }
  const requiredArtifactGeneratedMatrixLimitations = artifacts.requiredArtifactGeneratedMatrixLimitations ?? []
  const missingArtifactGeneratedMatrixLimitations = artifacts.missingArtifactGeneratedMatrixLimitations ?? []
  if (requiredArtifactGeneratedMatrixLimitations.length > 0) {
    lines.push(`artifact_required_generated_matrix_limitations=${requiredArtifactGeneratedMatrixLimitations.join(",")} missing=${missingArtifactGeneratedMatrixLimitations.join(",") || "none"}`)
  }
  const requiredArtifactEvidenceRepos = artifacts.requiredArtifactEvidenceRepos ?? []
  const missingArtifactEvidenceRepos = artifacts.missingArtifactEvidenceRepos ?? []
  if (requiredArtifactEvidenceRepos.length > 0) {
    lines.push(`artifact_required_evidence_repos=${requiredArtifactEvidenceRepos.join(",")} missing=${missingArtifactEvidenceRepos.join(",") || "none"}`)
  }
  const requiredArtifactProviderAccountAliases = artifacts.requiredArtifactProviderAccountAliases ?? []
  const missingArtifactProviderAccountAliases = artifacts.missingArtifactProviderAccountAliases ?? []
  if (requiredArtifactProviderAccountAliases.length > 0) {
    lines.push(`artifact_required_provider_account_aliases=${requiredArtifactProviderAccountAliases.join(",")} missing=${missingArtifactProviderAccountAliases.join(",") || "none"}`)
  }
  const requiredArtifactRuntimeSignals = artifacts.requiredArtifactRuntimeSignals ?? []
  const missingArtifactRuntimeSignals = artifacts.missingArtifactRuntimeSignals ?? []
  if (requiredArtifactRuntimeSignals.length > 0) {
    lines.push(`artifact_required_runtime_signals=${requiredArtifactRuntimeSignals.join(",")} missing=${missingArtifactRuntimeSignals.join(",") || "none"}`)
  }
  const requiredArtifactRuntimeSignalOwners = artifacts.requiredArtifactRuntimeSignalOwners ?? []
  const missingArtifactRuntimeSignalOwners = artifacts.missingArtifactRuntimeSignalOwners ?? []
  if (requiredArtifactRuntimeSignalOwners.length > 0) {
    lines.push(`artifact_required_runtime_signal_owners=${requiredArtifactRuntimeSignalOwners.join(",")} missing=${missingArtifactRuntimeSignalOwners.join(",") || "none"}`)
  }
  const requiredArtifactOwners = artifacts.requiredArtifactOwners ?? []
  const missingArtifactOwners = artifacts.missingArtifactOwners ?? []
  if (requiredArtifactOwners.length > 0) {
    lines.push(`artifact_required_owners=${requiredArtifactOwners.join(",")} missing=${missingArtifactOwners.join(",") || "none"}`)
  }
  const requiredArtifactClassifications = artifacts.requiredArtifactClassifications ?? []
  const missingArtifactClassifications = artifacts.missingArtifactClassifications ?? []
  if (requiredArtifactClassifications.length > 0) {
    lines.push(`artifact_required_classifications=${requiredArtifactClassifications.join(",")} missing=${missingArtifactClassifications.join(",") || "none"}`)
  }
  if (artifacts.requiredArtifactMaxAgeMs !== undefined && artifacts.requiredArtifactMaxAgeMs !== null) {
    lines.push(`artifact_required_max_age_ms=${artifacts.requiredArtifactMaxAgeMs} stale_indexes=${(artifacts.staleArtifactIndexes ?? []).length}`)
    for (const staleIndex of artifacts.staleArtifactIndexes ?? []) {
      lines.push(`- stale_artifact_index=${staleIndex.source ?? "unknown"} created_at=${staleIndex.createdAt} age_ms=${staleIndex.ageMs} max_age_ms=${staleIndex.maxAgeMs}`)
    }
  }
  if (artifacts.error) lines.push(`artifact_error=${artifacts.error}`)
  if (artifacts.aggregate) {
    lines.push(`artifact_total=${artifacts.aggregate.totals.artifacts} size_bytes=${artifacts.aggregate.totals.sizeBytes}`)
    const artifactSchemas = Object.entries(artifacts.aggregate.schemas ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (artifactSchemas.length > 0) {
      lines.push(`artifact_schemas=${artifactSchemas.map(([schema, count]) => `${schema}:${count}`).join(",")}`)
    }
    const artifactCoverageAreas = Object.entries(artifacts.aggregate.coverageAreas ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (artifactCoverageAreas.length > 0) {
      lines.push(`artifact_coverage_areas=${artifactCoverageAreas.map(([area, count]) => `${area}:${count}`).join(",")}`)
    }
    const runtimeSignals = Object.entries(artifacts.aggregate.runtimeSignals ?? {})
    if (runtimeSignals.length > 0) {
      lines.push(`artifact_runtime_signals=${runtimeSignals.map(([signal, count]) => `${signal}:${count}`).join(",")}`)
    }
    const runtimeSignalOwners = Object.entries(artifacts.aggregate.runtimeSignalOwners ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (runtimeSignalOwners.length > 0) {
      lines.push(`artifact_runtime_signal_owners=${runtimeSignalOwners.map(([owner, count]) => `${owner}:${count}`).join(",")}`)
    }
    const owners = Object.entries(artifacts.aggregate.owners ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (owners.length > 0) {
      lines.push(`artifact_owners=${owners.map(([owner, count]) => `${owner}:${count}`).join(",")}`)
    }
    const classifications = Object.entries(artifacts.aggregate.classifications ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (classifications.length > 0) {
      lines.push(`artifact_classifications=${classifications.map(([classification, count]) => `${classification}:${count}`).join(",")}`)
    }
    const artifactKinds = Object.entries(artifacts.aggregate.artifactKinds ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (artifactKinds.length > 0) {
      lines.push(`artifact_kinds=${artifactKinds.map(([kind, count]) => `${kind}:${count}`).join(",")}`)
    }
    const generatedEvidenceKinds = Object.entries(artifacts.aggregate.generatedEvidenceKinds ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (generatedEvidenceKinds.length > 0) {
      lines.push(`artifact_generated_evidence_kinds=${generatedEvidenceKinds.map(([kind, count]) => `${kind}:${count}`).join(",")}`)
    }
    const generatedMatrixLimitations = Object.entries(artifacts.aggregate.generatedMatrixLimitations ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (generatedMatrixLimitations.length > 0) {
      lines.push(`artifact_generated_matrix_limitations=${generatedMatrixLimitations.map(([kind, count]) => `${kind}:${count}`).join(",")}`)
    }
    const generatedValidationSuiteFailureRoots = Object.entries(artifacts.aggregate.generatedValidationSuiteFailureRoots ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (generatedValidationSuiteFailureRoots.length > 0) {
      lines.push(`artifact_generated_validation_suite_failure_roots=${generatedValidationSuiteFailureRoots.map(([root, count]) => `${root}:${count}`).join(",")}`)
    }
    const requiredGeneratedEvidenceKinds = Object.entries(artifacts.aggregate.requiredGeneratedEvidenceKinds ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (requiredGeneratedEvidenceKinds.length > 0) {
      lines.push(`artifact_required_generated_evidence_kinds=${requiredGeneratedEvidenceKinds.map(([kind, count]) => `${kind}:${count}`).join(",")}`)
    }
    const missingGeneratedEvidenceKinds = Object.entries(artifacts.aggregate.missingGeneratedEvidenceKinds ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (missingGeneratedEvidenceKinds.length > 0) {
      lines.push(`artifact_missing_generated_evidence_kinds=${missingGeneratedEvidenceKinds.map(([kind, count]) => `${kind}:${count}`).join(",")}`)
    }
    const requiredGeneratedMatrixLimitations = Object.entries(artifacts.aggregate.requiredGeneratedMatrixLimitations ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (requiredGeneratedMatrixLimitations.length > 0) {
      lines.push(`artifact_required_generated_matrix_limitations=${requiredGeneratedMatrixLimitations.map(([kind, count]) => `${kind}:${count}`).join(",")}`)
    }
    const missingGeneratedMatrixLimitations = Object.entries(artifacts.aggregate.missingGeneratedMatrixLimitations ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (missingGeneratedMatrixLimitations.length > 0) {
      lines.push(`artifact_missing_generated_matrix_limitations=${missingGeneratedMatrixLimitations.map(([kind, count]) => `${kind}:${count}`).join(",")}`)
    }
    const providerAccountAliases = Object.entries(artifacts.aggregate.providerAccountAliases ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (providerAccountAliases.length > 0) {
      lines.push(`artifact_provider_account_aliases=${providerAccountAliases.map(([alias, count]) => `${alias}:${count}`).join(",")}`)
    }
    const evidenceRepos = Object.entries(artifacts.aggregate.evidenceRepos ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (evidenceRepos.length > 0) {
      lines.push(`artifact_evidence_repos=${evidenceRepos.map(([repo, count]) => `${repo}:${count}`).join(",")}`)
    }
    const artifactCoverageInputSources = Object.entries(artifacts.aggregate.artifactCoverageInputSources ?? {}).sort(([left], [right]) => left.localeCompare(right))
    if (artifactCoverageInputSources.length > 0) {
      lines.push(`artifact_coverage_input_sources=${artifactCoverageInputSources.map(([source, count]) => `${source}:${count}`).join(",")}`)
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
  if (matrices.requiredMatrixMaxAgeMs !== undefined && matrices.requiredMatrixMaxAgeMs !== null) {
    lines.push(`matrix_required_max_age_ms=${matrices.requiredMatrixMaxAgeMs} stale_reports=${(matrices.staleMatrixReports ?? []).length}`)
    for (const staleReport of matrices.staleMatrixReports ?? []) {
      lines.push(`- stale_matrix_report=${staleReport.source ?? "unknown"} matrix=${staleReport.matrix} completed_at=${staleReport.completedAt} age_ms=${staleReport.ageMs} max_age_ms=${staleReport.maxAgeMs}`)
    }
  }
  if (matrices.error) lines.push(`matrix_error=${matrices.error}`)
  if (matrices.aggregate) {
    lines.push(`matrix_status=${matrices.aggregate.status} failed=${matrices.aggregate.totals.failed} skipped=${matrices.aggregate.totals.skipped} dry_run=${matrices.aggregate.totals.dryRun}`)
    const runtimeSignals = Object.entries(matrices.aggregate.runtimeSignals ?? {})
    if (runtimeSignals.length > 0) {
      lines.push(`matrix_runtime_signals=${runtimeSignals.map(([signal, count]) => `${signal}:${count}`).join(",")}`)
      lines.push(`matrix_runtime_signal_owners=${formatCountObject(drillRuntimeSignalOwnerCounts(matrices.aggregate.runtimeSignals))}`)
    }
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
    if (failures.requiredFailureMaxAgeMs !== undefined && failures.requiredFailureMaxAgeMs !== null) {
      lines.push(`failure_required_max_age_ms=${failures.requiredFailureMaxAgeMs} stale_manifests=${(failures.staleFailureManifests ?? []).length}`)
      for (const staleFailure of failures.staleFailureManifests ?? []) {
        lines.push(`- stale_failure_manifest=${staleFailure.source ?? "unknown"} drill=${staleFailure.drill} failed_at=${staleFailure.failedAt} age_ms=${staleFailure.ageMs} max_age_ms=${staleFailure.maxAgeMs}`)
      }
    }
    const runtimeSignals = Object.entries(failures.aggregate.runtimeSignals ?? {})
    if (runtimeSignals.length > 0) {
      lines.push(`failure_runtime_signals=${runtimeSignals.map(([signal, count]) => `${signal}:${count}`).join(",")}`)
      lines.push(`failure_runtime_signal_owners=${formatCountObject(drillRuntimeSignalOwnerCounts(failures.aggregate.runtimeSignals))}`)
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

function appendGeneratedEvidenceSummary(lines, generatedEvidence) {
  if (!generatedEvidence || typeof generatedEvidence !== "object" || Array.isArray(generatedEvidence)) return
  const validationSuites = generatedEvidence.validationSuites
  if (validationSuites && typeof validationSuites === "object" && !Array.isArray(validationSuites)) {
    const outputRoots = stringArrayForSummary(validationSuites.outputRoots)
    const artifactIndexes = stringArrayForSummary(validationSuites.artifactIndexes)
    const commands = Array.isArray(validationSuites.commands) ? validationSuites.commands.length : 0
    const explicitFailureRoots = stringArrayForSummary(validationSuites.failureRoots)
    const failureRoots = explicitFailureRoots.length > 0
      ? explicitFailureRoots
      : Array.isArray(validationSuites.commands)
      ? validationSuites.commands
        .map((command) => command?.failureRoot)
        .filter((failureRoot) => typeof failureRoot === "string" && failureRoot.length > 0)
      : []
    lines.push([
      `generated_validation_suites=${validationSuites.enabled === true ? "enabled" : "disabled"}`,
      `output_roots=${outputRoots.join(",") || "none"}`,
      `artifact_indexes=${artifactIndexes.join(",") || "none"}`,
      `commands=${commands}`,
      `failure_roots=${failureRoots.join(",") || "none"}`,
    ].join(" "))
  }
  const matrixReports = generatedEvidence.matrixReports
  if (matrixReports && typeof matrixReports === "object" && !Array.isArray(matrixReports)) {
    const roots = stringArrayForSummary(matrixReports.roots)
    const artifactIndexes = stringArrayForSummary(matrixReports.artifactIndexes)
    const commands = Array.isArray(matrixReports.commands) ? matrixReports.commands.length : 0
    lines.push([
      `generated_matrices=${matrixReports.enabled === true ? "enabled" : "disabled"}`,
      `roots=${roots.join(",") || "none"}`,
      `artifact_indexes=${artifactIndexes.join(",") || "none"}`,
      `commands=${commands}`,
      `dry_run=${matrixReports.dryRun === true}`,
      `continue_on_failure=${matrixReports.continueOnFailure === true}`,
    ].join(" "))
    const limitations = Array.isArray(matrixReports.limitations)
      ? matrixReports.limitations.filter((limitation) => limitation && typeof limitation === "object" && !Array.isArray(limitation))
      : []
    if (limitations.length > 0) {
      lines.push("generated_matrix_limitations:")
      for (const limitation of limitations) {
        const kind = typeof limitation.kind === "string" && limitation.kind ? limitation.kind : "unknown"
        const owner = typeof limitation.owner === "string" && limitation.owner ? limitation.owner : "unknown"
        const nextAction = typeof limitation.nextAction === "string" && limitation.nextAction
          ? limitation.nextAction
          : "inspect generated matrix evidence and rerun the required live drills"
        lines.push(`- kind=${kind} owner=${owner}: ${nextAction}`)
      }
    }
  }
}

function stringArrayForSummary(value) {
  return Array.isArray(value)
    ? value.filter((item) => typeof item === "string" && item.length > 0)
    : []
}

function formatCountObject(counts) {
  return Object.entries(counts).map(([key, count]) => `${key}:${count}`).join(",")
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
