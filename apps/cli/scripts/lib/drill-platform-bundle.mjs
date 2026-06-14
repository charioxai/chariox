import { mkdir, readFile, writeFile } from "node:fs/promises"
import path from "node:path"

import {
  DRILL_FAILURE_CLASSIFICATION_KINDS,
  drillFailureTaxonomyManifest,
} from "./drill-failure-taxonomy.mjs"
import { drillValidationSuiteManifest } from "./drill-validation-suite.mjs"

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

  for (const file of files) {
    await writeFile(
      path.join(outputDir, file.path),
      `${JSON.stringify(file.contents, null, 2)}\n`,
      "utf8",
    )
  }

  const bundle = {
    schema: DRILL_PLATFORM_BUNDLE_SCHEMA,
    outputDir,
    artifacts: files.map((file) => ({
      path: file.path,
      schema: file.contents.schema,
    })),
  }
  await writeFile(path.join(outputDir, "index.json"), `${JSON.stringify(bundle, null, 2)}\n`, "utf8")
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
  }
  assertRequiredBundleArtifacts(bundle.artifacts)
  for (const artifact of bundle.artifacts) {
    const contents = JSON.parse(await readFile(path.join(outputDir, artifact.path), "utf8"))
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
    if (typeof entry.owner !== "string" || entry.owner.length === 0) {
      throw new Error(`failure taxonomy ${expectedTarget} artifact has invalid owner`)
    }
    if (typeof entry.nextAction !== "string" || entry.nextAction.length === 0) {
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
