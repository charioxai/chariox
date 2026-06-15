import { isKnownDrillGeneratedMatrixName } from "./drill-generated-matrix-names.mjs"
import { redactDrillSecretText } from "./drill-secrets.mjs"

const SECRET_SENSITIVE_REQUIREMENT_KEYS = new Set([
  "requiredArtifactGeneratedMatrixArtifactIndexes",
  "requiredArtifactPlannedClassifications",
  "requiredArtifactPlannedOwners",
  "requiredArtifactGeneratedMatrixNames",
  "requiredGeneratedMatrixArtifactIndexes",
  "requiredGeneratedValidationSuiteArtifactIndexes",
  "requiredGeneratedValidationSuiteFailureRoots",
])

const REQUIREMENT_FLAGS = Object.freeze([
  ["--require-platform-coverage-area", "requiredPlatformCoverageAreas"],
  ["--require-artifact-coverage-area", "requiredArtifactCoverageAreas"],
  ["--require-artifact-schema", "requiredArtifactSchemas"],
  ["--require-artifact-kind", "requiredArtifactKinds"],
  ["--require-artifact-generated-evidence-kind", "requiredArtifactGeneratedEvidenceKinds"],
  ["--require-artifact-generated-matrix-artifact-index", "requiredArtifactGeneratedMatrixArtifactIndexes"],
  ["--require-artifact-generated-matrix-limitation", "requiredArtifactGeneratedMatrixLimitations"],
  ["--require-artifact-generated-matrix-name", "requiredArtifactGeneratedMatrixNames"],
  ["--require-artifact-generated-matrix-repo", "requiredArtifactGeneratedMatrixRepos"],
  ["--require-artifact-evidence-repo", "requiredArtifactEvidenceRepos"],
  ["--require-artifact-provider-account-alias", "requiredArtifactProviderAccountAliases"],
  ["--require-artifact-validation-preset", "requiredArtifactValidationPresets"],
  ["--require-artifact-runtime-signal", "requiredArtifactRuntimeSignals"],
  ["--require-artifact-runtime-signal-owner", "requiredArtifactRuntimeSignalOwners"],
  ["--require-artifact-owner", "requiredArtifactOwners"],
  ["--require-artifact-classification", "requiredArtifactClassifications"],
  ["--require-artifact-planned-owner", "requiredArtifactPlannedOwners"],
  ["--require-artifact-planned-classification", "requiredArtifactPlannedClassifications"],
  ["--require-artifact-exit-criterion-status", "requiredArtifactExitCriterionStatuses"],
  ["--require-artifact-incomplete-exit-criterion-status", "requiredArtifactIncompleteExitCriterionStatuses"],
  ["--require-runtime-signal", "requiredRuntimeSignals"],
  ["--require-runtime-signal-owner", "requiredRuntimeSignalOwners"],
  ["--require-failure-classification", "requiredFailureClassifications"],
  ["--require-matrix", "requiredMatrices"],
  ["--require-matrix-classification", "requiredMatrixClassifications"],
  ["--require-matrix-runtime-signal", "requiredMatrixRuntimeSignals"],
  ["--require-deployment-preset", "requiredDeploymentPresets"],
  ["--require-provider", "requiredProviders"],
  ["--require-scenario", "requiredScenarios"],
  ["--require-generated-evidence-kind", "requiredGeneratedEvidenceKinds"],
  ["--require-generated-matrix-artifact-index", "requiredGeneratedMatrixArtifactIndexes"],
  ["--require-generated-matrix-limitation", "requiredGeneratedMatrixLimitations"],
  ["--require-generated-validation-suite-artifact-index", "requiredGeneratedValidationSuiteArtifactIndexes"],
  ["--require-generated-validation-suite-failure-root", "requiredGeneratedValidationSuiteFailureRoots"],
])

export function validationGateRequirementOptionDefaults({ presetKey = "presets" } = {}) {
  return {
    [presetKey]: [],
    requiredPlatformCoverageAreas: [],
    requiredArtifactCoverageAreas: [],
    requiredArtifactSchemas: [],
    requiredArtifactKinds: [],
    requiredArtifactGeneratedEvidenceKinds: [],
    requiredArtifactGeneratedMatrixArtifactIndexes: [],
    requiredArtifactGeneratedMatrixLimitations: [],
    requiredArtifactGeneratedMatrixNames: [],
    requiredArtifactGeneratedMatrixRepos: [],
    requiredArtifactEvidenceRepos: [],
    requiredArtifactProviderAccountAliases: [],
    requiredArtifactValidationPresets: [],
    requiredArtifactRuntimeSignals: [],
    requiredArtifactRuntimeSignalOwners: [],
    requiredArtifactOwners: [],
    requiredArtifactClassifications: [],
    requiredArtifactPlannedOwners: [],
    requiredArtifactPlannedClassifications: [],
    requiredArtifactExitCriterionStatuses: [],
    requiredArtifactIncompleteExitCriterionStatuses: [],
    requiredRuntimeSignals: [],
    requiredRuntimeSignalOwners: [],
    requiredFailureClassifications: [],
    requiredMatrices: [],
    requiredMatrixClassifications: [],
    requiredMatrixRuntimeSignals: [],
    requiredDeploymentPresets: [],
    requiredProviders: [],
    requiredScenarios: [],
    requiredGeneratedEvidenceKinds: [],
    requiredGeneratedMatrixArtifactIndexes: [],
    requiredGeneratedMatrixLimitations: [],
    requiredGeneratedValidationSuiteArtifactIndexes: [],
    requiredGeneratedValidationSuiteFailureRoots: [],
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
      options[key].push(normalizedRequirementValue(requiredArgValue(argv, index, flag), flag, key))
      return index + 1
    }
    if (arg.startsWith(`${flag}=`)) {
      options[key].push(normalizedRequirementValue(arg.slice(`${flag}=`.length), flag, key))
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

function normalizedRequirementValue(value, flag, key) {
  if (!SECRET_SENSITIVE_REQUIREMENT_KEYS.has(key)) return value
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${flag} requires a value`)
  }
  if (redactDrillSecretText(value) !== value) {
    throw new Error(`${flag} includes secret-looking diagnostic text`)
  }
  if (key === "requiredArtifactGeneratedMatrixNames" && !isKnownDrillGeneratedMatrixName(value)) {
    throw new Error(`${flag} has unknown generated matrix name: ${value}`)
  }
  return value
}
