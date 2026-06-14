import {
  drillMatrixReportCompletionExitCode,
  drillMatrixReportExitCode,
  findDrillMatrixReportPaths,
  readDrillMatrixReport,
  summarizeDrillMatrixReports,
} from "./drill-matrix-report.mjs"

export async function matrixValidationGateCheck({ matrixReports, matrixRoots }, {
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
