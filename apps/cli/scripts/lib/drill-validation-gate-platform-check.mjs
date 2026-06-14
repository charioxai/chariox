import { readFile } from "node:fs/promises"
import path from "node:path"

import { verifyDrillPlatformBundle } from "./drill-platform-bundle.mjs"

export async function platformValidationGateCheck(
  platformBundleDir,
  {
    requiredCoverageAreas = [],
    requiredRuntimeSignals = [],
    requiredFailureClassifications = [],
  } = {},
) {
  if (!platformBundleDir) {
    if (requiredCoverageAreas.length > 0 || requiredRuntimeSignals.length > 0 || requiredFailureClassifications.length > 0) {
      return {
        status: "failed",
        dir: null,
        requiredCoverageAreas: [...requiredCoverageAreas],
        missingCoverageAreas: [...requiredCoverageAreas],
        requiredRuntimeSignals: [...requiredRuntimeSignals],
        missingRuntimeSignals: [...requiredRuntimeSignals],
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
    const missingFailureClassifications = missingRequiredFailureClassifications(failureTaxonomy, requiredFailureClassifications)
    const errors = [
      ...(missingCoverageAreas.length > 0 ? [`missing platform coverage areas: ${missingCoverageAreas.join(", ")}`] : []),
      ...(missingRuntimeSignals.length > 0 ? [`missing runtime signals: ${missingRuntimeSignals.join(", ")}`] : []),
      ...(missingFailureClassifications.length > 0 ? [`missing failure classifications: ${missingFailureClassifications.join(", ")}`] : []),
    ]
    return {
      status: errors.length === 0 ? "passed" : "failed",
      dir: platformBundleDir,
      requiredCoverageAreas: [...requiredCoverageAreas],
      missingCoverageAreas,
      requiredRuntimeSignals: [...requiredRuntimeSignals],
      missingRuntimeSignals,
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
      requiredMatrices: [...(preset.requiredMatrices ?? [])],
      requiredRuntimeSignals: [...(preset.requiredRuntimeSignals ?? [])],
      requiredFailureClassifications: [...(preset.requiredFailureClassifications ?? [])],
      requiredMatrixRuntimeSignals: [...(preset.requiredMatrixRuntimeSignals ?? [])],
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
  if (taxonomy.schema !== "arroba.drill.failure_taxonomy.v1") {
    throw new Error(`failure taxonomy ${target} has unsupported schema ${JSON.stringify(taxonomy.schema)}`)
  }
  if (taxonomy.target !== target) {
    throw new Error(`failure taxonomy ${target} has invalid target ${JSON.stringify(taxonomy.target)}`)
  }
  if (!Array.isArray(taxonomy.classifications)) {
    throw new Error(`failure taxonomy ${target} has invalid classifications`)
  }
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

function missingRequiredFailureClassifications(failureTaxonomy, requiredFailureClassifications) {
  const drill = new Set(failureTaxonomy.drill ?? [])
  const scenario = new Set(failureTaxonomy.scenario ?? [])
  return requiredFailureClassifications.filter((classification) => !drill.has(classification) || !scenario.has(classification))
}
