import {
  DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
  drillValidationGateAggregateExitCode,
  formatDrillValidationGateAggregateSummary,
  summarizeValidationGateReportAggregate,
  validateDrillValidationGateAggregate,
} from "./drill-validation-gate-aggregate.mjs"
import { artifactValidationGateCheck } from "./drill-validation-gate-artifact-check.mjs"
import { configurationValidationGateCheck } from "./drill-validation-gate-configuration-check.mjs"
import {
  findDrillValidationGateAggregatePaths,
  findDrillValidationGateReportPaths,
  readDrillValidationGateAggregate,
  readDrillValidationGateReport,
} from "./drill-validation-gate-discovery.mjs"
import { failureValidationGateCheck } from "./drill-validation-gate-failure-check.mjs"
import { matrixValidationGateCheck } from "./drill-validation-gate-matrix-check.mjs"
import { validationGateNextActions } from "./drill-validation-gate-next-actions.mjs"
import { platformValidationGateCheck } from "./drill-validation-gate-platform-check.mjs"
import {
  DRILL_VALIDATION_GATE_PRESETS,
  describeDrillValidationGatePresets,
  expandValidationGatePresetRequirements,
  normalizeRequiredDeploymentPresets,
  normalizeRequiredArtifactCoverageAreas,
  normalizeRequiredArtifactEvidenceRepos,
  normalizeRequiredArtifactGeneratedEvidenceKinds,
  normalizeRequiredArtifactGeneratedMatrixArtifactIndexes,
  normalizeRequiredArtifactGeneratedMatrixLimitations,
  normalizeRequiredArtifactGeneratedMatrixNames,
  normalizeRequiredArtifactGeneratedMatrixRepos,
  normalizeRequiredArtifactProviderAccountAliases,
  normalizeRequiredArtifactValidationPresets,
  normalizeRequiredArtifactClassifications,
  normalizeRequiredArtifactFailureClassifications,
  normalizeRequiredArtifactExitCriterionStatuses,
  normalizeRequiredArtifactIncompleteExitCriterionStatuses,
  normalizeRequiredArtifactKinds,
  normalizeRequiredArtifactOwners,
  normalizeRequiredArtifactPlannedClassifications,
  normalizeRequiredArtifactPlannedOwners,
  normalizeRequiredArtifactRuntimeSignalOwners,
  normalizeRequiredArtifactRuntimeSignals,
  normalizeRequiredArtifactSchemas,
  normalizeRequiredFailureClassifications,
  normalizeRequiredGeneratedEvidenceKinds,
  normalizeRequiredGeneratedMatrixArtifactIndexes,
  normalizeRequiredGeneratedMatrixLimitations,
  normalizeRequiredGeneratedValidationSuiteArtifactIndexes,
  normalizeRequiredGeneratedValidationSuiteFailureRoots,
  normalizeRequiredMatrices,
  normalizeRequiredMatrixClassifications,
  normalizeRequiredMatrixRuntimeSignals,
  normalizeRequiredPlatformCoverageAreas,
  normalizeRequiredPresets,
  normalizeRequiredProviders,
  normalizeRequiredRuntimeSignalOwners,
  normalizeRequiredRuntimeSignals,
  normalizeRequiredScenarios,
} from "./drill-validation-gate-presets.mjs"
import {
  DRILL_VALIDATION_GATE_SCHEMA,
  validateDrillValidationGateReport,
} from "./drill-validation-gate-report.mjs"
import { formatDrillValidationGateSummary } from "./drill-validation-gate-summary-format.mjs"

export {
  DRILL_VALIDATION_GATE_AGGREGATE_SCHEMA,
  DRILL_VALIDATION_GATE_PRESETS,
  DRILL_VALIDATION_GATE_SCHEMA,
  findDrillValidationGateAggregatePaths,
  findDrillValidationGateReportPaths,
  describeDrillValidationGatePresets,
  drillValidationGateAggregateExitCode,
  formatDrillValidationGateAggregateSummary,
  formatDrillValidationGateSummary,
  readDrillValidationGateAggregate,
  readDrillValidationGateReport,
  validateDrillValidationGateReport,
  validateDrillValidationGateAggregate,
}

export function summarizeDrillValidationGateReports(reports, options = {}) {
  const {
    sources = [],
    supplementalArtifactReports = [],
    supplementalArtifactSources = [],
    requiredPresets = [],
  } = options
  const normalizedRequiredPresets = normalizeRequiredPresets(requiredPresets)
  const normalizedAggregateRequirements = normalizeValidationGateAggregateRequirements(options)
  return summarizeValidationGateReportAggregate(reports, {
    sources,
    supplementalArtifactReports,
    supplementalArtifactSources,
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
  requiredArtifactCoverageAreas = [],
  requiredArtifactSchemas = [],
  requiredArtifactKinds = [],
  requiredArtifactGeneratedEvidenceKinds = [],
  requiredArtifactGeneratedMatrixArtifactIndexes = [],
  requiredArtifactGeneratedMatrixLimitations = [],
  requiredArtifactGeneratedMatrixNames = [],
  requiredArtifactGeneratedMatrixRepos = [],
  requiredArtifactEvidenceRepos = [],
  requiredArtifactProviderAccountAliases = [],
  requiredArtifactValidationPresets = [],
  requiredArtifactRuntimeSignals = [],
  requiredArtifactRuntimeSignalOwners = [],
  requiredArtifactOwners = [],
  requiredArtifactClassifications = [],
  requiredArtifactFailureClassifications = [],
  requiredArtifactPlannedOwners = [],
  requiredArtifactPlannedClassifications = [],
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
  suppressedPresetRequirements = [],
} = {}) {
  const normalizedPresets = normalizeRequiredPresets(presets)
  const expandedRequirements = expandValidationGatePresetRequirements({
    presets: normalizedPresets,
    requiredPlatformCoverageAreas,
    requiredArtifactCoverageAreas,
    requiredArtifactSchemas,
    requiredArtifactKinds,
    requiredArtifactGeneratedEvidenceKinds,
    requiredArtifactGeneratedMatrixArtifactIndexes,
    requiredArtifactGeneratedMatrixLimitations,
    requiredArtifactGeneratedMatrixNames,
    requiredArtifactGeneratedMatrixRepos,
    requiredArtifactEvidenceRepos,
    requiredArtifactProviderAccountAliases,
    requiredArtifactValidationPresets,
    requiredArtifactRuntimeSignals,
    requiredArtifactRuntimeSignalOwners,
    requiredArtifactOwners,
    requiredArtifactClassifications,
    requiredArtifactFailureClassifications,
    requiredArtifactPlannedOwners,
    requiredArtifactPlannedClassifications,
    requiredArtifactExitCriterionStatuses,
    requiredArtifactIncompleteExitCriterionStatuses,
    requiredRuntimeSignals,
    requiredRuntimeSignalOwners,
    requiredFailureClassifications,
    requiredMatrices,
    requiredMatrixClassifications,
    requiredMatrixRuntimeSignals,
    requiredDeploymentPresets,
    requiredProviders,
    requiredScenarios,
  })
  suppressPresetRequirements(expandedRequirements, {
    requiredMatrixClassifications,
  }, suppressedPresetRequirements)
  const normalizedRequiredPlatformCoverageAreas = normalizeRequiredPlatformCoverageAreas(expandedRequirements.requiredPlatformCoverageAreas)
  const normalizedRequiredArtifactCoverageAreas = normalizeRequiredArtifactCoverageAreas(expandedRequirements.requiredArtifactCoverageAreas)
  const normalizedRequiredArtifactSchemas = normalizeRequiredArtifactSchemas(expandedRequirements.requiredArtifactSchemas)
  const normalizedRequiredArtifactKinds = normalizeRequiredArtifactKinds(expandedRequirements.requiredArtifactKinds)
  const normalizedRequiredArtifactGeneratedEvidenceKinds = normalizeRequiredArtifactGeneratedEvidenceKinds(expandedRequirements.requiredArtifactGeneratedEvidenceKinds)
  const normalizedRequiredArtifactGeneratedMatrixArtifactIndexes = normalizeRequiredArtifactGeneratedMatrixArtifactIndexes(expandedRequirements.requiredArtifactGeneratedMatrixArtifactIndexes)
  const normalizedRequiredArtifactGeneratedMatrixLimitations = normalizeRequiredArtifactGeneratedMatrixLimitations(expandedRequirements.requiredArtifactGeneratedMatrixLimitations)
  const normalizedRequiredArtifactGeneratedMatrixNames = normalizeRequiredArtifactGeneratedMatrixNames(expandedRequirements.requiredArtifactGeneratedMatrixNames)
  const normalizedRequiredArtifactGeneratedMatrixRepos = normalizeRequiredArtifactGeneratedMatrixRepos(expandedRequirements.requiredArtifactGeneratedMatrixRepos)
  const normalizedRequiredArtifactEvidenceRepos = normalizeRequiredArtifactEvidenceRepos(expandedRequirements.requiredArtifactEvidenceRepos)
  const normalizedRequiredArtifactProviderAccountAliases = normalizeRequiredArtifactProviderAccountAliases(expandedRequirements.requiredArtifactProviderAccountAliases)
  const normalizedRequiredArtifactValidationPresets = normalizeRequiredArtifactValidationPresets(expandedRequirements.requiredArtifactValidationPresets)
  const normalizedRequiredArtifactRuntimeSignals = normalizeRequiredArtifactRuntimeSignals(expandedRequirements.requiredArtifactRuntimeSignals)
  const normalizedRequiredArtifactRuntimeSignalOwners = normalizeRequiredArtifactRuntimeSignalOwners(expandedRequirements.requiredArtifactRuntimeSignalOwners)
  const normalizedRequiredArtifactOwners = normalizeRequiredArtifactOwners(expandedRequirements.requiredArtifactOwners)
  const normalizedRequiredArtifactClassifications = normalizeRequiredArtifactClassifications(expandedRequirements.requiredArtifactClassifications)
  const normalizedRequiredArtifactFailureClassifications = normalizeRequiredArtifactFailureClassifications(expandedRequirements.requiredArtifactFailureClassifications)
  const normalizedRequiredArtifactPlannedOwners = normalizeRequiredArtifactPlannedOwners(expandedRequirements.requiredArtifactPlannedOwners)
  const normalizedRequiredArtifactPlannedClassifications = normalizeRequiredArtifactPlannedClassifications(expandedRequirements.requiredArtifactPlannedClassifications)
  const normalizedRequiredArtifactExitCriterionStatuses = normalizeRequiredArtifactExitCriterionStatuses(expandedRequirements.requiredArtifactExitCriterionStatuses)
  const normalizedRequiredArtifactIncompleteExitCriterionStatuses = normalizeRequiredArtifactIncompleteExitCriterionStatuses(expandedRequirements.requiredArtifactIncompleteExitCriterionStatuses)
  const normalizedRequiredRuntimeSignals = normalizeRequiredRuntimeSignals(expandedRequirements.requiredRuntimeSignals)
  const normalizedRequiredRuntimeSignalOwners = normalizeRequiredRuntimeSignalOwners(expandedRequirements.requiredRuntimeSignalOwners)
  const normalizedRequiredFailureClassifications = normalizeRequiredFailureClassifications(expandedRequirements.requiredFailureClassifications)
  const normalizedRequiredMatrices = normalizeRequiredMatrices(expandedRequirements.requiredMatrices)
  const normalizedRequiredMatrixClassifications = normalizeRequiredMatrixClassifications(expandedRequirements.requiredMatrixClassifications)
  const normalizedRequiredMatrixRuntimeSignals = normalizeRequiredMatrixRuntimeSignals(expandedRequirements.requiredMatrixRuntimeSignals)
  const normalizedRequiredDeploymentPresets = normalizeRequiredDeploymentPresets(expandedRequirements.requiredDeploymentPresets)
  const normalizedRequiredProviders = normalizeRequiredProviders(expandedRequirements.requiredProviders)
  const normalizedRequiredScenarios = normalizeRequiredScenarios(expandedRequirements.requiredScenarios)
  const checks = {
    configuration: configurationValidationGateCheck({
      artifactIndexes,
      artifactRoots,
      failureInputs,
      failureRoots,
      matrixReports,
      matrixRoots,
      platformBundleDir,
      requiredPlatformCoverageAreas: normalizedRequiredPlatformCoverageAreas,
      requiredArtifactCoverageAreas: normalizedRequiredArtifactCoverageAreas,
      requiredArtifactSchemas: normalizedRequiredArtifactSchemas,
      requiredArtifactKinds: normalizedRequiredArtifactKinds,
      requiredArtifactGeneratedEvidenceKinds: normalizedRequiredArtifactGeneratedEvidenceKinds,
      requiredArtifactGeneratedMatrixArtifactIndexes: normalizedRequiredArtifactGeneratedMatrixArtifactIndexes,
      requiredArtifactGeneratedMatrixLimitations: normalizedRequiredArtifactGeneratedMatrixLimitations,
      requiredArtifactGeneratedMatrixNames: normalizedRequiredArtifactGeneratedMatrixNames,
      requiredArtifactGeneratedMatrixRepos: normalizedRequiredArtifactGeneratedMatrixRepos,
      requiredArtifactEvidenceRepos: normalizedRequiredArtifactEvidenceRepos,
      requiredArtifactProviderAccountAliases: normalizedRequiredArtifactProviderAccountAliases,
      requiredArtifactValidationPresets: normalizedRequiredArtifactValidationPresets,
      requiredArtifactRuntimeSignals: normalizedRequiredArtifactRuntimeSignals,
      requiredArtifactRuntimeSignalOwners: normalizedRequiredArtifactRuntimeSignalOwners,
      requiredArtifactOwners: normalizedRequiredArtifactOwners,
      requiredArtifactClassifications: normalizedRequiredArtifactClassifications,
      requiredArtifactFailureClassifications: normalizedRequiredArtifactFailureClassifications,
      requiredArtifactPlannedOwners: normalizedRequiredArtifactPlannedOwners,
      requiredArtifactPlannedClassifications: normalizedRequiredArtifactPlannedClassifications,
      requiredArtifactExitCriterionStatuses: normalizedRequiredArtifactExitCriterionStatuses,
      requiredArtifactIncompleteExitCriterionStatuses: normalizedRequiredArtifactIncompleteExitCriterionStatuses,
      requiredArtifactMaxAgeMs,
      requiredFailureMaxAgeMs,
      requiredRuntimeSignals: normalizedRequiredRuntimeSignals,
      requiredRuntimeSignalOwners: normalizedRequiredRuntimeSignalOwners,
      requiredFailureClassifications: normalizedRequiredFailureClassifications,
      requiredMatrices: normalizedRequiredMatrices,
      requiredMatrixClassifications: normalizedRequiredMatrixClassifications,
      requiredMatrixRuntimeSignals: normalizedRequiredMatrixRuntimeSignals,
      requiredDeploymentPresets: normalizedRequiredDeploymentPresets,
      requiredProviders: normalizedRequiredProviders,
      requiredScenarios: normalizedRequiredScenarios,
      requiredMatrixMaxAgeMs,
    }),
    platformBundle: await platformValidationGateCheck(platformBundleDir, {
      requiredCoverageAreas: normalizedRequiredPlatformCoverageAreas,
      requiredRuntimeSignals: normalizedRequiredRuntimeSignals,
      requiredRuntimeSignalOwners: normalizedRequiredRuntimeSignalOwners,
      requiredFailureClassifications: normalizedRequiredFailureClassifications,
    }),
    artifacts: await artifactValidationGateCheck({ artifactIndexes, artifactRoots }, {
      maxDepth,
      requiredArtifactCoverageAreas: normalizedRequiredArtifactCoverageAreas,
      requiredArtifactSchemas: normalizedRequiredArtifactSchemas,
      requiredArtifactKinds: normalizedRequiredArtifactKinds,
      requiredArtifactGeneratedEvidenceKinds: normalizedRequiredArtifactGeneratedEvidenceKinds,
      requiredArtifactGeneratedMatrixArtifactIndexes: normalizedRequiredArtifactGeneratedMatrixArtifactIndexes,
      requiredArtifactGeneratedMatrixLimitations: normalizedRequiredArtifactGeneratedMatrixLimitations,
      requiredArtifactGeneratedMatrixNames: normalizedRequiredArtifactGeneratedMatrixNames,
      requiredArtifactGeneratedMatrixRepos: normalizedRequiredArtifactGeneratedMatrixRepos,
      requiredArtifactEvidenceRepos: normalizedRequiredArtifactEvidenceRepos,
      requiredArtifactProviderAccountAliases: normalizedRequiredArtifactProviderAccountAliases,
      requiredArtifactValidationPresets: normalizedRequiredArtifactValidationPresets,
      requiredArtifactRuntimeSignals: normalizedRequiredArtifactRuntimeSignals,
      requiredArtifactRuntimeSignalOwners: normalizedRequiredArtifactRuntimeSignalOwners,
      requiredArtifactOwners: normalizedRequiredArtifactOwners,
      requiredArtifactClassifications: normalizedRequiredArtifactClassifications,
      requiredArtifactFailureClassifications: normalizedRequiredArtifactFailureClassifications,
      requiredArtifactPlannedOwners: normalizedRequiredArtifactPlannedOwners,
      requiredArtifactPlannedClassifications: normalizedRequiredArtifactPlannedClassifications,
      requiredArtifactExitCriterionStatuses: normalizedRequiredArtifactExitCriterionStatuses,
      requiredArtifactIncompleteExitCriterionStatuses: normalizedRequiredArtifactIncompleteExitCriterionStatuses,
      requiredArtifactMaxAgeMs,
    }),
    matrices: await matrixValidationGateCheck({
      matrixReports,
      matrixRoots,
    }, {
      maxDepth,
      requireComplete,
      requiredMatrices: normalizedRequiredMatrices,
      requiredMatrixClassifications: normalizedRequiredMatrixClassifications,
      requiredMatrixRuntimeSignals: normalizedRequiredMatrixRuntimeSignals,
      requiredDeploymentPresets: normalizedRequiredDeploymentPresets,
      requiredProviders: normalizedRequiredProviders,
      requiredScenarios: normalizedRequiredScenarios,
      requiredMatrixMaxAgeMs,
    }),
    failures: await failureValidationGateCheck({ failureInputs, failureRoots }, {
      maxDepth,
      requiredFailureMaxAgeMs,
    }),
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

function suppressPresetRequirements(expandedRequirements, explicitRequirements, suppressedPresetRequirements) {
  if (!Array.isArray(suppressedPresetRequirements)) throw new Error("suppressedPresetRequirements must be an array")
  const suppressibleRequirements = new Set(Object.keys(explicitRequirements))
  for (const requirement of suppressedPresetRequirements) {
    if (typeof requirement !== "string" || !suppressibleRequirements.has(requirement)) {
      throw new Error(`unsupported suppressed preset requirement ${JSON.stringify(requirement)}`)
    }
    expandedRequirements[requirement] = [...(explicitRequirements[requirement] ?? [])]
  }
}

export function drillValidationGateExitCode(report) {
  validateDrillValidationGateReport(report)
  return report.status === "failed" ? 1 : 0
}

function normalizeValidationGateAggregateRequirements(options) {
  return {
    requiredPlatformCoverageAreas: normalizeRequiredPlatformCoverageAreas(options.requiredPlatformCoverageAreas ?? []),
    requiredArtifactCoverageAreas: normalizeRequiredArtifactCoverageAreas(options.requiredArtifactCoverageAreas ?? []),
    requiredArtifactSchemas: normalizeRequiredArtifactSchemas(options.requiredArtifactSchemas ?? []),
    requiredArtifactKinds: normalizeRequiredArtifactKinds(options.requiredArtifactKinds ?? []),
    requiredArtifactGeneratedEvidenceKinds: normalizeRequiredArtifactGeneratedEvidenceKinds(options.requiredArtifactGeneratedEvidenceKinds ?? []),
    requiredArtifactGeneratedMatrixArtifactIndexes: normalizeRequiredArtifactGeneratedMatrixArtifactIndexes(options.requiredArtifactGeneratedMatrixArtifactIndexes ?? []),
    requiredArtifactGeneratedMatrixLimitations: normalizeRequiredArtifactGeneratedMatrixLimitations(options.requiredArtifactGeneratedMatrixLimitations ?? []),
    requiredArtifactGeneratedMatrixNames: normalizeRequiredArtifactGeneratedMatrixNames(options.requiredArtifactGeneratedMatrixNames ?? []),
    requiredArtifactGeneratedMatrixRepos: normalizeRequiredArtifactGeneratedMatrixRepos(options.requiredArtifactGeneratedMatrixRepos ?? []),
    requiredArtifactEvidenceRepos: normalizeRequiredArtifactEvidenceRepos(options.requiredArtifactEvidenceRepos ?? []),
    requiredArtifactProviderAccountAliases: normalizeRequiredArtifactProviderAccountAliases(options.requiredArtifactProviderAccountAliases ?? []),
    requiredArtifactValidationPresets: normalizeRequiredArtifactValidationPresets(options.requiredArtifactValidationPresets ?? []),
    requiredArtifactRuntimeSignals: normalizeRequiredArtifactRuntimeSignals(options.requiredArtifactRuntimeSignals ?? []),
    requiredArtifactRuntimeSignalOwners: normalizeRequiredArtifactRuntimeSignalOwners(options.requiredArtifactRuntimeSignalOwners ?? []),
    requiredArtifactOwners: normalizeRequiredArtifactOwners(options.requiredArtifactOwners ?? []),
    requiredArtifactClassifications: normalizeRequiredArtifactClassifications(options.requiredArtifactClassifications ?? []),
    requiredArtifactFailureClassifications: normalizeRequiredArtifactFailureClassifications(options.requiredArtifactFailureClassifications ?? []),
    requiredArtifactPlannedOwners: normalizeRequiredArtifactPlannedOwners(options.requiredArtifactPlannedOwners ?? []),
    requiredArtifactPlannedClassifications: normalizeRequiredArtifactPlannedClassifications(options.requiredArtifactPlannedClassifications ?? []),
    requiredArtifactExitCriterionStatuses: normalizeRequiredArtifactExitCriterionStatuses(options.requiredArtifactExitCriterionStatuses ?? []),
    requiredArtifactIncompleteExitCriterionStatuses: normalizeRequiredArtifactIncompleteExitCriterionStatuses(options.requiredArtifactIncompleteExitCriterionStatuses ?? []),
    requiredRuntimeSignals: normalizeRequiredRuntimeSignals(options.requiredRuntimeSignals ?? []),
    requiredRuntimeSignalOwners: normalizeRequiredRuntimeSignalOwners(options.requiredRuntimeSignalOwners ?? []),
    requiredFailureClassifications: normalizeRequiredFailureClassifications(options.requiredFailureClassifications ?? []),
    requiredMatrices: normalizeRequiredMatrices(options.requiredMatrices ?? []),
    requiredMatrixClassifications: normalizeRequiredMatrixClassifications(options.requiredMatrixClassifications ?? []),
    requiredMatrixRuntimeSignals: normalizeRequiredMatrixRuntimeSignals(options.requiredMatrixRuntimeSignals ?? []),
    requiredDeploymentPresets: normalizeRequiredDeploymentPresets(options.requiredDeploymentPresets ?? []),
    requiredProviders: normalizeRequiredProviders(options.requiredProviders ?? []),
    requiredScenarios: normalizeRequiredScenarios(options.requiredScenarios ?? []),
    requiredGeneratedEvidenceKinds: normalizeRequiredGeneratedEvidenceKinds(options.requiredGeneratedEvidenceKinds ?? []),
    requiredGeneratedMatrixArtifactIndexes: normalizeRequiredGeneratedMatrixArtifactIndexes(options.requiredGeneratedMatrixArtifactIndexes ?? []),
    requiredGeneratedMatrixLimitations: normalizeRequiredGeneratedMatrixLimitations(options.requiredGeneratedMatrixLimitations ?? []),
    requiredGeneratedValidationSuiteArtifactIndexes: normalizeRequiredGeneratedValidationSuiteArtifactIndexes(options.requiredGeneratedValidationSuiteArtifactIndexes ?? []),
    requiredGeneratedValidationSuiteFailureRoots: normalizeRequiredGeneratedValidationSuiteFailureRoots(options.requiredGeneratedValidationSuiteFailureRoots ?? []),
  }
}
