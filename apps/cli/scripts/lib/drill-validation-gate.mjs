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
  normalizeRequiredArtifactClassifications,
  normalizeRequiredArtifactKinds,
  normalizeRequiredArtifactOwners,
  normalizeRequiredArtifactRuntimeSignalOwners,
  normalizeRequiredArtifactRuntimeSignals,
  normalizeRequiredArtifactSchemas,
  normalizeRequiredFailureClassifications,
  normalizeRequiredGeneratedEvidenceKinds,
  normalizeRequiredMatrices,
  normalizeRequiredMatrixClassifications,
  normalizeRequiredMatrixRuntimeSignals,
  normalizeRequiredPlatformCoverageAreas,
  normalizeRequiredPresets,
  normalizeRequiredProviders,
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
  const { sources = [], requiredPresets = [] } = options
  const normalizedRequiredPresets = normalizeRequiredPresets(requiredPresets)
  const normalizedAggregateRequirements = normalizeValidationGateAggregateRequirements(options)
  return summarizeValidationGateReportAggregate(reports, {
    sources,
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
  requiredArtifactEvidenceRepos = [],
  requiredArtifactRuntimeSignals = [],
  requiredArtifactRuntimeSignalOwners = [],
  requiredArtifactOwners = [],
  requiredArtifactClassifications = [],
  requiredRuntimeSignals = [],
  requiredFailureClassifications = [],
  requiredMatrices = [],
  requiredMatrixClassifications = [],
  requiredMatrixRuntimeSignals = [],
  requiredDeploymentPresets = [],
  requiredProviders = [],
  requiredScenarios = [],
} = {}) {
  const normalizedPresets = normalizeRequiredPresets(presets)
  const expandedRequirements = expandValidationGatePresetRequirements({
    presets: normalizedPresets,
    requiredPlatformCoverageAreas,
    requiredArtifactCoverageAreas,
    requiredArtifactSchemas,
    requiredArtifactKinds,
    requiredArtifactEvidenceRepos,
    requiredArtifactRuntimeSignals,
    requiredArtifactRuntimeSignalOwners,
    requiredArtifactOwners,
    requiredArtifactClassifications,
    requiredRuntimeSignals,
    requiredFailureClassifications,
    requiredMatrices,
    requiredMatrixClassifications,
    requiredMatrixRuntimeSignals,
    requiredDeploymentPresets,
    requiredProviders,
    requiredScenarios,
  })
  const normalizedRequiredPlatformCoverageAreas = normalizeRequiredPlatformCoverageAreas(expandedRequirements.requiredPlatformCoverageAreas)
  const normalizedRequiredArtifactCoverageAreas = normalizeRequiredArtifactCoverageAreas(expandedRequirements.requiredArtifactCoverageAreas)
  const normalizedRequiredArtifactSchemas = normalizeRequiredArtifactSchemas(expandedRequirements.requiredArtifactSchemas)
  const normalizedRequiredArtifactKinds = normalizeRequiredArtifactKinds(expandedRequirements.requiredArtifactKinds)
  const normalizedRequiredArtifactEvidenceRepos = normalizeRequiredArtifactEvidenceRepos(expandedRequirements.requiredArtifactEvidenceRepos)
  const normalizedRequiredArtifactRuntimeSignals = normalizeRequiredArtifactRuntimeSignals(expandedRequirements.requiredArtifactRuntimeSignals)
  const normalizedRequiredArtifactRuntimeSignalOwners = normalizeRequiredArtifactRuntimeSignalOwners(expandedRequirements.requiredArtifactRuntimeSignalOwners)
  const normalizedRequiredArtifactOwners = normalizeRequiredArtifactOwners(expandedRequirements.requiredArtifactOwners)
  const normalizedRequiredArtifactClassifications = normalizeRequiredArtifactClassifications(expandedRequirements.requiredArtifactClassifications)
  const normalizedRequiredRuntimeSignals = normalizeRequiredRuntimeSignals(expandedRequirements.requiredRuntimeSignals)
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
      requiredArtifactEvidenceRepos: normalizedRequiredArtifactEvidenceRepos,
      requiredArtifactRuntimeSignals: normalizedRequiredArtifactRuntimeSignals,
      requiredArtifactRuntimeSignalOwners: normalizedRequiredArtifactRuntimeSignalOwners,
      requiredArtifactOwners: normalizedRequiredArtifactOwners,
      requiredArtifactClassifications: normalizedRequiredArtifactClassifications,
      requiredRuntimeSignals: normalizedRequiredRuntimeSignals,
      requiredFailureClassifications: normalizedRequiredFailureClassifications,
      requiredMatrices: normalizedRequiredMatrices,
      requiredMatrixClassifications: normalizedRequiredMatrixClassifications,
      requiredMatrixRuntimeSignals: normalizedRequiredMatrixRuntimeSignals,
      requiredDeploymentPresets: normalizedRequiredDeploymentPresets,
      requiredProviders: normalizedRequiredProviders,
      requiredScenarios: normalizedRequiredScenarios,
    }),
    platformBundle: await platformValidationGateCheck(platformBundleDir, {
      requiredCoverageAreas: normalizedRequiredPlatformCoverageAreas,
      requiredRuntimeSignals: normalizedRequiredRuntimeSignals,
      requiredFailureClassifications: normalizedRequiredFailureClassifications,
    }),
    artifacts: await artifactValidationGateCheck({ artifactIndexes, artifactRoots }, {
      maxDepth,
      requiredArtifactCoverageAreas: normalizedRequiredArtifactCoverageAreas,
      requiredArtifactSchemas: normalizedRequiredArtifactSchemas,
      requiredArtifactKinds: normalizedRequiredArtifactKinds,
      requiredArtifactEvidenceRepos: normalizedRequiredArtifactEvidenceRepos,
      requiredArtifactRuntimeSignals: normalizedRequiredArtifactRuntimeSignals,
      requiredArtifactRuntimeSignalOwners: normalizedRequiredArtifactRuntimeSignalOwners,
      requiredArtifactOwners: normalizedRequiredArtifactOwners,
      requiredArtifactClassifications: normalizedRequiredArtifactClassifications,
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
    }),
    failures: await failureValidationGateCheck({ failureInputs, failureRoots }, { maxDepth }),
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
    requiredArtifactEvidenceRepos: normalizeRequiredArtifactEvidenceRepos(options.requiredArtifactEvidenceRepos ?? []),
    requiredArtifactRuntimeSignals: normalizeRequiredArtifactRuntimeSignals(options.requiredArtifactRuntimeSignals ?? []),
    requiredArtifactRuntimeSignalOwners: normalizeRequiredArtifactRuntimeSignalOwners(options.requiredArtifactRuntimeSignalOwners ?? []),
    requiredArtifactOwners: normalizeRequiredArtifactOwners(options.requiredArtifactOwners ?? []),
    requiredArtifactClassifications: normalizeRequiredArtifactClassifications(options.requiredArtifactClassifications ?? []),
    requiredRuntimeSignals: normalizeRequiredRuntimeSignals(options.requiredRuntimeSignals ?? []),
    requiredFailureClassifications: normalizeRequiredFailureClassifications(options.requiredFailureClassifications ?? []),
    requiredMatrices: normalizeRequiredMatrices(options.requiredMatrices ?? []),
    requiredMatrixClassifications: normalizeRequiredMatrixClassifications(options.requiredMatrixClassifications ?? []),
    requiredMatrixRuntimeSignals: normalizeRequiredMatrixRuntimeSignals(options.requiredMatrixRuntimeSignals ?? []),
    requiredDeploymentPresets: normalizeRequiredDeploymentPresets(options.requiredDeploymentPresets ?? []),
    requiredProviders: normalizeRequiredProviders(options.requiredProviders ?? []),
    requiredScenarios: normalizeRequiredScenarios(options.requiredScenarios ?? []),
    requiredGeneratedEvidenceKinds: normalizeRequiredGeneratedEvidenceKinds(options.requiredGeneratedEvidenceKinds ?? []),
  }
}
