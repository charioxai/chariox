import { validateDrillAggregateNextAction } from "./drill-aggregate-actions.mjs"
import { isKnownDrillArtifactKind } from "./drill-artifact-kinds.mjs"
import { validateDrillArtifactIndexAggregate } from "./drill-artifacts.mjs"
import { isKnownDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import { isKnownDrillDeploymentPreset } from "./drill-environment-presets.mjs"
import { isKnownDrillFailureClassification } from "./drill-failure-taxonomy.mjs"
import { validateDrillFailureManifestAggregate } from "./drill-failure-manifest.mjs"
import { isKnownDrillGeneratedEvidenceKind } from "./drill-generated-evidence-kinds.mjs"
import { isKnownDrillGeneratedMatrixLimitation } from "./drill-generated-matrix-limitations.mjs"
import { validateDrillMatrixAggregate } from "./drill-matrix-report.mjs"
import {
  isKnownDrillProvider,
  parseProviderAccountAlias,
} from "./drill-provider-profiles.mjs"
import { isKnownDrillValidationGatePreset } from "./drill-validation-gate-presets.mjs"
import {
  DRILL_RUNTIME_SIGNAL_OWNERS,
  validateDrillRuntimeSignals,
} from "./drill-runtime-signals.mjs"

export const DRILL_VALIDATION_GATE_SCHEMA = "arroba.drill.validation_gate.v1"

export function validateDrillValidationGateReport(report, source = "validation gate report") {
  if (!report || typeof report !== "object" || Array.isArray(report)) {
    throw new Error(`${source} is not an object`)
  }
  if (report.schema !== DRILL_VALIDATION_GATE_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(report.schema)}`)
  }
  if (!["passed", "failed"].includes(report.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(report.status)}`)
  }
  validatePresetArray(report.presets ?? [], `${source}.presets`)
  if (!report.checks || typeof report.checks !== "object" || Array.isArray(report.checks)) {
    throw new Error(`${source} is missing checks`)
  }
  validateConfigurationCheck(report.checks.configuration, `${source}.checks.configuration`)
  validatePlatformBundleCheck(report.checks.platformBundle, `${source}.checks.platformBundle`)
  validateArtifactIndexCheck(report.checks.artifacts, `${source}.checks.artifacts`)
  validateMatrixCheck(report.checks.matrices, `${source}.checks.matrices`)
  validateFailureCheck(report.checks.failures, `${source}.checks.failures`)
  if (!Array.isArray(report.nextActions)) {
    throw new Error(`${source} has invalid nextActions`)
  }
  for (const [index, action] of report.nextActions.entries()) {
    validateDrillAggregateNextAction(action, `${source}.nextActions[${index}]`)
  }
  if (report.generatedEvidence !== undefined) {
    validateGeneratedEvidence(report.generatedEvidence, `${source}.generatedEvidence`)
  }
  const expectedStatus = Object.values(report.checks).some((check) => check.status === "failed") ? "failed" : "passed"
  if (report.status !== expectedStatus) {
    throw new Error(`${source} status does not match check statuses`)
  }
}

function validateConfigurationCheck(check, source) {
  validateCheckObject(check, source)
  if (check.status === "skipped") {
    throw new Error(`${source} cannot be skipped`)
  }
  if (check.status === "failed" && !nonEmptyString(check.error)) {
    throw new Error(`${source} is missing error`)
  }
}

function validatePlatformBundleCheck(check, source) {
  validateCheckObject(check, source)
  validateStringArray(check.requiredCoverageAreas ?? [], `${source}.requiredCoverageAreas`)
  validateStringArray(check.missingCoverageAreas ?? [], `${source}.missingCoverageAreas`)
  validateRuntimeSignalArray(check.requiredRuntimeSignals ?? [], `${source}.requiredRuntimeSignals`)
  validateRuntimeSignalArray(check.missingRuntimeSignals ?? [], `${source}.missingRuntimeSignals`)
  validateFailureClassificationArray(check.requiredFailureClassifications ?? [], `${source}.requiredFailureClassifications`)
  validateFailureClassificationArray(check.missingFailureClassifications ?? [], `${source}.missingFailureClassifications`)
  if (check.status === "skipped") {
    if (check.dir !== null) {
      throw new Error(`${source} skipped check has invalid dir`)
    }
    if ((check.requiredCoverageAreas ?? []).length > 0
      || (check.missingCoverageAreas ?? []).length > 0
      || (check.requiredRuntimeSignals ?? []).length > 0
      || (check.missingRuntimeSignals ?? []).length > 0
      || (check.requiredFailureClassifications ?? []).length > 0
      || (check.missingFailureClassifications ?? []).length > 0) {
      throw new Error(`${source} skipped check has invalid coverage requirements`)
    }
    return
  }
  if (check.status === "failed") {
    if (!nonEmptyString(check.error)) {
      throw new Error(`${source} is missing error`)
    }
    if (!check.validationSuite) {
      if (check.dir !== null && check.dir !== undefined && !nonEmptyString(check.dir)) {
        throw new Error(`${source} has invalid dir`)
      }
      return
    }
  }
  if (!nonEmptyString(check.dir)) {
    throw new Error(`${source} is missing dir`)
  }
  if (!Array.isArray(check.artifacts)) {
    throw new Error(`${source} has invalid artifacts`)
  }
  for (const [index, artifact] of check.artifacts.entries()) {
    validatePlatformBundleArtifact(artifact, `${source}.artifacts[${index}]`)
  }
  validatePlatformValidationSuiteSummary(check.validationSuite, `${source}.validationSuite`)
  if (check.failureTaxonomy !== undefined) {
    validatePlatformFailureTaxonomySummary(check.failureTaxonomy, `${source}.failureTaxonomy`)
  }
  if (check.runtimeSignals !== undefined) {
    validatePlatformRuntimeSignalsSummary(check.runtimeSignals, `${source}.runtimeSignals`)
  }
}

function validateArtifactIndexCheck(check, source) {
  validateCheckObject(check, source)
  validateStringArray(check.roots, `${source}.roots`)
  validateStringArray(check.inputs, `${source}.inputs`)
  validateStringArray(check.indexPaths, `${source}.indexPaths`)
  validateStringArray(check.requiredArtifactCoverageAreas ?? [], `${source}.requiredArtifactCoverageAreas`)
  validateStringArray(check.missingArtifactCoverageAreas ?? [], `${source}.missingArtifactCoverageAreas`)
  validateStringArray(check.requiredArtifactSchemas ?? [], `${source}.requiredArtifactSchemas`)
  validateStringArray(check.missingArtifactSchemas ?? [], `${source}.missingArtifactSchemas`)
  validateArtifactKindArray(check.requiredArtifactKinds ?? [], `${source}.requiredArtifactKinds`)
  validateArtifactKindArray(check.missingArtifactKinds ?? [], `${source}.missingArtifactKinds`)
  validateGeneratedEvidenceKindArray(check.requiredArtifactGeneratedEvidenceKinds ?? [], `${source}.requiredArtifactGeneratedEvidenceKinds`)
  validateGeneratedEvidenceKindArray(check.missingArtifactGeneratedEvidenceKinds ?? [], `${source}.missingArtifactGeneratedEvidenceKinds`)
  validateStringArray(check.requiredArtifactGeneratedMatrixArtifactIndexes ?? [], `${source}.requiredArtifactGeneratedMatrixArtifactIndexes`)
  validateStringArray(check.missingArtifactGeneratedMatrixArtifactIndexes ?? [], `${source}.missingArtifactGeneratedMatrixArtifactIndexes`)
  validateGeneratedMatrixLimitationArray(check.requiredArtifactGeneratedMatrixLimitations ?? [], `${source}.requiredArtifactGeneratedMatrixLimitations`)
  validateGeneratedMatrixLimitationArray(check.missingArtifactGeneratedMatrixLimitations ?? [], `${source}.missingArtifactGeneratedMatrixLimitations`)
  validateArtifactEvidenceRepoArray(check.requiredArtifactEvidenceRepos ?? [], `${source}.requiredArtifactEvidenceRepos`)
  validateArtifactEvidenceRepoArray(check.missingArtifactEvidenceRepos ?? [], `${source}.missingArtifactEvidenceRepos`)
  validateProviderAccountAliasArray(check.requiredArtifactProviderAccountAliases ?? [], `${source}.requiredArtifactProviderAccountAliases`)
  validateProviderAccountAliasArray(check.missingArtifactProviderAccountAliases ?? [], `${source}.missingArtifactProviderAccountAliases`)
  validateRuntimeSignalArray(check.requiredArtifactRuntimeSignals ?? [], `${source}.requiredArtifactRuntimeSignals`)
  validateRuntimeSignalArray(check.missingArtifactRuntimeSignals ?? [], `${source}.missingArtifactRuntimeSignals`)
  validateRuntimeSignalOwnerArray(check.requiredArtifactRuntimeSignalOwners ?? [], `${source}.requiredArtifactRuntimeSignalOwners`)
  validateRuntimeSignalOwnerArray(check.missingArtifactRuntimeSignalOwners ?? [], `${source}.missingArtifactRuntimeSignalOwners`)
  validateStringArray(check.requiredArtifactOwners ?? [], `${source}.requiredArtifactOwners`)
  validateStringArray(check.missingArtifactOwners ?? [], `${source}.missingArtifactOwners`)
  validateStringArray(check.requiredArtifactClassifications ?? [], `${source}.requiredArtifactClassifications`)
  validateStringArray(check.missingArtifactClassifications ?? [], `${source}.missingArtifactClassifications`)
  validateExitCriterionStatusArray(check.requiredArtifactExitCriterionStatuses ?? [], `${source}.requiredArtifactExitCriterionStatuses`)
  validateExitCriterionStatusArray(check.missingArtifactExitCriterionStatuses ?? [], `${source}.missingArtifactExitCriterionStatuses`)
  validateExitCriterionStatusArray(check.requiredArtifactIncompleteExitCriterionStatuses ?? [], `${source}.requiredArtifactIncompleteExitCriterionStatuses`)
  validateExitCriterionStatusArray(check.missingArtifactIncompleteExitCriterionStatuses ?? [], `${source}.missingArtifactIncompleteExitCriterionStatuses`)
  if (check.requiredArtifactMaxAgeMs !== undefined
    && check.requiredArtifactMaxAgeMs !== null
    && (!Number.isSafeInteger(check.requiredArtifactMaxAgeMs) || check.requiredArtifactMaxAgeMs < 0)) {
    throw new Error(`${source} has invalid requiredArtifactMaxAgeMs`)
  }
  if (check.staleArtifactIndexes !== undefined) {
    validateStaleArtifactIndexes(check.staleArtifactIndexes, `${source}.staleArtifactIndexes`)
  }
  if (check.status === "failed" && !check.aggregate && !nonEmptyString(check.error)) {
    throw new Error(`${source} is missing error`)
  }
  if (check.aggregate) {
    validateDrillArtifactIndexAggregate(check.aggregate, `${source}.aggregate`)
  }
}

function validateStaleArtifactIndexes(indexes, source) {
  if (!Array.isArray(indexes)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, entry] of indexes.entries()) {
    const entrySource = `${source}[${index}]`
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
      throw new Error(`${entrySource} is not an object`)
    }
    if (entry.source !== null && typeof entry.source !== "string") {
      throw new Error(`${entrySource} has invalid source`)
    }
    if (!nonEmptyString(entry.createdAt)) {
      throw new Error(`${entrySource} has invalid createdAt`)
    }
    if (!Number.isSafeInteger(entry.ageMs) || entry.ageMs < 0) {
      throw new Error(`${entrySource} has invalid ageMs`)
    }
    if (!Number.isSafeInteger(entry.maxAgeMs) || entry.maxAgeMs < 0) {
      throw new Error(`${entrySource} has invalid maxAgeMs`)
    }
  }
}

function validateMatrixCheck(check, source) {
  validateCheckObject(check, source)
  validateStringArray(check.roots, `${source}.roots`)
  validateStringArray(check.inputs, `${source}.inputs`)
  validateStringArray(check.reportPaths, `${source}.reportPaths`)
  validateStringArray(check.requiredMatrices ?? [], `${source}.requiredMatrices`)
  validateStringArray(check.missingMatrices ?? [], `${source}.missingMatrices`)
  validateFailureClassificationArray(check.requiredMatrixClassifications ?? [], `${source}.requiredMatrixClassifications`)
  validateFailureClassificationArray(check.missingMatrixClassifications ?? [], `${source}.missingMatrixClassifications`)
  validateRuntimeSignalArray(check.requiredMatrixRuntimeSignals ?? [], `${source}.requiredMatrixRuntimeSignals`)
  validateRuntimeSignalArray(check.missingMatrixRuntimeSignals ?? [], `${source}.missingMatrixRuntimeSignals`)
  validateDeploymentPresetArray(check.requiredDeploymentPresets ?? [], `${source}.requiredDeploymentPresets`)
  validateDeploymentPresetArray(check.missingDeploymentPresets ?? [], `${source}.missingDeploymentPresets`)
  validateProviderArray(check.requiredProviders ?? [], `${source}.requiredProviders`)
  validateProviderArray(check.missingProviders ?? [], `${source}.missingProviders`)
  validateStringArray(check.requiredScenarios ?? [], `${source}.requiredScenarios`)
  validateStringArray(check.missingScenarios ?? [], `${source}.missingScenarios`)
  if (typeof check.requireComplete !== "boolean") {
    throw new Error(`${source} has invalid requireComplete`)
  }
  if (check.requiredMatrixMaxAgeMs !== undefined
    && check.requiredMatrixMaxAgeMs !== null
    && (!Number.isSafeInteger(check.requiredMatrixMaxAgeMs) || check.requiredMatrixMaxAgeMs < 0)) {
    throw new Error(`${source} has invalid requiredMatrixMaxAgeMs`)
  }
  if (check.staleMatrixReports !== undefined) {
    validateStaleMatrixReports(check.staleMatrixReports, `${source}.staleMatrixReports`)
  }
  if (check.status === "failed" && !check.aggregate && !nonEmptyString(check.error)) {
    throw new Error(`${source} is missing error`)
  }
  if (check.aggregate) {
    try {
      validateDrillMatrixAggregate(check.aggregate)
    } catch (error) {
      const message = String(error.message ?? error).replace(/^aggregate\s+/, "")
      throw new Error(`${source}.aggregate ${message}`)
    }
  }
}

function validateStaleMatrixReports(reports, source) {
  if (!Array.isArray(reports)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, entry] of reports.entries()) {
    const entrySource = `${source}[${index}]`
    if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
      throw new Error(`${entrySource} is not an object`)
    }
    if (entry.source !== null && typeof entry.source !== "string") {
      throw new Error(`${entrySource} has invalid source`)
    }
    if (!nonEmptyString(entry.matrix)) {
      throw new Error(`${entrySource} has invalid matrix`)
    }
    if (!nonEmptyString(entry.completedAt)) {
      throw new Error(`${entrySource} has invalid completedAt`)
    }
    if (!Number.isSafeInteger(entry.ageMs) || entry.ageMs < 0) {
      throw new Error(`${entrySource} has invalid ageMs`)
    }
    if (!Number.isSafeInteger(entry.maxAgeMs) || entry.maxAgeMs < 0) {
      throw new Error(`${entrySource} has invalid maxAgeMs`)
    }
  }
}

function validateFailureCheck(check, source) {
  validateCheckObject(check, source)
  validateStringArray(check.roots, `${source}.roots`)
  validateStringArray(check.inputs, `${source}.inputs`)
  validateStringArray(check.manifestPaths, `${source}.manifestPaths`)
  if (check.requiredFailureMaxAgeMs !== undefined
    && check.requiredFailureMaxAgeMs !== null
    && (!Number.isSafeInteger(check.requiredFailureMaxAgeMs) || check.requiredFailureMaxAgeMs < 0)) {
    throw new Error(`${source} has invalid requiredFailureMaxAgeMs`)
  }
  if (check.staleFailureManifests !== undefined) {
    validateStaleFailureManifests(check.staleFailureManifests, `${source}.staleFailureManifests`)
  }
  if (check.status === "failed" && !check.aggregate && !nonEmptyString(check.error)) {
    throw new Error(`${source} is missing error`)
  }
  if (check.aggregate) {
    try {
      validateDrillFailureManifestAggregate(check.aggregate)
    } catch (error) {
      const message = String(error.message ?? error).replace(/^aggregate\s+/, "")
      throw new Error(`${source}.aggregate ${message}`)
    }
  }
}

function validateStaleFailureManifests(staleFailures, source) {
  if (!Array.isArray(staleFailures)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, staleFailure] of staleFailures.entries()) {
    const entrySource = `${source}[${index}]`
    if (!staleFailure || typeof staleFailure !== "object" || Array.isArray(staleFailure)) {
      throw new Error(`${entrySource} is not an object`)
    }
    if (staleFailure.source !== null && staleFailure.source !== undefined && !nonEmptyString(staleFailure.source)) {
      throw new Error(`${entrySource} has invalid source`)
    }
    for (const key of ["rootDir", "drill", "failedAt"]) {
      if (!nonEmptyString(staleFailure[key])) {
        throw new Error(`${entrySource} is missing ${key}`)
      }
    }
    if (!Number.isSafeInteger(staleFailure.ageMs) || staleFailure.ageMs < 0) {
      throw new Error(`${entrySource} has invalid ageMs`)
    }
    if (!Number.isSafeInteger(staleFailure.maxAgeMs) || staleFailure.maxAgeMs < 0) {
      throw new Error(`${entrySource} has invalid maxAgeMs`)
    }
  }
}

function validateCheckObject(check, source) {
  if (!check || typeof check !== "object" || Array.isArray(check)) {
    throw new Error(`${source} is not an object`)
  }
  if (!["passed", "failed", "skipped"].includes(check.status)) {
    throw new Error(`${source} has invalid status ${JSON.stringify(check.status)}`)
  }
}

function validatePlatformBundleArtifact(artifact, source) {
  if (!artifact || typeof artifact !== "object" || Array.isArray(artifact)) {
    throw new Error(`${source} is not an object`)
  }
  for (const key of ["path", "schema"]) {
    if (!nonEmptyString(artifact[key])) {
      throw new Error(`${source} is missing ${key}`)
    }
  }
  if (typeof artifact.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(artifact.sha256)) {
    throw new Error(`${source} has invalid sha256`)
  }
  if (!Number.isSafeInteger(artifact.sizeBytes) || artifact.sizeBytes < 0) {
    throw new Error(`${source} has invalid sizeBytes`)
  }
}

function validatePlatformValidationSuiteSummary(summary, source) {
  if (!summary || typeof summary !== "object" || Array.isArray(summary)) {
    throw new Error(`${source} is not an object`)
  }
  if (!Number.isSafeInteger(summary.testCount) || summary.testCount <= 0) {
    throw new Error(`${source} has invalid testCount`)
  }
  if (!Array.isArray(summary.coverageAreas) || summary.coverageAreas.length === 0) {
    throw new Error(`${source} has invalid coverageAreas`)
  }
  let coveredTests = 0
  const areaIds = new Set()
  for (const [index, area] of summary.coverageAreas.entries()) {
    const areaSource = `${source}.coverageAreas[${index}]`
    if (!area || typeof area !== "object" || Array.isArray(area)) {
      throw new Error(`${areaSource} is not an object`)
    }
    if (!nonEmptyString(area.id)) {
      throw new Error(`${areaSource} has invalid id`)
    }
    if (areaIds.has(area.id)) {
      throw new Error(`${source} has duplicate coverage area ${area.id}`)
    }
    areaIds.add(area.id)
    if (!Number.isSafeInteger(area.testCount) || area.testCount <= 0) {
      throw new Error(`${areaSource} has invalid testCount`)
    }
    coveredTests += area.testCount
  }
  if (coveredTests !== summary.testCount) {
    throw new Error(`${source} coverageAreas do not match testCount`)
  }
}

function validatePlatformFailureTaxonomySummary(summary, source) {
  if (!summary || typeof summary !== "object" || Array.isArray(summary)) {
    throw new Error(`${source} is not an object`)
  }
  validateStringArray(summary.drill, `${source}.drill`)
  validateStringArray(summary.scenario, `${source}.scenario`)
}

function validatePlatformRuntimeSignalsSummary(summary, source) {
  if (!Array.isArray(summary)) {
    throw new Error(`${source} is not an array`)
  }
  const ids = new Set()
  for (const [index, signal] of summary.entries()) {
    const signalSource = `${source}[${index}]`
    if (!signal || typeof signal !== "object" || Array.isArray(signal)) {
      throw new Error(`${signalSource} is not an object`)
    }
    if (!nonEmptyString(signal.id)) {
      throw new Error(`${signalSource} has invalid id`)
    }
    if (ids.has(signal.id)) {
      throw new Error(`${source} has duplicate signal ${signal.id}`)
    }
    ids.add(signal.id)
    if (!nonEmptyString(signal.owner)) {
      throw new Error(`${signalSource} has invalid owner`)
    }
  }
}

function validateGeneratedEvidence(generatedEvidence, source) {
  if (!generatedEvidence || typeof generatedEvidence !== "object" || Array.isArray(generatedEvidence)) {
    throw new Error(`${source} is not an object`)
  }
  validateGeneratedEvidenceKindArray(generatedEvidence.kinds ?? [], `${source}.kinds`)
  validateGeneratedValidationSuites(generatedEvidence.validationSuites, `${source}.validationSuites`)
  validateGeneratedMatrixReports(generatedEvidence.matrixReports, `${source}.matrixReports`)
}

function validateGeneratedValidationSuites(validationSuites, source) {
  if (!validationSuites || typeof validationSuites !== "object" || Array.isArray(validationSuites)) {
    throw new Error(`${source} is not an object`)
  }
  if (typeof validationSuites.enabled !== "boolean") {
    throw new Error(`${source} has invalid enabled`)
  }
  validateStringArray(validationSuites.artifactIndexes, `${source}.artifactIndexes`)
  validateStringArray(validationSuites.failureRoots, `${source}.failureRoots`)
  validateStringArray(validationSuites.outputRoots, `${source}.outputRoots`)
  if (!Array.isArray(validationSuites.commands)) {
    throw new Error(`${source}.commands is not an array`)
  }
  for (const [index, command] of validationSuites.commands.entries()) {
    validateGeneratedValidationSuiteCommand(command, `${source}.commands[${index}]`)
  }
  if (validationSuites.enabled && (validationSuites.artifactIndexes.length === 0 || validationSuites.failureRoots.length === 0 || validationSuites.outputRoots.length === 0 || validationSuites.commands.length === 0)) {
    throw new Error(`${source} enabled evidence is missing paths`)
  }
  if (!validationSuites.enabled && (validationSuites.artifactIndexes.length > 0 || validationSuites.failureRoots.length > 0 || validationSuites.outputRoots.length > 0 || validationSuites.commands.length > 0)) {
    throw new Error(`${source} disabled evidence has paths`)
  }
}

function validateGeneratedValidationSuiteCommand(command, source) {
  if (!command || typeof command !== "object" || Array.isArray(command)) {
    throw new Error(`${source} is not an object`)
  }
  for (const key of ["artifactIndexPath", "cwd", "failureRoot", "reportPath", "scriptPath"]) {
    if (!nonEmptyString(command[key])) {
      throw new Error(`${source} has invalid ${key}`)
    }
  }
  validateStringArray(command.args, `${source}.args`)
}

function validateGeneratedMatrixReports(matrixReports, source) {
  if (!matrixReports || typeof matrixReports !== "object" || Array.isArray(matrixReports)) {
    throw new Error(`${source} is not an object`)
  }
  if (typeof matrixReports.enabled !== "boolean") {
    throw new Error(`${source} has invalid enabled`)
  }
  if (typeof matrixReports.dryRun !== "boolean") {
    throw new Error(`${source} has invalid dryRun`)
  }
  if (typeof matrixReports.continueOnFailure !== "boolean") {
    throw new Error(`${source} has invalid continueOnFailure`)
  }
  validateGeneratedMatrixLimitations(matrixReports.limitations ?? [], `${source}.limitations`)
  validateStringArray(matrixReports.artifactIndexes, `${source}.artifactIndexes`)
  validateStringArray(matrixReports.roots, `${source}.roots`)
  if (!Array.isArray(matrixReports.commands)) {
    throw new Error(`${source}.commands is not an array`)
  }
  for (const [index, command] of matrixReports.commands.entries()) {
    validateGeneratedMatrixCommand(command, `${source}.commands[${index}]`)
  }
  if (matrixReports.enabled && (matrixReports.artifactIndexes.length === 0 || matrixReports.roots.length === 0 || matrixReports.commands.length === 0)) {
    throw new Error(`${source} enabled evidence is missing paths`)
  }
  if (matrixReports.enabled && matrixReports.dryRun && (matrixReports.limitations ?? []).length === 0) {
    throw new Error(`${source} dry-run evidence is missing limitations`)
  }
  if (!matrixReports.enabled && (matrixReports.artifactIndexes.length > 0 || matrixReports.roots.length > 0 || matrixReports.commands.length > 0 || (matrixReports.limitations ?? []).length > 0)) {
    throw new Error(`${source} disabled evidence has generated data`)
  }
}

function validateGeneratedMatrixLimitations(limitations, source) {
  if (!Array.isArray(limitations)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, limitation] of limitations.entries()) {
    const limitationSource = `${source}[${index}]`
    if (!limitation || typeof limitation !== "object" || Array.isArray(limitation)) {
      throw new Error(`${limitationSource} is not an object`)
    }
    for (const key of ["kind", "owner", "nextAction"]) {
      if (!nonEmptyString(limitation[key])) {
        throw new Error(`${limitationSource} has invalid ${key}`)
      }
    }
    if (!isKnownDrillGeneratedMatrixLimitation(limitation.kind)) {
      throw new Error(`${limitationSource} has unknown generated matrix limitation ${JSON.stringify(limitation.kind)}`)
    }
  }
}

function validateGeneratedMatrixCommand(command, source) {
  if (!command || typeof command !== "object" || Array.isArray(command)) {
    throw new Error(`${source} is not an object`)
  }
  for (const key of ["artifactIndexPath", "cwd", "reportPath", "scriptPath"]) {
    if (!nonEmptyString(command[key])) {
      throw new Error(`${source} has invalid ${key}`)
    }
  }
  validateStringArray(command.args, `${source}.args`)
}

function validateStringArray(value, source) {
  if (!Array.isArray(value)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, entry] of value.entries()) {
    if (typeof entry !== "string") {
      throw new Error(`${source}[${index}] is not a string`)
    }
  }
}

function validateArtifactEvidenceRepoArray(value, source) {
  validateStringArray(value, source)
  for (const [index, repo] of value.entries()) {
    if (!isKnownDrillArtifactEvidenceRepo(repo)) {
      throw new Error(`${source}[${index}] has unknown evidence repo ${JSON.stringify(repo)}`)
    }
  }
}

function validateArtifactKindArray(value, source) {
  validateStringArray(value, source)
  for (const [index, kind] of value.entries()) {
    if (!isKnownDrillArtifactKind(kind)) {
      throw new Error(`${source}[${index}] has unknown artifact kind ${JSON.stringify(kind)}`)
    }
  }
}

function validateGeneratedEvidenceKindArray(value, source) {
  validateStringArray(value, source)
  for (const [index, kind] of value.entries()) {
    if (!isKnownDrillGeneratedEvidenceKind(kind)) {
      throw new Error(`${source}[${index}] has unknown generated evidence kind ${JSON.stringify(kind)}`)
    }
  }
}

function validateGeneratedMatrixLimitationArray(value, source) {
  validateStringArray(value, source)
  for (const [index, limitation] of value.entries()) {
    if (!isKnownDrillGeneratedMatrixLimitation(limitation)) {
      throw new Error(`${source}[${index}] has unknown generated matrix limitation ${JSON.stringify(limitation)}`)
    }
  }
}

function validateExitCriterionStatusArray(value, source) {
  validateStringArray(value, source)
  for (const [index, status] of value.entries()) {
    if (!["satisfied", "failed", "skipped", "dry-run"].includes(status)) {
      throw new Error(`${source}[${index}] has unknown exit criterion status ${JSON.stringify(status)}`)
    }
  }
}

function validateProviderArray(value, source) {
  validateStringArray(value, source)
  for (const [index, provider] of value.entries()) {
    if (!isKnownDrillProvider(provider)) {
      throw new Error(`${source}[${index}] has unknown provider ${JSON.stringify(provider)}`)
    }
  }
}

function validateProviderAccountAliasArray(value, source) {
  validateStringArray(value, source)
  for (const [index, alias] of value.entries()) {
    const { provider } = parseProviderAccountAlias(alias)
    if (!isKnownDrillProvider(provider)) {
      throw new Error(`${source}[${index}] has unknown provider account alias provider ${JSON.stringify(provider)}`)
    }
  }
}

function validatePresetArray(value, source) {
  validateStringArray(value, source)
  for (const [index, preset] of value.entries()) {
    if (!isKnownDrillValidationGatePreset(preset)) {
      throw new Error(`${source}[${index}] has unknown validation gate preset ${JSON.stringify(preset)}`)
    }
  }
}

function validateDeploymentPresetArray(value, source) {
  validateStringArray(value, source)
  for (const [index, preset] of value.entries()) {
    if (!isKnownDrillDeploymentPreset(preset)) {
      throw new Error(`${source}[${index}] has unknown deployment preset ${JSON.stringify(preset)}`)
    }
  }
}

function validateRuntimeSignalArray(value, source) {
  validateStringArray(value, source)
  validateDrillRuntimeSignals(value, source)
}

function validateRuntimeSignalOwnerArray(value, source) {
  validateStringArray(value, source)
  for (const [index, owner] of value.entries()) {
    if (!DRILL_RUNTIME_SIGNAL_OWNERS.includes(owner)) {
      throw new Error(`${source}[${index}] has unknown runtime signal owner ${JSON.stringify(owner)}`)
    }
  }
}

function validateFailureClassificationArray(value, source) {
  validateStringArray(value, source)
  for (const [index, classification] of value.entries()) {
    if (!isKnownDrillFailureClassification(classification)) {
      throw new Error(`${source}[${index}] has unknown failure classification ${JSON.stringify(classification)}`)
    }
  }
}

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}
