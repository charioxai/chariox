import path from "node:path"
import { pathToFileURL } from "node:url"

import {
  drillFailureOwnerForClassification,
  drillFailureTaxonomyManifest,
  isKnownDrillFailureClassification,
} from "./drill-failure-taxonomy.mjs"

const CLOUD_CONTEXT_OWNER_OVERRIDES = Object.freeze({
  "docker-runtime": Object.freeze(["local-machine", "worker-kernel"]),
})

export async function verifyDrillFailureTaxonomyRegistryParity({ cloudRoot }) {
  const cloudRegistryPath = path.join(cloudRoot, "scripts", "lib", "cloud-failure-taxonomy.mjs")
  let cloudModule
  try {
    cloudModule = await import(pathToFileURL(cloudRegistryPath).href)
  } catch (error) {
    throw new Error(`failure taxonomy registry parity requires Cloud registry at ${cloudRegistryPath}: ${error.message}`)
  }
  if (typeof cloudModule.cloudFailureTaxonomyManifest !== "function") {
    throw new Error(`failure taxonomy registry parity requires cloudFailureTaxonomyManifest in ${cloudRegistryPath}`)
  }
  const ossClassifications = failureClassificationMap(
    drillFailureTaxonomyManifest(),
    "OSS failure taxonomy registry",
  )
  const cloudClassifications = failureClassificationMap(
    cloudModule.cloudFailureTaxonomyManifest(),
    "Cloud failure taxonomy registry",
  )
  const failures = []
  for (const [kind, classification] of Object.entries(cloudClassifications)) {
    if (!isKnownDrillFailureClassification(kind)) {
      failures.push(`${kind}: unknown in OSS failure taxonomy`)
      continue
    }
    const expectedOwner = drillFailureOwnerForClassification(kind)
    if (classification.owner !== expectedOwner && !allowedCloudOwnerOverride(kind, expectedOwner, classification.owner)) {
      failures.push(`${kind}: owner OSS=${expectedOwner} Cloud=${classification.owner}`)
    }
    if (!ossClassifications[kind]) {
      failures.push(`${kind}: missing OSS failure taxonomy manifest entry`)
    }
  }
  if (failures.length > 0) {
    throw new Error(`failure taxonomy registry parity failed: ${failures.join("; ")}`)
  }
}

function failureClassificationMap(manifest, source) {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`${source} is not an object`)
  }
  if (manifest.schema !== "arroba.drill.failure_taxonomy.v1") {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  if (manifest.target !== "scenario") {
    throw new Error(`${source} has invalid target ${JSON.stringify(manifest.target)}`)
  }
  if (!Array.isArray(manifest.classifications)) {
    throw new Error(`${source} has invalid classifications`)
  }
  const classificationEntries = []
  const seenKinds = new Set()
  for (const [index, classification] of manifest.classifications.entries()) {
    if (!classification || typeof classification !== "object" || Array.isArray(classification)) {
      throw new Error(`${source}.classifications[${index}] is not an object`)
    }
    if (typeof classification.kind !== "string" || classification.kind.length === 0) {
      throw new Error(`${source}.classifications[${index}] has invalid kind`)
    }
    if (seenKinds.has(classification.kind)) {
      throw new Error(`${source}.classifications[${index}] duplicates classification ${JSON.stringify(classification.kind)}`)
    }
    seenKinds.add(classification.kind)
    if (typeof classification.owner !== "string" || classification.owner.length === 0) {
      throw new Error(`${source}.classifications[${index}] has invalid owner`)
    }
    if (typeof classification.nextAction !== "string" || classification.nextAction.length === 0) {
      throw new Error(`${source}.classifications[${index}] has invalid nextAction`)
    }
    classificationEntries.push([classification.kind, {
      nextAction: classification.nextAction,
      owner: classification.owner,
    }])
  }
  return Object.fromEntries(classificationEntries.sort(([left], [right]) => left.localeCompare(right)))
}

function allowedCloudOwnerOverride(kind, ossOwner, cloudOwner) {
  return (CLOUD_CONTEXT_OWNER_OVERRIDES[kind] ?? []).includes(ossOwner)
    && (CLOUD_CONTEXT_OWNER_OVERRIDES[kind] ?? []).includes(cloudOwner)
}
