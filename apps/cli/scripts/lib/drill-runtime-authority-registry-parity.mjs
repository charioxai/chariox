import path from "node:path"
import { pathToFileURL } from "node:url"

import {
  DRILL_RUNTIME_AUTHORITY_INVARIANTS_SCHEMA,
  drillRuntimeAuthorityManifest,
} from "./drill-runtime-authority-invariants.mjs"

const CLOUD_INVARIANT_ALIASES = Object.freeze({
  "cloud-control-plane-only": "relay-cloud-transport-only",
})

const CLOUD_CONTEXT_OWNER_OVERRIDES = Object.freeze({
  "relay-cloud-transport-only": Object.freeze(["cloud-deployment", "runtime-network"]),
})

export async function verifyDrillRuntimeAuthorityRegistryParity({ cloudRoot }) {
  const cloudRegistryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-authority-invariants.mjs")
  let cloudModule
  try {
    cloudModule = await import(pathToFileURL(cloudRegistryPath).href)
  } catch (error) {
    throw new Error(`runtime authority registry parity requires Cloud registry at ${cloudRegistryPath}: ${error.message}`)
  }
  if (typeof cloudModule.cloudRuntimeAuthorityManifest !== "function") {
    throw new Error(`runtime authority registry parity requires cloudRuntimeAuthorityManifest in ${cloudRegistryPath}`)
  }
  const ossInvariants = runtimeAuthorityInvariantMap(
    drillRuntimeAuthorityManifest(),
    "OSS runtime authority registry",
  )
  const cloudInvariants = runtimeAuthorityInvariantMap(
    cloudModule.cloudRuntimeAuthorityManifest(),
    "Cloud runtime authority registry",
    { aliases: CLOUD_INVARIANT_ALIASES },
  )
  const failures = []
  for (const [id, ossInvariant] of Object.entries(ossInvariants)) {
    const cloudInvariant = cloudInvariants[id]
    if (!cloudInvariant) {
      failures.push(`${id}: missing Cloud runtime authority invariant`)
      continue
    }
    if (
      cloudInvariant.owner !== ossInvariant.owner
      && !allowedCloudOwnerOverride(id, ossInvariant.owner, cloudInvariant.owner)
    ) {
      failures.push(`${id}: owner OSS=${ossInvariant.owner} Cloud=${cloudInvariant.owner}`)
    }
    if (JSON.stringify(cloudInvariant.requiredRuntimeSignals) !== JSON.stringify(ossInvariant.requiredRuntimeSignals)) {
      failures.push(`${id}: requiredRuntimeSignals OSS=${JSON.stringify(ossInvariant.requiredRuntimeSignals)} Cloud=${JSON.stringify(cloudInvariant.requiredRuntimeSignals)}`)
    }
  }
  for (const id of Object.keys(cloudInvariants)) {
    if (!ossInvariants[id]) {
      failures.push(`${id}: unknown in OSS runtime authority registry`)
    }
  }
  if (failures.length > 0) {
    throw new Error(`runtime authority registry parity failed: ${failures.join("; ")}`)
  }
}

function runtimeAuthorityInvariantMap(manifest, source, { aliases = {} } = {}) {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`${source} is not an object`)
  }
  if (manifest.schema !== DRILL_RUNTIME_AUTHORITY_INVARIANTS_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  if (!Array.isArray(manifest.invariants)) {
    throw new Error(`${source} has invalid invariants`)
  }
  const invariantEntries = []
  const seenIds = new Set()
  for (const [index, invariant] of manifest.invariants.entries()) {
    if (!invariant || typeof invariant !== "object" || Array.isArray(invariant)) {
      throw new Error(`${source}.invariants[${index}] is not an object`)
    }
    if (typeof invariant.id !== "string" || invariant.id.length === 0) {
      throw new Error(`${source}.invariants[${index}] has invalid id`)
    }
    const normalizedId = aliases[invariant.id] ?? invariant.id
    if (seenIds.has(normalizedId)) {
      throw new Error(`${source}.invariants[${index}] duplicates invariant ${JSON.stringify(normalizedId)}`)
    }
    seenIds.add(normalizedId)
    if (typeof invariant.owner !== "string" || invariant.owner.length === 0) {
      throw new Error(`${source}.invariants[${index}] has invalid owner`)
    }
    if (typeof invariant.description !== "string" || invariant.description.length === 0) {
      throw new Error(`${source}.invariants[${index}] has invalid description`)
    }
    if (!Array.isArray(invariant.requiredRuntimeSignals)) {
      throw new Error(`${source}.invariants[${index}] has invalid requiredRuntimeSignals`)
    }
    invariantEntries.push([normalizedId, {
      owner: invariant.owner,
      requiredRuntimeSignals: [...invariant.requiredRuntimeSignals].sort(),
    }])
  }
  return Object.fromEntries(invariantEntries.sort(([left], [right]) => left.localeCompare(right)))
}

function allowedCloudOwnerOverride(id, ossOwner, cloudOwner) {
  return (CLOUD_CONTEXT_OWNER_OVERRIDES[id] ?? []).includes(ossOwner)
    && (CLOUD_CONTEXT_OWNER_OVERRIDES[id] ?? []).includes(cloudOwner)
}
