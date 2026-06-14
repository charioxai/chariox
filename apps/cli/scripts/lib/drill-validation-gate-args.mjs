const REQUIREMENT_FLAGS = Object.freeze([
  ["--require-platform-coverage-area", "requiredPlatformCoverageAreas"],
  ["--require-artifact-coverage-area", "requiredArtifactCoverageAreas"],
  ["--require-artifact-schema", "requiredArtifactSchemas"],
  ["--require-artifact-kind", "requiredArtifactKinds"],
  ["--require-artifact-evidence-repo", "requiredArtifactEvidenceRepos"],
  ["--require-artifact-runtime-signal", "requiredArtifactRuntimeSignals"],
  ["--require-artifact-runtime-signal-owner", "requiredArtifactRuntimeSignalOwners"],
  ["--require-artifact-owner", "requiredArtifactOwners"],
  ["--require-artifact-classification", "requiredArtifactClassifications"],
  ["--require-runtime-signal", "requiredRuntimeSignals"],
  ["--require-failure-classification", "requiredFailureClassifications"],
  ["--require-matrix", "requiredMatrices"],
  ["--require-matrix-classification", "requiredMatrixClassifications"],
  ["--require-matrix-runtime-signal", "requiredMatrixRuntimeSignals"],
  ["--require-deployment-preset", "requiredDeploymentPresets"],
  ["--require-provider", "requiredProviders"],
  ["--require-scenario", "requiredScenarios"],
  ["--require-generated-evidence-kind", "requiredGeneratedEvidenceKinds"],
])

export function validationGateRequirementOptionDefaults({ presetKey = "presets" } = {}) {
  return {
    [presetKey]: [],
    requiredPlatformCoverageAreas: [],
    requiredArtifactCoverageAreas: [],
    requiredArtifactSchemas: [],
    requiredArtifactKinds: [],
    requiredArtifactEvidenceRepos: [],
    requiredArtifactRuntimeSignals: [],
    requiredArtifactRuntimeSignalOwners: [],
    requiredArtifactOwners: [],
    requiredArtifactClassifications: [],
    requiredRuntimeSignals: [],
    requiredFailureClassifications: [],
    requiredMatrices: [],
    requiredMatrixClassifications: [],
    requiredMatrixRuntimeSignals: [],
    requiredDeploymentPresets: [],
    requiredProviders: [],
    requiredScenarios: [],
    requiredGeneratedEvidenceKinds: [],
  }
}

export function parseValidationGateRequirementArg(
  argv,
  index,
  options,
  { presetFlag = "--preset", presetKey = "presets" } = {},
) {
  const arg = argv[index]
  if (presetFlag && arg === presetFlag) {
    options[presetKey].push(requiredArgValue(argv, index, presetFlag))
    return index + 1
  }
  if (presetFlag && arg.startsWith(`${presetFlag}=`)) {
    options[presetKey].push(arg.slice(`${presetFlag}=`.length))
    return index
  }
  for (const [flag, key] of REQUIREMENT_FLAGS) {
    if (arg === flag) {
      options[key].push(requiredArgValue(argv, index, flag))
      return index + 1
    }
    if (arg.startsWith(`${flag}=`)) {
      options[key].push(arg.slice(`${flag}=`.length))
      return index
    }
  }
  return null
}

function requiredArgValue(argv, index, flag) {
  const value = argv[index + 1]
  if (!value || value.startsWith("--")) throw new Error(`${flag} requires a value`)
  return value
}
