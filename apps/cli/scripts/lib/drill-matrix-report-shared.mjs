import {
  drillFailureNextActionForClassification,
  drillFailureOwnerForClassification,
  validateDrillFailureClassification,
} from "./drill-failure-taxonomy.mjs"
import { validateDrillExitCriterionStatus } from "./drill-exit-criterion-statuses.mjs"
import { validateDrillRuntimeSignal } from "./drill-runtime-signals.mjs"
import { validateDrillDeploymentPresets } from "./drill-environment-presets.mjs"
import { validateDrillProviders } from "./drill-provider-profiles.mjs"
import { validateDrillMatrixScenarioStatus } from "./drill-matrix-statuses.mjs"
import { isSensitiveDrillKey, looksLikeDrillSecretValue } from "./drill-secrets.mjs"

export function validateMatrixReportIncompleteExitCriteria(criteria, source) {
  if (!Array.isArray(criteria)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, criterion] of criteria.entries()) {
    const criterionSource = `${source}[${index}]`
    if (!criterion || typeof criterion !== "object" || Array.isArray(criterion)) {
      throw new Error(`${criterionSource} is not an object`)
    }
    if (!nonEmptyString(criterion.scenarioId)) {
      throw new Error(`${criterionSource} is missing scenarioId`)
    }
    validateExitCriterionEvidence(criterion, criterionSource)
    if (criterion.status === "satisfied") {
      throw new Error(`${criterionSource} must not be satisfied`)
    }
    validateOptionalCriterionDiagnostics(criterion, criterionSource)
  }
}

export function validateOptionalCriterionDiagnostics(criterion, source) {
  if (criterion.owner !== undefined && criterion.owner !== null && !nonEmptyString(criterion.owner)) {
    throw new Error(`${source} has invalid owner`)
  }
  if (criterion.classification !== undefined && criterion.classification !== null) {
    if (!nonEmptyString(criterion.classification)) {
      throw new Error(`${source} has invalid classification`)
    }
    validateDrillFailureClassification(criterion.classification, source)
    const expectedOwner = drillFailureOwnerForClassification(criterion.classification)
    if (criterion.owner !== expectedOwner) {
      throw new Error(`${source} owner does not match classification`)
    }
    const expectedNextAction = drillFailureNextActionForClassification(criterion.classification, { target: "scenario" })
    if (criterion.nextAction !== expectedNextAction) {
      throw new Error(`${source} nextAction does not match classification`)
    }
  } else if (criterion.nextAction !== undefined && criterion.nextAction !== null) {
    throw new Error(`${source} nextAction requires classification`)
  }
}

export function validateReportMetadata(value, source) {
  if (!value || typeof value !== "object") {
    throw new Error(`${source} must be an object`)
  }
  validateReportMetadataValue(value, source)
  validateDeploymentPresetMetadata(value, source)
  validateProviderMetadata(value, source)
}

export function validateDeploymentPresetMetadata(metadata, source) {
  const deploymentPresets = deploymentPresetsForReport({ metadata })
  if (deploymentPresets.length > 0) validateDeploymentPresetList(deploymentPresets, `${source}.deploymentPresets`)
  if (metadata.deploymentPresetCount !== undefined) {
    if (!Number.isInteger(metadata.deploymentPresetCount) || metadata.deploymentPresetCount < 0) {
      throw new Error(`${source}.deploymentPresetCount is invalid`)
    }
    if (metadata.deploymentPresetCount !== deploymentPresets.length) {
      throw new Error(`${source}.deploymentPresetCount does not match deploymentPresets`)
    }
  }
}

export function validateProviderMetadata(metadata, source) {
  const providers = metadataListValue(metadata.providers)
  if (providers.length > 0) validateProviderList(providers, `${source}.providers`)
  if (metadata.providerCount !== undefined) {
    if (!Number.isInteger(metadata.providerCount) || metadata.providerCount < 0) {
      throw new Error(`${source}.providerCount is invalid`)
    }
    if (metadata.providerCount !== providers.length) {
      throw new Error(`${source}.providerCount does not match providers`)
    }
  }
  if (metadata.providerModelOverrides !== undefined) {
    if (typeof metadata.providerModelOverrides !== "string") {
      throw new Error(`${source}.providerModelOverrides is invalid`)
    }
    const providerSet = new Set(providers)
    for (const provider of metadataListValue(metadata.providerModelOverrides)) {
      if (!providerSet.has(provider)) {
        throw new Error(`${source}.providerModelOverrides includes provider not in providers`)
      }
    }
  }
  if (metadata.providerAccountAliases !== undefined) {
    if (typeof metadata.providerAccountAliases !== "string") {
      throw new Error(`${source}.providerAccountAliases is invalid`)
    }
    const providerSet = new Set(providers)
    for (const entry of metadataListValue(metadata.providerAccountAliases)) {
      const [provider, alias] = entry.split("=", 2).map((part) => part.trim())
      if (!providerSet.has(provider)) {
        throw new Error(`${source}.providerAccountAliases includes provider not in providers`)
      }
      if (!validProviderAccountAlias(alias)) {
        throw new Error(`${source}.providerAccountAliases includes invalid account alias`)
      }
    }
  }
}

export function validProviderAccountAlias(alias) {
  return typeof alias === "string"
    && /^[a-zA-Z0-9._-]{1,64}$/.test(alias)
    && !looksLikeDrillSecretValue(alias)
}

export function validateScenarioProviderMetadataConsistency(report, source) {
  if (!report.scenarios.some((scenario) => scenario.providers !== undefined)) return
  const metadataProviders = providersForReport(report)
  if (metadataProviders.length === 0) return
  const scenarioProviders = [...new Set(report.scenarios.flatMap((scenario) => scenario.providers ?? []))].sort()
  if (JSON.stringify(metadataProviders) !== JSON.stringify(scenarioProviders)) {
    throw new Error(`${source}.metadata.providers do not match scenario providers`)
  }
}

export function validateMatrixAggregateReportCounts(counts, source) {
  if (!counts || typeof counts !== "object" || Array.isArray(counts)) {
    throw new Error(`${source} is missing`)
  }
  for (const key of ["passed", "failed", "skipped", "dryRun"]) {
    if (!Number.isSafeInteger(counts[key]) || counts[key] < 0) {
      throw new Error(`${source} has invalid ${key}`)
    }
  }
}

export function sumMatrixAggregateReportEntries(reports) {
  return reports.reduce((totals, report) => {
    totals.scenarios += report.scenarioCount
    totals.passed += report.counts.passed
    totals.failed += report.counts.failed
    totals.skipped += report.counts.skipped
    totals.dryRun += report.counts.dryRun
    totals.durationMs += report.durationMs
    return totals
  }, {
    scenarios: 0,
    passed: 0,
    failed: 0,
    skipped: 0,
    dryRun: 0,
    durationMs: 0,
  })
}

export function validateCountObject(value, source) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is missing`)
  }
  for (const [key, count] of Object.entries(value)) {
    if (!nonEmptyString(key) || !Number.isSafeInteger(count) || count < 0) {
      throw new Error(`${source} has invalid count for ${JSON.stringify(key)}`)
    }
  }
}

export function validateExitCriteriaCountObject(value, source) {
  validateCountObject(value, source)
  for (const status of Object.keys(value)) {
    validateDrillExitCriterionStatus(status, source, {
      message: () => `${source} has invalid status ${JSON.stringify(status)}`,
    })
  }
}

export function validateRuntimeSignalCountObject(value, source) {
  validateCountObject(value, source)
  for (const signal of Object.keys(value)) {
    validateDrillRuntimeSignal(signal, source)
  }
}

export function validateFailureClassificationCountObject(value, source) {
  validateCountObject(value, source)
  for (const classification of Object.keys(value)) {
    validateDrillFailureClassification(classification, source)
  }
}

export function validateRuntimeSignalEvidenceObject(value, source, { aggregate }) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is missing`)
  }
  for (const [signal, scenarios] of Object.entries(value)) {
    validateDrillRuntimeSignal(signal, source)
    if (!Array.isArray(scenarios) || scenarios.length === 0) {
      throw new Error(`${source}.${signal} has invalid scenarios`)
    }
    for (const [index, scenario] of scenarios.entries()) {
      validateRuntimeSignalEvidenceEntry(scenario, `${source}.${signal}[${index}]`, { aggregate })
    }
  }
}

export function validateRuntimeSignalEvidenceEntry(entry, source, { aggregate }) {
  if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
    throw new Error(`${source} is not an object`)
  }
  if (aggregate && !nonEmptyString(entry.matrix)) {
    throw new Error(`${source} is missing matrix`)
  }
  if (aggregate && entry.source !== null && entry.source !== undefined && !nonEmptyString(entry.source)) {
    throw new Error(`${source} has invalid source`)
  }
  if (!nonEmptyString(entry.id)) {
    throw new Error(`${source} is missing id`)
  }
  validateDrillMatrixScenarioStatus(entry.status, source)
}

export function assertRuntimeSignalEvidenceCounts(label, counts, evidence) {
  const expected = Object.fromEntries(Object.entries(evidence)
    .map(([signal, scenarios]) => [signal, scenarios.length])
    .sort(([left], [right]) => left.localeCompare(right)))
  if (JSON.stringify(counts) !== JSON.stringify(expected)) {
    throw new Error(`${label} do not match runtimeSignalScenarios`)
  }
}

export function assertRuntimeSignalEvidenceScenarioIds(label, scenarioIds, evidence) {
  const knownScenarioIds = new Set(scenarioIds)
  for (const scenarios of Object.values(evidence)) {
    for (const scenario of scenarios) {
      if (!knownScenarioIds.has(scenario.id)) {
        throw new Error(`${label} references unknown scenario ${JSON.stringify(scenario.id)}`)
      }
    }
  }
}

export function runtimeSignalCountsForScenarios(scenarios) {
  const counts = new Map()
  for (const scenario of scenarios) {
    for (const signal of runtimeSignalsForScenario(scenario)) {
      counts.set(signal, (counts.get(signal) ?? 0) + 1)
    }
  }
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

export function runtimeSignalScenariosForReport(report) {
  const evidence = new Map()
  for (const scenario of report.scenarios) {
    for (const signal of runtimeSignalsForScenario(scenario)) {
      appendRuntimeSignalEvidence(evidence, signal, {
        id: scenario.id,
        status: scenario.status,
      })
    }
  }
  return formatRuntimeSignalEvidence(evidence)
}

export function deploymentPresetsForReport(report) {
  const value = report.metadata?.deploymentPresets
  if (!nonEmptyString(value)) return []
  return [...new Set(value.split(",").map((preset) => preset.trim()).filter(Boolean))].sort()
}

export function providersForReport(report) {
  return metadataListValue(report.metadata?.providers)
}

export function metadataListValue(value) {
  if (!nonEmptyString(value)) return []
  return [...new Set(value.split(",").map((item) => item.trim()).filter(Boolean))].sort()
}

export function validateReportMetadataValue(value, source, key = "") {
  if (isSensitiveDrillKey(key)) {
    throw new Error(`${source} includes sensitive metadata key ${JSON.stringify(key)}`)
  }
  if (typeof value === "string") {
    if (looksLikeDrillSecretValue(value)) {
      throw new Error(`${source} includes secret-looking metadata value`)
    }
    return
  }
  if (value === null || typeof value === "number" || typeof value === "boolean") return
  if (Array.isArray(value)) {
    for (const [index, item] of value.entries()) {
      validateReportMetadataValue(item, `${source}[${index}]`)
    }
    return
  }
  if (typeof value !== "object") {
    throw new Error(`${source} has unsupported metadata value`)
  }
  for (const [childKey, childValue] of Object.entries(value)) {
    validateReportMetadataValue(childValue, `${source}.${childKey}`, childKey)
  }
}

export function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}

export function nonSecretString(value) {
  return nonEmptyString(value) && !looksLikeDrillSecretValue(value)
}

export function validateRuntimeSignals(value, source) {
  if (!Array.isArray(value) || !value.every(nonEmptyString)) {
    throw new Error(`${source} has invalid runtimeSignals`)
  }
  for (const signal of value) {
    validateDrillRuntimeSignal(signal, source)
  }
}

export function validateProviderList(value, source) {
  validateDrillProviders(value, source)
}

export function validateProviderCountObject(value, source) {
  validateCountObject(value, source)
  validateDrillProviders(Object.keys(value), source)
}

export function validateDeploymentPresetList(value, source) {
  validateDrillDeploymentPresets(value, source)
}

export function validateDeploymentPresetCountObject(value, source) {
  validateCountObject(value, source)
  validateDrillDeploymentPresets(Object.keys(value), source)
}

export function validateExitCriteriaEvidence(scenario, source) {
  if (!Array.isArray(scenario.exitCriteriaEvidence)) {
    throw new Error(`${source} is not an array`)
  }
  const criteria = exitCriteriaForScenario(scenario)
  if (scenario.exitCriteriaEvidence.length !== criteria.length) {
    throw new Error(`${source} length does not match exitCriteria`)
  }
  for (const [index, criterion] of scenario.exitCriteriaEvidence.entries()) {
    validateExitCriterionEvidence(criterion, `${source}[${index}]`)
    if (criterion.criterion !== criteria[index]) {
      throw new Error(`${source}[${index}] criterion does not match exitCriteria`)
    }
  }
}

export function validateExitCriterionEvidence(criterion, source) {
  if (!criterion || typeof criterion !== "object" || Array.isArray(criterion)) {
    throw new Error(`${source} is not an object`)
  }
  if (!nonEmptyString(criterion.id)) {
    throw new Error(`${source} is missing id`)
  }
  if (!nonEmptyString(criterion.criterion)) {
    throw new Error(`${source} is missing criterion`)
  }
  validateDrillExitCriterionStatus(criterion.status, source, {
    message: () => `${source} has invalid status ${JSON.stringify(criterion.status)}`,
  })
  if (criterion.reason !== null && typeof criterion.reason !== "string") {
    throw new Error(`${source} has invalid reason`)
  }
  if (criterion.status === "satisfied" && criterion.reason !== null) {
    throw new Error(`${source} satisfied criterion must not include reason`)
  }
  if (criterion.status !== "satisfied" && !nonEmptyString(criterion.reason)) {
    throw new Error(`${source} incomplete criterion is missing reason`)
  }
}

export function countExitCriteriaStatuses(scenarios) {
  const counts = new Map()
  for (const scenario of scenarios) {
    for (const criterion of exitCriteriaEvidenceForScenario(scenario)) {
      counts.set(criterion.status, (counts.get(criterion.status) ?? 0) + 1)
    }
  }
  return Object.fromEntries([...counts.entries()].sort(([left], [right]) => left.localeCompare(right)))
}

export function incompleteExitCriteriaForScenarios(scenarios) {
  const incomplete = []
  for (const scenario of scenarios) {
    for (const criterion of exitCriteriaEvidenceForScenario(scenario)) {
      if (criterion.status !== "satisfied") {
        incomplete.push({
          scenarioId: scenario.id,
          id: criterion.id,
          criterion: criterion.criterion,
          status: criterion.status,
          reason: criterion.reason ?? null,
          ...diagnosticsForIncompleteExitCriterion(scenario),
        })
      }
    }
  }
  return incomplete
}

export function sameIncompleteExitCriteria(left, right) {
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false
  const leftKeys = left.map(canonicalIncompleteExitCriterionKey).sort()
  const rightKeys = right.map(canonicalIncompleteExitCriterionKey).sort()
  return leftKeys.every((key, index) => key === rightKeys[index])
}

export function canonicalIncompleteExitCriterionKey(criterion) {
  return JSON.stringify({
    scenarioId: criterion.scenarioId,
    id: criterion.id,
    criterion: criterion.criterion,
    status: criterion.status,
    reason: criterion.reason ?? null,
    owner: criterion.owner ?? null,
    classification: criterion.classification ?? null,
    nextAction: criterion.nextAction ?? null,
  })
}

export function diagnosticsForIncompleteExitCriterion(scenario) {
  const diagnostics = {}
  if (nonEmptyString(scenario.owner)) diagnostics.owner = scenario.owner
  if (nonEmptyString(scenario.classification)) {
    diagnostics.classification = scenario.classification
    diagnostics.owner = drillFailureOwnerForClassification(scenario.classification)
    diagnostics.nextAction = scenario.nextAction ?? drillFailureNextActionForClassification(scenario.classification, { target: "scenario" })
  }
  return diagnostics
}

export function incompleteExitCriteriaCount(exitCriteria) {
  return Object.entries(exitCriteria)
    .filter(([status]) => status !== "satisfied")
    .reduce((total, [, count]) => total + count, 0)
}

export function exitCriteriaEvidenceForScenario(scenario) {
  if (Array.isArray(scenario.exitCriteriaEvidence)) return scenario.exitCriteriaEvidence
  return exitCriteriaForScenario(scenario).map((criterion, index) => ({
    id: `${scenario.id}:exit-${String(index + 1).padStart(2, "0")}`,
    criterion,
    status: exitCriteriaStatusForScenario(scenario),
    reason: exitCriteriaReasonForScenario(scenario),
  }))
}

export function exitCriteriaStatusForScenario(scenario) {
  if (scenario.status === "passed") return "satisfied"
  return scenario.status
}

export function exitCriteriaReasonForScenario(scenario) {
  if (scenario.status === "passed") return null
  return scenario.reason ?? (scenario.status === "dry-run"
    ? "scenario command was selected but not executed"
    : "scenario did not complete")
}

export function countScenarioStatuses(scenarios) {
  const counts = {
    passed: 0,
    failed: 0,
    skipped: 0,
    dryRun: 0,
  }
  for (const scenario of scenarios) {
    if (scenario.status === "passed") counts.passed += 1
    else if (scenario.status === "failed") counts.failed += 1
    else if (scenario.status === "skipped") counts.skipped += 1
    else if (scenario.status === "dry-run") counts.dryRun += 1
  }
  return counts
}

export function exitCriteriaForScenario(scenario) {
  return Array.isArray(scenario.exitCriteria)
    ? scenario.exitCriteria.filter((criterion) => typeof criterion === "string" && criterion.trim().length > 0)
    : []
}

export function artifactHintsForScenario(scenario) {
  return Array.isArray(scenario.artifactHints)
    ? scenario.artifactHints.filter(isValidArtifactHint).map(formatArtifactHint)
    : []
}

export function isValidArtifactHint(hint) {
  if (typeof hint === "string") return hint.trim().length > 0
  return Boolean(hint)
    && typeof hint === "object"
    && !Array.isArray(hint)
    && nonEmptyString(hint.kind)
    && nonEmptyString(hint.path)
}

export function artifactHintLooksSecret(hint) {
  if (typeof hint === "string") return looksLikeDrillSecretValue(hint)
  return looksLikeDrillSecretValue(hint?.kind) || looksLikeDrillSecretValue(hint?.path)
}

export function formatArtifactHint(hint) {
  if (typeof hint === "string") return hint
  return `${hint.kind}:${hint.path}`
}

export function runtimeSignalsForScenario(scenario) {
  return Array.isArray(scenario.runtimeSignals)
    ? [...new Set(scenario.runtimeSignals.filter(nonEmptyString))].sort()
    : []
}

export function appendRuntimeSignalEvidence(evidence, signal, entry) {
  const entries = evidence.get(signal) ?? []
  entries.push(entry)
  evidence.set(signal, entries)
}

export function formatRuntimeSignalEvidence(evidence) {
  return Object.fromEntries([...evidence.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([signal, entries]) => [signal, entries
      .map((entry) => ({
        ...(entry.matrix !== undefined ? { matrix: entry.matrix } : {}),
        ...(entry.source !== undefined ? { source: entry.source } : {}),
        id: entry.id,
        status: entry.status,
      }))
      .sort(compareRuntimeSignalEvidenceEntries)]))
}

export function compareRuntimeSignalEvidenceEntries(left, right) {
  return String(left.matrix ?? "").localeCompare(String(right.matrix ?? ""))
    || String(left.source ?? "").localeCompare(String(right.source ?? ""))
    || left.id.localeCompare(right.id)
    || left.status.localeCompare(right.status)
}

export function formatCountObject(counts) {
  return Object.entries(counts).map(([key, count]) => `${key}=${count}`).join(" ")
}

export function formatRuntimeSignalScenarioRef(scenario) {
  const source = scenario.source ? ` source=${scenario.source}` : ""
  return `${scenario.matrix}/${scenario.id}(${scenario.status})${source}`
}
