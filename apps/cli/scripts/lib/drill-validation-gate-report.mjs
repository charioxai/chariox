import { validateDrillAggregateNextAction } from "./drill-aggregate-actions.mjs"
import { validateDrillArtifactIndexAggregate } from "./drill-artifacts.mjs"
import { validateDrillFailureManifestAggregate } from "./drill-failure-manifest.mjs"
import { validateDrillMatrixAggregate } from "./drill-matrix-report.mjs"

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
  validateStringArray(report.presets ?? [], `${source}.presets`)
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
  validateStringArray(check.requiredRuntimeSignals ?? [], `${source}.requiredRuntimeSignals`)
  validateStringArray(check.missingRuntimeSignals ?? [], `${source}.missingRuntimeSignals`)
  validateStringArray(check.requiredFailureClassifications ?? [], `${source}.requiredFailureClassifications`)
  validateStringArray(check.missingFailureClassifications ?? [], `${source}.missingFailureClassifications`)
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
  validateStringArray(check.requiredArtifactKinds ?? [], `${source}.requiredArtifactKinds`)
  validateStringArray(check.missingArtifactKinds ?? [], `${source}.missingArtifactKinds`)
  validateStringArray(check.requiredArtifactEvidenceRepos ?? [], `${source}.requiredArtifactEvidenceRepos`)
  validateStringArray(check.missingArtifactEvidenceRepos ?? [], `${source}.missingArtifactEvidenceRepos`)
  validateStringArray(check.requiredArtifactRuntimeSignals ?? [], `${source}.requiredArtifactRuntimeSignals`)
  validateStringArray(check.missingArtifactRuntimeSignals ?? [], `${source}.missingArtifactRuntimeSignals`)
  validateStringArray(check.requiredArtifactRuntimeSignalOwners ?? [], `${source}.requiredArtifactRuntimeSignalOwners`)
  validateStringArray(check.missingArtifactRuntimeSignalOwners ?? [], `${source}.missingArtifactRuntimeSignalOwners`)
  validateStringArray(check.requiredArtifactOwners ?? [], `${source}.requiredArtifactOwners`)
  validateStringArray(check.missingArtifactOwners ?? [], `${source}.missingArtifactOwners`)
  validateStringArray(check.requiredArtifactClassifications ?? [], `${source}.requiredArtifactClassifications`)
  validateStringArray(check.missingArtifactClassifications ?? [], `${source}.missingArtifactClassifications`)
  if (check.status === "failed" && !check.aggregate && !nonEmptyString(check.error)) {
    throw new Error(`${source} is missing error`)
  }
  if (check.aggregate) {
    validateDrillArtifactIndexAggregate(check.aggregate, `${source}.aggregate`)
  }
}

function validateMatrixCheck(check, source) {
  validateCheckObject(check, source)
  validateStringArray(check.roots, `${source}.roots`)
  validateStringArray(check.inputs, `${source}.inputs`)
  validateStringArray(check.reportPaths, `${source}.reportPaths`)
  validateStringArray(check.requiredMatrices ?? [], `${source}.requiredMatrices`)
  validateStringArray(check.missingMatrices ?? [], `${source}.missingMatrices`)
  validateStringArray(check.requiredMatrixClassifications ?? [], `${source}.requiredMatrixClassifications`)
  validateStringArray(check.missingMatrixClassifications ?? [], `${source}.missingMatrixClassifications`)
  validateStringArray(check.requiredMatrixRuntimeSignals ?? [], `${source}.requiredMatrixRuntimeSignals`)
  validateStringArray(check.missingMatrixRuntimeSignals ?? [], `${source}.missingMatrixRuntimeSignals`)
  validateStringArray(check.requiredDeploymentPresets ?? [], `${source}.requiredDeploymentPresets`)
  validateStringArray(check.missingDeploymentPresets ?? [], `${source}.missingDeploymentPresets`)
  validateStringArray(check.requiredProviders ?? [], `${source}.requiredProviders`)
  validateStringArray(check.missingProviders ?? [], `${source}.missingProviders`)
  validateStringArray(check.requiredScenarios ?? [], `${source}.requiredScenarios`)
  validateStringArray(check.missingScenarios ?? [], `${source}.missingScenarios`)
  if (typeof check.requireComplete !== "boolean") {
    throw new Error(`${source} has invalid requireComplete`)
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

function validateFailureCheck(check, source) {
  validateCheckObject(check, source)
  validateStringArray(check.roots, `${source}.roots`)
  validateStringArray(check.inputs, `${source}.inputs`)
  validateStringArray(check.manifestPaths, `${source}.manifestPaths`)
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
  validateStringArray(validationSuites.outputRoots, `${source}.outputRoots`)
  if (validationSuites.enabled && (validationSuites.artifactIndexes.length === 0 || validationSuites.outputRoots.length === 0)) {
    throw new Error(`${source} enabled evidence is missing paths`)
  }
  if (!validationSuites.enabled && (validationSuites.artifactIndexes.length > 0 || validationSuites.outputRoots.length > 0)) {
    throw new Error(`${source} disabled evidence has paths`)
  }
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
  validateStringArray(matrixReports.roots, `${source}.roots`)
  if (!Array.isArray(matrixReports.commands)) {
    throw new Error(`${source}.commands is not an array`)
  }
  for (const [index, command] of matrixReports.commands.entries()) {
    validateGeneratedMatrixCommand(command, `${source}.commands[${index}]`)
  }
  if (matrixReports.enabled && (matrixReports.roots.length === 0 || matrixReports.commands.length === 0)) {
    throw new Error(`${source} enabled evidence is missing paths`)
  }
  if (!matrixReports.enabled && (matrixReports.roots.length > 0 || matrixReports.commands.length > 0)) {
    throw new Error(`${source} disabled evidence has paths`)
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

function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}
