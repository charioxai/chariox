export function configurationValidationGateCheck({
  artifactIndexes,
  artifactRoots,
  failureInputs,
  failureRoots,
  matrixReports,
  matrixRoots,
  platformBundleDir,
  requiredPlatformCoverageAreas = [],
  requiredRuntimeSignals = [],
  requiredFailureClassifications = [],
  requiredMatrices = [],
  requiredMatrixClassifications = [],
  requiredMatrixRuntimeSignals = [],
  requiredDeploymentPresets = [],
  requiredProviders = [],
  requiredScenarios = [],
}) {
  const configured = Boolean(platformBundleDir)
    || artifactRoots.length > 0
    || artifactIndexes.length > 0
    || matrixRoots.length > 0
    || matrixReports.length > 0
    || requiredPlatformCoverageAreas.length > 0
    || requiredRuntimeSignals.length > 0
    || requiredFailureClassifications.length > 0
    || failureRoots.length > 0
    || failureInputs.length > 0
    || requiredMatrices.length > 0
    || requiredMatrixClassifications.length > 0
    || requiredMatrixRuntimeSignals.length > 0
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
