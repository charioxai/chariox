import { readFile } from "node:fs/promises"
import path from "node:path"

import { validateDrillFailureTaxonomyManifest } from "./drill-failure-taxonomy.mjs"
import { verifyDrillPlatformBundle } from "./drill-platform-bundle.mjs"

export async function platformValidationGateCheck(
  platformBundleDir,
  {
    requiredCoverageAreas = [],
    requiredRuntimeSignals = [],
    requiredRuntimeSignalOwners = [],
    requiredFailureClassifications = [],
  } = {},
) {
  if (!platformBundleDir) {
    if (requiredCoverageAreas.length > 0 || requiredRuntimeSignals.length > 0 || requiredRuntimeSignalOwners.length > 0 || requiredFailureClassifications.length > 0) {
      return {
        status: "failed",
        dir: null,
        requiredCoverageAreas: [...requiredCoverageAreas],
        missingCoverageAreas: [...requiredCoverageAreas],
        requiredRuntimeSignals: [...requiredRuntimeSignals],
        missingRuntimeSignals: [...requiredRuntimeSignals],
        requiredRuntimeSignalOwners: [...requiredRuntimeSignalOwners],
        missingRuntimeSignalOwners: [...requiredRuntimeSignalOwners],
        requiredFailureClassifications: [...requiredFailureClassifications],
        missingFailureClassifications: [...requiredFailureClassifications],
        error: "no platform bundle provided",
      }
    }
    return {
      status: "skipped",
      dir: null,
      requiredCoverageAreas: [],
      missingCoverageAreas: [],
      requiredRuntimeSignals: [],
      missingRuntimeSignals: [],
      requiredRuntimeSignalOwners: [],
      missingRuntimeSignalOwners: [],
      requiredFailureClassifications: [],
      missingFailureClassifications: [],
    }
  }
  try {
    const bundle = await verifyDrillPlatformBundle(platformBundleDir)
    const validationSuite = await readPlatformBundleValidationSuite(platformBundleDir)
    const runtimeSignals = await readPlatformBundleRuntimeSignals(platformBundleDir)
    const failureTaxonomy = await readPlatformBundleFailureTaxonomy(platformBundleDir)
    const missingCoverageAreas = missingRequiredPlatformCoverageAreas(validationSuite, requiredCoverageAreas)
    const missingRuntimeSignals = missingRequiredRuntimeSignals(runtimeSignals, requiredRuntimeSignals)
    const missingRuntimeSignalOwners = missingRequiredRuntimeSignalOwners(runtimeSignals, requiredRuntimeSignalOwners)
    const missingFailureClassifications = missingRequiredFailureClassifications(failureTaxonomy, requiredFailureClassifications)
    const errors = [
      ...(missingCoverageAreas.length > 0 ? [`missing platform coverage areas: ${missingCoverageAreas.join(", ")}`] : []),
      ...(missingRuntimeSignals.length > 0 ? [`missing runtime signals: ${missingRuntimeSignals.join(", ")}`] : []),
      ...(missingRuntimeSignalOwners.length > 0 ? [`missing runtime signal owners: ${missingRuntimeSignalOwners.join(", ")}`] : []),
      ...(missingFailureClassifications.length > 0 ? [`missing failure classifications: ${missingFailureClassifications.join(", ")}`] : []),
    ]
    return {
      status: errors.length === 0 ? "passed" : "failed",
      dir: platformBundleDir,
      requiredCoverageAreas: [...requiredCoverageAreas],
      missingCoverageAreas,
      requiredRuntimeSignals: [...requiredRuntimeSignals],
      missingRuntimeSignals,
      requiredRuntimeSignalOwners: [...requiredRuntimeSignalOwners],
      missingRuntimeSignalOwners,
      requiredFailureClassifications: [...requiredFailureClassifications],
      missingFailureClassifications,
      ...(errors.length > 0 ? { error: errors.join("; ") } : {}),
      artifacts: bundle.artifacts.map((artifact) => ({
        path: artifact.path,
        schema: artifact.schema,
        sha256: artifact.sha256,
        sizeBytes: artifact.sizeBytes,
      })),
      validationSuite,
      runtimeSignals,
      failureTaxonomy,
    }
  } catch (error) {
    return {
      status: "failed",
      dir: platformBundleDir,
      requiredCoverageAreas: [...requiredCoverageAreas],
      missingCoverageAreas: [...requiredCoverageAreas],
      requiredRuntimeSignals: [...requiredRuntimeSignals],
      missingRuntimeSignals: [...requiredRuntimeSignals],
      requiredRuntimeSignalOwners: [...requiredRuntimeSignalOwners],
      missingRuntimeSignalOwners: [...requiredRuntimeSignalOwners],
      requiredFailureClassifications: [...requiredFailureClassifications],
      missingFailureClassifications: [...requiredFailureClassifications],
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

async function readPlatformBundleRuntimeSignals(platformBundleDir) {
  const manifest = JSON.parse(await readFile(path.join(platformBundleDir, "runtime-signals.json"), "utf8"))
  if (manifest.schema !== "arroba.drill.runtime_signals.v1") {
    throw new Error(`runtime signals has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  if (!Array.isArray(manifest.signals)) {
    throw new Error("runtime signals has invalid signals")
  }
  return manifest.signals
    .map((signal) => ({
      id: signal.id,
      owner: signal.owner,
    }))
    .sort((left, right) => left.id.localeCompare(right.id))
}

async function readPlatformBundleValidationSuite(platformBundleDir) {
  const suite = JSON.parse(await readFile(path.join(platformBundleDir, "validation-suite.json"), "utf8"))
  return {
    testCount: suite.testCount,
    coverageAreas: suite.coverage.map((area) => ({
      id: area.id,
      testCount: area.testCount,
    })),
    validationPresets: (suite.validationPresets ?? []).map((preset) => ({
      name: preset.name,
      requiredArtifactCoverageAreas: [...(preset.requiredArtifactCoverageAreas ?? [])],
      requiredArtifactSchemas: [...(preset.requiredArtifactSchemas ?? [])],
      requiredArtifactKinds: [...(preset.requiredArtifactKinds ?? [])],
      requiredArtifactGeneratedEvidenceKinds: [...(preset.requiredArtifactGeneratedEvidenceKinds ?? [])],
      requiredArtifactGeneratedMatrixArtifactIndexes: [...(preset.requiredArtifactGeneratedMatrixArtifactIndexes ?? [])],
      requiredArtifactGeneratedMatrixLimitations: [...(preset.requiredArtifactGeneratedMatrixLimitations ?? [])],
      requiredArtifactGeneratedMatrixNames: [...(preset.requiredArtifactGeneratedMatrixNames ?? [])],
      requiredArtifactGeneratedMatrixRepos: [...(preset.requiredArtifactGeneratedMatrixRepos ?? [])],
      requiredArtifactEvidenceRepos: [...(preset.requiredArtifactEvidenceRepos ?? [])],
      requiredArtifactProviderAccountAliases: [...(preset.requiredArtifactProviderAccountAliases ?? [])],
      requiredArtifactExitCriterionStatuses: [...(preset.requiredArtifactExitCriterionStatuses ?? [])],
      requiredArtifactIncompleteExitCriterionStatuses: [...(preset.requiredArtifactIncompleteExitCriterionStatuses ?? [])],
      requiredMatrices: [...(preset.requiredMatrices ?? [])],
      requiredRuntimeSignals: [...(preset.requiredRuntimeSignals ?? [])],
      requiredFailureClassifications: [...(preset.requiredFailureClassifications ?? [])],
      requiredMatrixRuntimeSignals: [...(preset.requiredMatrixRuntimeSignals ?? [])],
      requiredDeploymentPresets: [...(preset.requiredDeploymentPresets ?? [])],
      requiredProviders: [...(preset.requiredProviders ?? [])],
      requiredScenarios: [...(preset.requiredScenarios ?? [])],
      requiredGeneratedEvidenceKinds: [...(preset.requiredGeneratedEvidenceKinds ?? [])],
      requiredGeneratedMatrixArtifactIndexes: [...(preset.requiredGeneratedMatrixArtifactIndexes ?? [])],
      requiredGeneratedMatrixLimitations: [...(preset.requiredGeneratedMatrixLimitations ?? [])],
      requiredGeneratedValidationSuiteArtifactIndexes: [...(preset.requiredGeneratedValidationSuiteArtifactIndexes ?? [])],
      requiredGeneratedValidationSuiteFailureRoots: [...(preset.requiredGeneratedValidationSuiteFailureRoots ?? [])],
    })),
  }
}

async function readPlatformBundleFailureTaxonomy(platformBundleDir) {
  const [drill, scenario] = await Promise.all([
    readPlatformBundleFailureTaxonomyKinds(platformBundleDir, "drill"),
    readPlatformBundleFailureTaxonomyKinds(platformBundleDir, "scenario"),
  ])
  return { drill, scenario }
}

async function readPlatformBundleFailureTaxonomyKinds(platformBundleDir, target) {
  const taxonomy = JSON.parse(await readFile(path.join(platformBundleDir, `failure-taxonomy-${target}.json`), "utf8"))
  validateDrillFailureTaxonomyManifest(taxonomy, `failure taxonomy ${target}`, { target })
  return [...new Set(taxonomy.classifications
    .map((entry) => entry?.kind)
    .filter((kind) => typeof kind === "string"))].sort()
}

function missingRequiredPlatformCoverageAreas(validationSuite, requiredCoverageAreas) {
  const present = new Set((validationSuite.coverageAreas ?? []).map((area) => area.id))
  return requiredCoverageAreas.filter((area) => !present.has(area))
}

function missingRequiredRuntimeSignals(runtimeSignals, requiredRuntimeSignals) {
  const present = new Set((runtimeSignals ?? []).map((signal) => signal.id))
  return requiredRuntimeSignals.filter((signal) => !present.has(signal))
}

function missingRequiredRuntimeSignalOwners(runtimeSignals, requiredRuntimeSignalOwners) {
  const present = new Set((runtimeSignals ?? []).map((signal) => signal.owner))
  return requiredRuntimeSignalOwners.filter((owner) => !present.has(owner))
}

function missingRequiredFailureClassifications(failureTaxonomy, requiredFailureClassifications) {
  const drill = new Set(failureTaxonomy.drill ?? [])
  const scenario = new Set(failureTaxonomy.scenario ?? [])
  return requiredFailureClassifications.filter((classification) => !drill.has(classification) || !scenario.has(classification))
}
