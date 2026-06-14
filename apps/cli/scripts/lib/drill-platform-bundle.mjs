import { createHash } from "node:crypto"
import { mkdir, readFile, writeFile } from "node:fs/promises"
import path from "node:path"

import {
  DRILL_FAILURE_CLASSIFICATION_KINDS,
  drillFailureNextActionForClassification,
  drillFailureOwnerForClassification,
  drillFailureTaxonomyManifest,
} from "./drill-failure-taxonomy.mjs"
import { writeDrillArtifactIndex } from "./drill-artifacts.mjs"
import {
  drillValidationSuiteManifest,
  normalizeValidationSuitePresetContracts,
  validateValidationSuiteCoverage,
} from "./drill-validation-suite.mjs"

export const DRILL_PLATFORM_BUNDLE_SCHEMA = "arroba.drill.platform_bundle.v1"
export const DRILL_PLATFORM_BUNDLE_ARTIFACTS = Object.freeze([
  {
    path: "failure-taxonomy-drill.json",
    schema: "arroba.drill.failure_taxonomy.v1",
  },
  {
    path: "failure-taxonomy-scenario.json",
    schema: "arroba.drill.failure_taxonomy.v1",
  },
  {
    path: "validation-suite.json",
    schema: "arroba.drill.validation_suite.v1",
  },
])

export async function writeDrillPlatformBundle(outputDir) {
  await mkdir(outputDir, { recursive: true })
  const files = [
    {
      path: "validation-suite.json",
      contents: drillValidationSuiteManifest(),
    },
    {
      path: "failure-taxonomy-scenario.json",
      contents: drillFailureTaxonomyManifest({ target: "scenario" }),
    },
    {
      path: "failure-taxonomy-drill.json",
      contents: drillFailureTaxonomyManifest({ target: "drill" }),
    },
  ]

  const artifactRecords = []
  for (const file of files) {
    const serialized = `${JSON.stringify(file.contents, null, 2)}\n`
    await writeFile(path.join(outputDir, file.path), serialized, "utf8")
    artifactRecords.push({
      path: file.path,
      schema: file.contents.schema,
      sha256: sha256(serialized),
      sizeBytes: Buffer.byteLength(serialized),
    })
  }

  const bundle = {
    schema: DRILL_PLATFORM_BUNDLE_SCHEMA,
    outputDir,
    artifacts: artifactRecords,
  }
  await writeFile(path.join(outputDir, "index.json"), `${JSON.stringify(bundle, null, 2)}\n`, "utf8")
  await writeDrillArtifactIndex({
    rootDir: outputDir,
    artifacts: ["index.json", ...artifactRecords.map((artifact) => artifact.path)],
    metadata: {
      drill: "platform-bundle",
      artifacts: artifactRecords.length,
    },
  })
  return bundle
}

export async function verifyDrillPlatformBundle(outputDir) {
  const indexPath = path.join(outputDir, "index.json")
  const bundle = JSON.parse(await readFile(indexPath, "utf8"))
  if (bundle.schema !== DRILL_PLATFORM_BUNDLE_SCHEMA) {
    throw new Error(`unsupported platform bundle schema ${JSON.stringify(bundle.schema)}`)
  }
  if (!Array.isArray(bundle.artifacts) || bundle.artifacts.length === 0) {
    throw new Error("platform bundle has no artifacts")
  }
  for (const artifact of bundle.artifacts) {
    if (!artifact || typeof artifact !== "object") throw new Error("platform bundle has invalid artifact entry")
    if (!relativeBundlePath(artifact.path)) {
      throw new Error(`platform bundle has unsafe artifact path ${JSON.stringify(artifact.path)}`)
    }
    if (typeof artifact.schema !== "string" || artifact.schema.length === 0) {
      throw new Error(`platform bundle artifact ${artifact.path} has invalid schema`)
    }
    if (!/^[a-f0-9]{64}$/.test(artifact.sha256)) {
      throw new Error(`platform bundle artifact ${artifact.path} has invalid sha256`)
    }
    if (!Number.isSafeInteger(artifact.sizeBytes) || artifact.sizeBytes < 0) {
      throw new Error(`platform bundle artifact ${artifact.path} has invalid sizeBytes`)
    }
  }
  assertRequiredBundleArtifacts(bundle.artifacts)
  for (const artifact of bundle.artifacts) {
    const serialized = await readFile(path.join(outputDir, artifact.path), "utf8")
    if (sha256(serialized) !== artifact.sha256) {
      throw new Error(`platform bundle artifact ${artifact.path} sha256 mismatch`)
    }
    if (Buffer.byteLength(serialized) !== artifact.sizeBytes) {
      throw new Error(`platform bundle artifact ${artifact.path} sizeBytes mismatch`)
    }
    const contents = JSON.parse(serialized)
    if (contents.schema !== artifact.schema) {
      throw new Error(
        `platform bundle artifact ${artifact.path} schema mismatch: `
          + `expected ${artifact.schema}, got ${JSON.stringify(contents.schema)}`,
      )
    }
    validateBundleArtifactContents(artifact.path, contents)
  }
  return bundle
}

function validateBundleArtifactContents(artifactPath, contents) {
  if (artifactPath === "validation-suite.json") {
    validateValidationSuiteArtifact(contents)
    return
  }
  if (artifactPath === "failure-taxonomy-scenario.json") {
    validateFailureTaxonomyArtifact(contents, "scenario")
    return
  }
  if (artifactPath === "failure-taxonomy-drill.json") {
    validateFailureTaxonomyArtifact(contents, "drill")
  }
}

function validateValidationSuiteArtifact(contents) {
  if (!Array.isArray(contents.testPaths) || contents.testPaths.length === 0) {
    throw new Error("validation-suite.json has invalid testPaths")
  }
  if (contents.testCount !== contents.testPaths.length) {
    throw new Error("validation-suite.json testCount does not match testPaths")
  }
  if (typeof contents.command !== "string" || !contents.command.includes("--test")) {
    throw new Error("validation-suite.json has invalid command")
  }
  if (!Array.isArray(contents.coverage) || contents.coverage.length === 0) {
    throw new Error("validation-suite.json has invalid coverage")
  }
  normalizeValidationSuitePresetContracts(contents.validationPresets)
  validateValidationSuiteCoverage({
    coverageAreas: contents.coverage.map((area) => ({
      id: area?.id,
      description: area?.description,
      testPaths: area?.testPaths,
    })),
    testPaths: contents.testPaths,
  })
  for (const area of contents.coverage) {
    if (area.testCount !== area.testPaths.length) {
      throw new Error(`validation-suite.json coverage area ${area.id} testCount does not match testPaths`)
    }
  }
}

function validateFailureTaxonomyArtifact(contents, expectedTarget) {
  if (contents.target !== expectedTarget) {
    throw new Error(`failure taxonomy artifact target mismatch: expected ${expectedTarget}, got ${JSON.stringify(contents.target)}`)
  }
  if (!Array.isArray(contents.classifications)) {
    throw new Error(`failure taxonomy ${expectedTarget} artifact has invalid classifications`)
  }
  const kinds = contents.classifications.map((entry) => entry?.kind).sort()
  if (JSON.stringify(kinds) !== JSON.stringify(DRILL_FAILURE_CLASSIFICATION_KINDS)) {
    throw new Error(`failure taxonomy ${expectedTarget} artifact classifications do not match taxonomy`)
  }
  for (const entry of contents.classifications) {
    if (typeof entry.kind !== "string" || entry.kind.length === 0) {
      throw new Error(`failure taxonomy ${expectedTarget} artifact has invalid kind`)
    }
    const expectedOwner = drillFailureOwnerForClassification(entry.kind)
    if (entry.owner !== expectedOwner) {
      throw new Error(`failure taxonomy ${expectedTarget} artifact has invalid owner`)
    }
    const expectedNextAction = drillFailureNextActionForClassification(entry.kind, { target: expectedTarget })
    if (entry.nextAction !== expectedNextAction) {
      throw new Error(`failure taxonomy ${expectedTarget} artifact has invalid nextAction`)
    }
  }
}

function assertRequiredBundleArtifacts(artifacts) {
  const actual = artifacts
    .map((artifact) => ({ path: artifact?.path, schema: artifact?.schema }))
    .sort((left, right) => String(left.path).localeCompare(String(right.path)))
  if (JSON.stringify(actual) !== JSON.stringify(DRILL_PLATFORM_BUNDLE_ARTIFACTS)) {
    throw new Error("platform bundle artifacts do not match required platform contracts")
  }
}

function relativeBundlePath(value) {
  return typeof value === "string"
    && value.length > 0
    && !path.isAbsolute(value)
    && !value.split(/[\\/]/).includes("..")
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex")
}
