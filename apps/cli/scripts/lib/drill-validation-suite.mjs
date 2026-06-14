import { access, readdir } from "node:fs/promises"
import path from "node:path"
import {
  DRILL_RUNTIME_SIGNAL_OWNERS,
  drillRuntimeSignalOwnersFor,
  drillRuntimeSignalsManifest,
  isKnownDrillRuntimeSignal,
  validateDrillRuntimeSignalsManifest,
} from "./drill-runtime-signals.mjs"
import { describeDrillValidationGatePresets } from "./drill-validation-gate-presets.mjs"

export const SHARED_DRILL_TEST_PATHS = Object.freeze([
  "apps/cli/scripts/drill-artifact-index-summary.test.mjs",
  "apps/cli/scripts/drill-cross-repo-validation-gate.test.mjs",
  "apps/cli/scripts/drill-distributed-runtime-gate.test.mjs",
  "apps/cli/scripts/drill-failure-summary.test.mjs",
  "apps/cli/scripts/drill-failure-taxonomy.test.mjs",
  "apps/cli/scripts/drill-matrix-report-summary.test.mjs",
  "apps/cli/scripts/drill-platform-bundle.test.mjs",
  "apps/cli/scripts/drill-validation-gate-summary.test.mjs",
  "apps/cli/scripts/drill-validation-gate.test.mjs",
  "apps/cli/scripts/drill-validation-suite.test.mjs",
  "apps/cli/scripts/lib/drill-aggregate-actions.test.mjs",
  "apps/cli/scripts/lib/drill-artifacts.test.mjs",
  "apps/cli/scripts/lib/drill-child-process.test.mjs",
  "apps/cli/scripts/lib/drill-cli-args.test.mjs",
  "apps/cli/scripts/lib/drill-distributed-runtime-evidence.test.mjs",
  "apps/cli/scripts/lib/drill-environment-presets.test.mjs",
  "apps/cli/scripts/lib/drill-failure-manifest.test.mjs",
  "apps/cli/scripts/lib/drill-failure-taxonomy.test.mjs",
  "apps/cli/scripts/lib/drill-history-outline.test.mjs",
  "apps/cli/scripts/lib/drill-json-discovery.test.mjs",
  "apps/cli/scripts/lib/drill-matrix-report.test.mjs",
  "apps/cli/scripts/lib/drill-matrix-runner.test.mjs",
  "apps/cli/scripts/lib/drill-platform-bundle.test.mjs",
  "apps/cli/scripts/lib/drill-provider-profiles.test.mjs",
  "apps/cli/scripts/lib/drill-runtime-helpers.test.mjs",
  "apps/cli/scripts/lib/drill-runtime-signals.test.mjs",
  "apps/cli/scripts/lib/drill-secrets.test.mjs",
  "apps/cli/scripts/lib/drill-time.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-aggregate.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-args.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-artifact-check.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-configuration-check.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-discovery.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-failure-check.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-matrix-check.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-next-actions.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-platform-check.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-presets.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-report.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-runtime-signal-metadata.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate-summary-format.test.mjs",
  "apps/cli/scripts/lib/drill-validation-gate.test.mjs",
  "apps/cli/scripts/lib/drill-validation-suite.test.mjs",
  "apps/cli/scripts/lib/remote-home-extension-hetzner-helpers.test.mjs",
  "apps/cli/scripts/lib/workspace-live-sync-fixtures.test.mjs",
  "apps/cli/scripts/live-native-provider-tui-matrix-drill.test.mjs",
  "apps/cli/scripts/live-remote-agent-runtime-matrix-drill.test.mjs",
  "apps/cli/scripts/live-remote-home-extension-matrix-drill.test.mjs",
  "apps/cli/scripts/live-slice-runtime-matrix-drill.test.mjs",
  "apps/cli/scripts/live-workspace-live-sync-matrix-drill.test.mjs",
])

export const DRILL_VALIDATION_COVERAGE_AREAS = Object.freeze([
  {
    id: "distributed-observability",
    description: "Runtime signal contracts for authority, health, projection, sync, slice, provider, and relay diagnostics.",
    testPaths: Object.freeze([
      "apps/cli/scripts/lib/drill-runtime-signals.test.mjs",
    ]),
  },
  {
    id: "artifact-contracts",
    description: "Artifact indexes, failure manifests, aggregate actions, summaries, and platform bundles.",
    testPaths: Object.freeze([
      "apps/cli/scripts/drill-artifact-index-summary.test.mjs",
      "apps/cli/scripts/drill-failure-summary.test.mjs",
      "apps/cli/scripts/drill-platform-bundle.test.mjs",
      "apps/cli/scripts/lib/drill-aggregate-actions.test.mjs",
      "apps/cli/scripts/lib/drill-artifacts.test.mjs",
      "apps/cli/scripts/lib/drill-failure-manifest.test.mjs",
      "apps/cli/scripts/lib/drill-json-discovery.test.mjs",
      "apps/cli/scripts/lib/drill-platform-bundle.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-artifact-check.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-failure-check.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-platform-check.test.mjs",
    ]),
  },
  {
    id: "failure-diagnostics",
    description: "Failure classification, owners, next actions, and user-facing taxonomy summaries.",
    testPaths: Object.freeze([
      "apps/cli/scripts/drill-failure-taxonomy.test.mjs",
      "apps/cli/scripts/lib/drill-child-process.test.mjs",
      "apps/cli/scripts/lib/drill-failure-taxonomy.test.mjs",
    ]),
  },
  {
    id: "matrix-validation",
    description: "Matrix runners, report validation, report summaries, and validation gates.",
    testPaths: Object.freeze([
      "apps/cli/scripts/drill-matrix-report-summary.test.mjs",
      "apps/cli/scripts/drill-cross-repo-validation-gate.test.mjs",
      "apps/cli/scripts/drill-distributed-runtime-gate.test.mjs",
      "apps/cli/scripts/drill-validation-gate-summary.test.mjs",
      "apps/cli/scripts/drill-validation-gate.test.mjs",
      "apps/cli/scripts/live-native-provider-tui-matrix-drill.test.mjs",
      "apps/cli/scripts/live-remote-agent-runtime-matrix-drill.test.mjs",
      "apps/cli/scripts/live-remote-home-extension-matrix-drill.test.mjs",
      "apps/cli/scripts/live-slice-runtime-matrix-drill.test.mjs",
      "apps/cli/scripts/live-workspace-live-sync-matrix-drill.test.mjs",
      "apps/cli/scripts/lib/drill-distributed-runtime-evidence.test.mjs",
      "apps/cli/scripts/lib/drill-matrix-report.test.mjs",
      "apps/cli/scripts/lib/drill-matrix-runner.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-aggregate.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-args.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-configuration-check.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-discovery.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-matrix-check.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-next-actions.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-presets.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-report.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-runtime-signal-metadata.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate-summary-format.test.mjs",
      "apps/cli/scripts/lib/drill-validation-gate.test.mjs",
      "apps/cli/scripts/lib/workspace-live-sync-fixtures.test.mjs",
    ]),
  },
  {
    id: "runtime-fixtures",
    description: "Reusable runtime, provider, environment, time, and history-outline helpers.",
    testPaths: Object.freeze([
      "apps/cli/scripts/lib/drill-cli-args.test.mjs",
      "apps/cli/scripts/lib/drill-environment-presets.test.mjs",
      "apps/cli/scripts/lib/drill-history-outline.test.mjs",
      "apps/cli/scripts/lib/drill-provider-profiles.test.mjs",
      "apps/cli/scripts/lib/remote-home-extension-hetzner-helpers.test.mjs",
      "apps/cli/scripts/lib/drill-runtime-helpers.test.mjs",
      "apps/cli/scripts/lib/drill-secrets.test.mjs",
      "apps/cli/scripts/lib/drill-time.test.mjs",
    ]),
  },
  {
    id: "suite-contract",
    description: "The validation suite manifest and command contract itself.",
    testPaths: Object.freeze([
      "apps/cli/scripts/drill-validation-suite.test.mjs",
      "apps/cli/scripts/lib/drill-validation-suite.test.mjs",
    ]),
  },
])

export function drillValidationSuiteArgs({ testPaths = SHARED_DRILL_TEST_PATHS } = {}) {
  return ["--test", ...testPaths]
}

export function drillValidationSuiteCommand({ nodeCommand = "node", testPaths = SHARED_DRILL_TEST_PATHS } = {}) {
  return [nodeCommand, ...drillValidationSuiteArgs({ testPaths })]
    .map((part) => (/[ "'\\]/.test(part) ? JSON.stringify(part) : part))
    .join(" ")
}

export function drillValidationSuiteManifest({
  nodeCommand = "node",
  schema = "arroba.drill.validation_suite.v1",
  testPaths = SHARED_DRILL_TEST_PATHS,
  coverageAreas = DRILL_VALIDATION_COVERAGE_AREAS,
  validationPresets = describeDrillValidationGatePresets(),
} = {}) {
  const coverage = validationSuiteCoverage({ coverageAreas, testPaths })
  return {
    schema,
    testCount: testPaths.length,
    command: drillValidationSuiteCommand({ nodeCommand, testPaths }),
    coverage,
    runtimeSignalsManifest: drillRuntimeSignalsManifest(),
    validationPresets: normalizeValidationSuitePresetContracts(validationPresets),
    testPaths: [...testPaths],
  }
}

export function drillValidationSuiteArtifactMetadata(suiteArtifact) {
  const manifest = suiteArtifact?.schema === "arroba.drill.validation_suite_run.v1"
    ? suiteArtifact.manifest
    : suiteArtifact
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error("validation suite artifact metadata requires a manifest or run report")
  }
  const coverageAreas = sortedStringArray(
    (manifest.coverage ?? []).map((area) => area?.id).filter((id) => typeof id === "string" && id.length > 0),
    "coverageAreas",
  )
  const validationPresets = sortedStringArray(
    (manifest.validationPresets ?? []).map((preset) => preset?.name).filter((name) => typeof name === "string" && name.length > 0),
    "validationPresets",
  )
  const runtimeSignals = sortedStringArray(
    (manifest.validationPresets ?? []).flatMap((preset) => [
      ...(preset?.requiredRuntimeSignals ?? []),
      ...(preset?.requiredMatrixRuntimeSignals ?? []),
    ]),
    "runtimeSignals",
  )
  if (runtimeSignals.length > 0) {
    validateDrillRuntimeSignalsManifest(manifest.runtimeSignalsManifest, "validation suite runtimeSignalsManifest")
  }
  return {
    drill: "validation-suite",
    ...(suiteArtifact?.schema === "arroba.drill.validation_suite_run.v1" ? { status: suiteArtifact.status } : {}),
    tests: manifest.testCount,
    owners: "validation-platform",
    classifications: "validation-suite",
    artifactKinds: suiteArtifact?.schema === "arroba.drill.validation_suite_run.v1"
      ? "validation-suite-run"
      : "validation-suite",
    evidenceRepos: "oss",
    ...(coverageAreas.length > 0 ? { coverageAreas: coverageAreas.join(",") } : {}),
    ...(validationPresets.length > 0 ? { validationPresets: validationPresets.join(",") } : {}),
    ...(runtimeSignals.length > 0
      ? {
        runtimeSignals: runtimeSignals.join(","),
        runtimeSignalOwners: drillRuntimeSignalOwnersFor(runtimeSignals).join(","),
      }
      : {}),
  }
}

export function validationSuiteCoverage({
  coverageAreas = DRILL_VALIDATION_COVERAGE_AREAS,
  testPaths = SHARED_DRILL_TEST_PATHS,
} = {}) {
  validateValidationSuiteCoverage({ coverageAreas, testPaths })
  return coverageAreas.map((area) => ({
    id: area.id,
    description: area.description,
    testCount: area.testPaths.length,
    testPaths: [...area.testPaths],
  }))
}

export function validateValidationSuiteCoverage({
  coverageAreas = DRILL_VALIDATION_COVERAGE_AREAS,
  testPaths = SHARED_DRILL_TEST_PATHS,
} = {}) {
  const suitePaths = new Set(testPaths)
  const coveredPaths = new Set()
  const areaIds = new Set()
  for (const area of coverageAreas) {
    if (!area || typeof area !== "object") throw new Error("validation suite coverage has invalid area")
    if (typeof area.id !== "string" || area.id.length === 0) throw new Error("validation suite coverage area has invalid id")
    if (areaIds.has(area.id)) throw new Error(`validation suite coverage has duplicate area id ${area.id}`)
    areaIds.add(area.id)
    if (typeof area.description !== "string" || area.description.length === 0) {
      throw new Error(`validation suite coverage area ${area.id} has invalid description`)
    }
    if (!Array.isArray(area.testPaths) || area.testPaths.length === 0) {
      throw new Error(`validation suite coverage area ${area.id} has invalid testPaths`)
    }
    for (const testPath of area.testPaths) {
      if (!suitePaths.has(testPath)) {
        throw new Error(`validation suite coverage area ${area.id} references non-suite test ${testPath}`)
      }
      if (coveredPaths.has(testPath)) {
        throw new Error(`validation suite coverage references duplicate test ${testPath}`)
      }
      coveredPaths.add(testPath)
    }
  }
  const missingCoverage = [...suitePaths].filter((testPath) => !coveredPaths.has(testPath))
  if (missingCoverage.length > 0) {
    throw new Error(`validation suite tests missing coverage areas: ${missingCoverage.join(", ")}`)
  }
}

export function normalizeValidationSuitePresetContracts(validationPresets) {
  if (!Array.isArray(validationPresets)) {
    throw new Error("validation suite presets must be an array")
  }
  return validationPresets.map((preset, index) => normalizeValidationSuitePresetContract(preset, index))
    .sort((left, right) => left.name.localeCompare(right.name))
}

function normalizeValidationSuitePresetContract(preset, index) {
  if (!preset || typeof preset !== "object" || Array.isArray(preset)) {
    throw new Error(`validation suite preset ${index} is not an object`)
  }
  if (typeof preset.name !== "string" || preset.name.length === 0) {
    throw new Error(`validation suite preset ${index} has invalid name`)
  }
  if (typeof preset.description !== "string" || preset.description.length === 0) {
    throw new Error(`validation suite preset ${preset.name} has invalid description`)
  }
  return {
    name: preset.name,
    description: preset.description,
    requiredPlatformCoverageAreas: sortedStringArray(preset.requiredPlatformCoverageAreas, `${preset.name}.requiredPlatformCoverageAreas`),
    requiredArtifactCoverageAreas: sortedStringArray(preset.requiredArtifactCoverageAreas, `${preset.name}.requiredArtifactCoverageAreas`),
    requiredArtifactSchemas: sortedStringArray(preset.requiredArtifactSchemas, `${preset.name}.requiredArtifactSchemas`),
    requiredArtifactKinds: sortedStringArray(preset.requiredArtifactKinds, `${preset.name}.requiredArtifactKinds`),
    requiredArtifactGeneratedEvidenceKinds: sortedStringArray(preset.requiredArtifactGeneratedEvidenceKinds, `${preset.name}.requiredArtifactGeneratedEvidenceKinds`),
    requiredArtifactEvidenceRepos: sortedStringArray(preset.requiredArtifactEvidenceRepos, `${preset.name}.requiredArtifactEvidenceRepos`),
    requiredArtifactRuntimeSignals: sortedRuntimeSignalArray(preset.requiredArtifactRuntimeSignals, `${preset.name}.requiredArtifactRuntimeSignals`),
    requiredArtifactRuntimeSignalOwners: sortedRuntimeSignalOwnerArray(preset.requiredArtifactRuntimeSignalOwners, `${preset.name}.requiredArtifactRuntimeSignalOwners`),
    requiredArtifactOwners: sortedStringArray(preset.requiredArtifactOwners, `${preset.name}.requiredArtifactOwners`),
    requiredArtifactClassifications: sortedStringArray(preset.requiredArtifactClassifications, `${preset.name}.requiredArtifactClassifications`),
    requiredRuntimeSignals: sortedRuntimeSignalArray(preset.requiredRuntimeSignals, `${preset.name}.requiredRuntimeSignals`),
    requiredFailureClassifications: sortedStringArray(preset.requiredFailureClassifications, `${preset.name}.requiredFailureClassifications`),
    requiredMatrices: sortedStringArray(preset.requiredMatrices, `${preset.name}.requiredMatrices`),
    requiredMatrixClassifications: sortedStringArray(preset.requiredMatrixClassifications, `${preset.name}.requiredMatrixClassifications`),
    requiredMatrixRuntimeSignals: sortedRuntimeSignalArray(preset.requiredMatrixRuntimeSignals, `${preset.name}.requiredMatrixRuntimeSignals`),
    requiredDeploymentPresets: sortedStringArray(preset.requiredDeploymentPresets, `${preset.name}.requiredDeploymentPresets`),
    requiredProviders: sortedStringArray(preset.requiredProviders, `${preset.name}.requiredProviders`),
    requiredScenarios: sortedStringArray(preset.requiredScenarios, `${preset.name}.requiredScenarios`),
  }
}

function sortedStringArray(value, source) {
  if (value === undefined) return []
  if (!Array.isArray(value)) {
    throw new Error(`validation suite preset ${source} must be an array`)
  }
  for (const item of value) {
    if (typeof item !== "string" || item.length === 0) {
      throw new Error(`validation suite preset ${source} has invalid entry`)
    }
  }
  return [...new Set(value)].sort()
}

function sortedRuntimeSignalArray(value, source) {
  const signals = sortedStringArray(value, source)
  for (const [index, signal] of signals.entries()) {
    if (!isKnownDrillRuntimeSignal(signal)) {
      throw new Error(`validation suite preset ${source}[${index}] has unknown runtime signal ${JSON.stringify(signal)}`)
    }
  }
  return signals
}

function sortedRuntimeSignalOwnerArray(value, source) {
  const owners = sortedStringArray(value, source)
  for (const [index, owner] of owners.entries()) {
    if (!DRILL_RUNTIME_SIGNAL_OWNERS.includes(owner)) {
      throw new Error(`validation suite preset ${source}[${index}] has unknown runtime signal owner ${JSON.stringify(owner)}`)
    }
  }
  return owners
}

export async function findMissingDrillValidationSuitePaths({
  rootDir = process.cwd(),
  testPaths = SHARED_DRILL_TEST_PATHS,
} = {}) {
  const missing = []
  for (const testPath of testPaths) {
    const absolutePath = path.resolve(rootDir, testPath)
    try {
      await access(absolutePath)
    } catch {
      missing.push(testPath)
    }
  }
  return missing
}

export async function findUnlistedDrillValidationSuitePaths({
  rootDir = process.cwd(),
  scriptsDir = "apps/cli/scripts",
  testPaths = SHARED_DRILL_TEST_PATHS,
} = {}) {
  const discovered = await discoverDrillValidationSuiteTestPaths({ rootDir, scriptsDir })
  const listed = new Set(testPaths)
  return discovered.filter((testPath) => !listed.has(testPath))
}

export async function discoverDrillValidationSuiteTestPaths({
  rootDir = process.cwd(),
  scriptsDir = "apps/cli/scripts",
} = {}) {
  const found = []
  await collectDrillValidationSuiteTestPaths(path.resolve(rootDir, scriptsDir), rootDir, found)
  return found.sort()
}

async function collectDrillValidationSuiteTestPaths(dir, rootDir, found) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const fullPath = path.join(dir, entry.name)
    if (entry.isDirectory()) {
      await collectDrillValidationSuiteTestPaths(fullPath, rootDir, found)
    } else if (entry.isFile() && entry.name.endsWith(".test.mjs")) {
      found.push(path.relative(rootDir, fullPath).split(path.sep).join("/"))
    }
  }
}
