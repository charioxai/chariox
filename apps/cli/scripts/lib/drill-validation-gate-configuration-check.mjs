export function configurationValidationGateCheck({
  artifactIndexes,
  artifactRoots,
  failureInputs,
  failureRoots,
  matrixReports,
  matrixRoots,
  platformBundleDir,
  requiredPlatformCoverageAreas = [],
  requiredArtifactCoverageAreas = [],
  requiredArtifactSchemas = [],
  requiredArtifactKinds = [],
  requiredArtifactGeneratedEvidenceKinds = [],
  requiredArtifactGeneratedEvidenceRepos = [],
  requiredArtifactGeneratedMatrixArtifactIndexes = [],
  requiredArtifactGeneratedMatrixLimitations = [],
  requiredArtifactGeneratedMatrixNames = [],
  requiredArtifactGeneratedMatrixRepos = [],
  requiredArtifactGeneratedValidationSuiteArtifactIndexes = [],
  requiredArtifactGeneratedValidationSuiteFailureRoots = [],
  requiredArtifactEvidenceRepos = [],
  requiredArtifactProviderAccountAliases = [],
  requiredArtifactValidationPresets = [],
  requiredArtifactRuntimeSignals = [],
  requiredArtifactRuntimeSignalOwners = [],
  requiredArtifactOwners = [],
  requiredArtifactClassifications = [],
  requiredArtifactFailureClassifications = [],
  requiredArtifactExitCriterionStatuses = [],
  requiredArtifactIncompleteExitCriterionStatuses = [],
  requiredArtifactMaxAgeMs = null,
  requiredFailureMaxAgeMs = null,
  requiredRuntimeSignals = [],
  requiredRuntimeSignalOwners = [],
  requiredFailureClassifications = [],
  requiredMatrices = [],
  requiredMatrixClassifications = [],
  requiredMatrixRuntimeSignals = [],
  requiredDeploymentPresets = [],
  requiredProviders = [],
  requiredScenarios = [],
  requiredMatrixMaxAgeMs = null,
}) {
  const configured = Boolean(platformBundleDir)
    || artifactRoots.length > 0
    || artifactIndexes.length > 0
    || matrixRoots.length > 0
    || matrixReports.length > 0
    || requiredPlatformCoverageAreas.length > 0
    || requiredArtifactCoverageAreas.length > 0
    || requiredArtifactSchemas.length > 0
    || requiredArtifactKinds.length > 0
    || requiredArtifactGeneratedEvidenceKinds.length > 0
    || requiredArtifactGeneratedEvidenceRepos.length > 0
    || requiredArtifactGeneratedMatrixArtifactIndexes.length > 0
    || requiredArtifactGeneratedMatrixLimitations.length > 0
    || requiredArtifactGeneratedMatrixNames.length > 0
    || requiredArtifactGeneratedMatrixRepos.length > 0
    || requiredArtifactGeneratedValidationSuiteArtifactIndexes.length > 0
    || requiredArtifactGeneratedValidationSuiteFailureRoots.length > 0
    || requiredArtifactEvidenceRepos.length > 0
    || requiredArtifactProviderAccountAliases.length > 0
    || requiredArtifactValidationPresets.length > 0
    || requiredArtifactRuntimeSignals.length > 0
    || requiredArtifactRuntimeSignalOwners.length > 0
    || requiredArtifactOwners.length > 0
    || requiredArtifactClassifications.length > 0
    || requiredArtifactFailureClassifications.length > 0
    || requiredArtifactExitCriterionStatuses.length > 0
    || requiredArtifactIncompleteExitCriterionStatuses.length > 0
    || requiredArtifactMaxAgeMs !== null
    || requiredFailureMaxAgeMs !== null
    || requiredRuntimeSignals.length > 0
    || requiredRuntimeSignalOwners.length > 0
    || requiredFailureClassifications.length > 0
    || failureRoots.length > 0
    || failureInputs.length > 0
    || requiredMatrices.length > 0
    || requiredMatrixClassifications.length > 0
    || requiredMatrixRuntimeSignals.length > 0
    || requiredDeploymentPresets.length > 0
    || requiredProviders.length > 0
    || requiredScenarios.length > 0
    || requiredMatrixMaxAgeMs !== null
  return configured
    ? { status: "passed" }
    : {
        status: "failed",
        error: "no validation checks configured",
      }
}
