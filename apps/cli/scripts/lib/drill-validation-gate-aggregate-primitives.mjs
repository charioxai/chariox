import { validateDrillArtifactKind } from "./drill-artifact-kinds.mjs"
import { validateDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import { validateDrillDeploymentPreset } from "./drill-environment-presets.mjs"
import { validateDrillExitCriterionStatus } from "./drill-exit-criterion-statuses.mjs"
import { validateDrillFailureClassification } from "./drill-failure-taxonomy.mjs"
import {
  validateDrillGeneratedEvidenceKind,
  validateDrillGeneratedEvidencePath,
} from "./drill-generated-evidence-metadata.mjs"
import { validateDrillGeneratedMatrixLimitation } from "./drill-generated-matrix-limitations.mjs"
import { validateDrillMatrixScenarioStatus } from "./drill-matrix-statuses.mjs"
import {
  parseProviderAccountAlias,
  validateDrillProvider,
} from "./drill-provider-profiles.mjs"
import {
  validateDrillArtifactValidationPreset,
  validateDrillValidationGatePreset,
} from "./drill-validation-gate-presets.mjs"
import {
  drillRuntimeSignalOwnerCounts,
  validateDrillRuntimeSignal,
  validateDrillRuntimeSignalOwner,
} from "./drill-runtime-signals.mjs"
import { validateDrillRuntimeAuthorityInvariant } from "./drill-runtime-authority-invariants.mjs"

export function validateMatrixRuntimeSignalSources(value, source) {
  validateRuntimeSignalScenarioMap(value, source, { reportSource: true })
}

export function validateRuntimeSignalScenarioMap(value, source, { reportSource }) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is not an object`)
  }
  for (const [signal, scenarios] of Object.entries(value)) {
    validateDrillRuntimeSignal(signal, source)
    if (!Array.isArray(scenarios)) {
      throw new Error(`${source}.${signal} is not an array`)
    }
    for (const [index, scenario] of scenarios.entries()) {
      validateRuntimeSignalScenario(scenario, `${source}.${signal}[${index}]`, { reportSource })
    }
  }
}

export function validateRuntimeSignalScenario(scenario, source, { reportSource }) {
  if (!scenario || typeof scenario !== "object" || Array.isArray(scenario)) {
    throw new Error(`${source} is not an object`)
  }
  if (reportSource && scenario.reportSource !== null && scenario.reportSource !== undefined && !nonEmptyString(scenario.reportSource)) {
    throw new Error(`${source} has invalid reportSource`)
  }
  if (!nonEmptyString(scenario.matrix)) {
    throw new Error(`${source} is missing matrix`)
  }
  if (scenario.source !== null && scenario.source !== undefined && !nonEmptyString(scenario.source)) {
    throw new Error(`${source} has invalid source`)
  }
  if (!nonEmptyString(scenario.id)) {
    throw new Error(`${source} is missing id`)
  }
  validateDrillMatrixScenarioStatus(scenario.status, source)
}

export function validateRuntimeSignalArray(value, source) {
  validateStringArray(value, source)
  for (const [index, signal] of value.entries()) {
    validateDrillRuntimeSignal(signal, `${source}[${index}]`)
  }
}

export function validateRuntimeSignalOwnerArray(value, source) {
  validateStringArray(value, source)
  for (const [index, owner] of value.entries()) {
    validateDrillRuntimeSignalOwner(owner, `${source}[${index}]`)
  }
}

export function validateRuntimeAuthorityInvariantArray(value, source) {
  validateStringArray(value, source)
  for (const [index, invariant] of value.entries()) {
    validateDrillRuntimeAuthorityInvariant(invariant, `${source}[${index}]`)
  }
}

export function validateFailureClassificationArray(value, source) {
  validateStringArray(value, source)
  for (const [index, classification] of value.entries()) {
    validateDrillFailureClassification(classification, `${source}[${index}]`, {
      label: "failure classification",
    })
  }
}

export function validateArtifactEvidenceRepoArray(value, source) {
  validateStringArray(value, source)
  for (const [index, repo] of value.entries()) {
    validateDrillArtifactEvidenceRepo(repo, `${source}[${index}]`)
  }
}

export function validateArtifactKindArray(value, source) {
  validateStringArray(value, source)
  for (const [index, kind] of value.entries()) {
    validateDrillArtifactKind(kind, `${source}[${index}]`)
  }
}

export function validateProviderArray(value, source) {
  validateStringArray(value, source)
  for (const [index, provider] of value.entries()) {
    validateDrillProvider(provider, `${source}[${index}]`)
  }
}

export function validateProviderAccountAliasArray(value, source) {
  validateStringArray(value, source)
  for (const [index, alias] of value.entries()) {
    const { provider } = parseProviderAccountAlias(alias)
    validateDrillProvider(provider, `${source}[${index}]`, {
      label: "provider account alias provider",
    })
  }
}

export function validateArtifactValidationPresetArray(value, source) {
  validateStringArray(value, source)
  for (const [index, preset] of value.entries()) {
    validateDrillArtifactValidationPreset(preset, `${source}[${index}]`, {
      label: "artifact validation preset",
    })
  }
}

export function validatePresetArray(value, source) {
  validateStringArray(value, source)
  for (const [index, preset] of value.entries()) {
    validateDrillValidationGatePreset(preset, `${source}[${index}]`)
  }
}

export function validateDeploymentPresetArray(value, source) {
  validateStringArray(value, source)
  for (const [index, preset] of value.entries()) {
    validateDrillDeploymentPreset(preset, `${source}[${index}]`)
  }
}

export function validateGeneratedEvidenceKindArray(value, source) {
  validateStringArray(value, source)
  for (const [index, kind] of value.entries()) {
    validateDrillGeneratedEvidenceKind(kind, `${source}[${index}]`)
  }
}

export function validateGeneratedMatrixLimitationArray(value, source) {
  validateStringArray(value, source)
  for (const [index, limitation] of value.entries()) {
    validateDrillGeneratedMatrixLimitation(limitation, `${source}[${index}]`)
  }
}

export function validateExitCriterionStatusArray(value, source) {
  validateStringArray(value, source)
  for (const [index, status] of value.entries()) {
    validateDrillExitCriterionStatus(status, `${source}[${index}]`)
  }
}

export function validateStringArray(value, source) {
  if (!Array.isArray(value)) {
    throw new Error(`${source} is not an array`)
  }
  for (const [index, entry] of value.entries()) {
    if (typeof entry !== "string") {
      throw new Error(`${source}[${index}] is not a string`)
    }
  }
}

export function validateGeneratedEvidencePathArray(value, source) {
  validateStringArray(value, source)
  for (const [index, entry] of value.entries()) {
    validateGeneratedEvidencePathText(entry, `${source}[${index}]`)
  }
}

export function validateGeneratedEvidencePathText(value, source) {
  validateDrillGeneratedEvidencePath(value, source)
}

export function validateRuntimeSignalCountObject(value, source) {
  validateCountObject(value, source)
  for (const signal of Object.keys(value)) {
    validateDrillRuntimeSignal(signal, source)
  }
}

export function validateRuntimeAuthorityInvariantCountObject(value, source) {
  validateCountObject(value, source)
  for (const invariant of Object.keys(value)) {
    validateDrillRuntimeAuthorityInvariant(invariant, source)
  }
}

export function validateRuntimeSignalOwnerCountObject(value, source) {
  validateCountObject(value, source)
  for (const owner of Object.keys(value)) {
    validateDrillRuntimeSignalOwner(owner, source)
  }
}

export function validateFailureClassificationCountObject(value, source) {
  validateCountObject(value, source)
  for (const classification of Object.keys(value)) {
    validateDrillFailureClassification(classification, source, {
      label: "failure classification",
    })
  }
}

export function validateArtifactEvidenceRepoCountObject(value, source) {
  validateCountObject(value, source)
  for (const repo of Object.keys(value)) {
    validateDrillArtifactEvidenceRepo(repo, source)
  }
}

export function validateArtifactKindCountObject(value, source) {
  validateCountObject(value, source)
  for (const kind of Object.keys(value)) {
    validateDrillArtifactKind(kind, source)
  }
}

export function validateExitCriterionStatusCountObject(value, source) {
  validateCountObject(value, source)
  for (const status of Object.keys(value)) {
    validateDrillExitCriterionStatus(status, source)
  }
}

export function validateProviderCountObject(value, source) {
  validateCountObject(value, source)
  for (const provider of Object.keys(value)) {
    validateDrillProvider(provider, source)
  }
}

export function validateProviderAccountAliasCountObject(value, source) {
  validateCountObject(value, source)
  for (const alias of Object.keys(value)) {
    const { provider } = parseProviderAccountAlias(alias)
    validateDrillProvider(provider, source, {
      label: "provider account alias provider",
    })
  }
}

export function validateArtifactValidationPresetCountObject(value, source) {
  validateCountObject(value, source)
  for (const preset of Object.keys(value)) {
    validateDrillArtifactValidationPreset(preset, source, {
      label: "artifact validation preset",
    })
  }
}

export function validatePresetCountObject(value, source) {
  validateCountObject(value, source)
  for (const preset of Object.keys(value)) {
    validateDrillValidationGatePreset(preset, source)
  }
}

export function validateDeploymentPresetCountObject(value, source) {
  validateCountObject(value, source)
  for (const preset of Object.keys(value)) {
    validateDrillDeploymentPreset(preset, source)
  }
}

export function validateGeneratedEvidenceKindCountObject(value, source) {
  validateCountObject(value, source)
  for (const kind of Object.keys(value)) {
    validateDrillGeneratedEvidenceKind(kind, source)
  }
}

export function validateGeneratedMatrixLimitationCountObject(value, source) {
  validateCountObject(value, source)
  for (const limitation of Object.keys(value)) {
    validateDrillGeneratedMatrixLimitation(limitation, source)
  }
}

export function validateGeneratedEvidencePathCountObject(value, source) {
  validateCountObject(value, source)
  for (const key of Object.keys(value)) {
    validateGeneratedEvidencePathText(key, `${source}.${key}`)
  }
}

export function validateRuntimeSignalOwnerCountsMatch(runtimeSignals, runtimeSignalOwners, source) {
  validateRuntimeSignalCountObject(runtimeSignals, source.replace(/\.runtimeSignalOwners$/, ".runtimeSignals"))
  validateRuntimeSignalOwnerCountObject(runtimeSignalOwners, source)
  const expected = drillRuntimeSignalOwnerCounts(runtimeSignals)
  if (JSON.stringify(runtimeSignalOwners) !== JSON.stringify(expected)) {
    throw new Error(`${source} must match runtimeSignals`)
  }
}

export function validateCountObject(value, source) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${source} is not an object`)
  }
  for (const [key, count] of Object.entries(value)) {
    if (!nonEmptyString(key) || !Number.isSafeInteger(count) || count < 0) {
      throw new Error(`${source} has invalid count for ${JSON.stringify(key)}`)
    }
  }
}

export function nonEmptyString(value) {
  return typeof value === "string" && value.trim().length > 0
}
