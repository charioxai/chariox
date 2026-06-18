import path from "node:path"
import { pathToFileURL } from "node:url"

import { drillRuntimeSignalsManifest } from "./drill-runtime-signals.mjs"

export async function verifyDrillRuntimeSignalRegistryParity({ cloudRoot }) {
  const cloudRegistryPath = path.join(cloudRoot, "scripts", "lib", "cloud-runtime-signals.mjs")
  let cloudModule
  try {
    cloudModule = await import(pathToFileURL(cloudRegistryPath).href)
  } catch (error) {
    throw new Error(`runtime signal registry parity requires Cloud registry at ${cloudRegistryPath}: ${error.message}`)
  }
  if (typeof cloudModule.cloudRuntimeSignalsManifest !== "function") {
    throw new Error(`runtime signal registry parity requires cloudRuntimeSignalsManifest in ${cloudRegistryPath}`)
  }
  const ossRegistry = runtimeSignalMap(drillRuntimeSignalsManifest(), "OSS runtime signal registry")
  const cloudRegistry = runtimeSignalMap(
    cloudModule.cloudRuntimeSignalsManifest(),
    "Cloud runtime signal registry",
  )
  if (JSON.stringify(ossRegistry) !== JSON.stringify(cloudRegistry)) {
    throw new Error(
      "runtime signal registry parity failed: "
        + `OSS=${JSON.stringify(ossRegistry)} Cloud=${JSON.stringify(cloudRegistry)}`,
    )
  }
}

function runtimeSignalMap(manifest, source) {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`${source} is not an object`)
  }
  if (!Array.isArray(manifest.signals)) {
    throw new Error(`${source} has invalid signals`)
  }
  const signalEntries = []
  const seenIds = new Set()
  for (const [index, signal] of manifest.signals.entries()) {
    if (!signal || typeof signal !== "object" || Array.isArray(signal)) {
      throw new Error(`${source}.signals[${index}] is not an object`)
    }
    if (typeof signal.id !== "string" || signal.id.length === 0) {
      throw new Error(`${source}.signals[${index}] has invalid id`)
    }
    if (seenIds.has(signal.id)) {
      throw new Error(`${source}.signals[${index}] duplicates signal ${JSON.stringify(signal.id)}`)
    }
    seenIds.add(signal.id)
    if (typeof signal.owner !== "string" || signal.owner.length === 0) {
      throw new Error(`${source}.signals[${index}] has invalid owner`)
    }
    if (typeof signal.description !== "string" || signal.description.length === 0) {
      throw new Error(`${source}.signals[${index}] has invalid description`)
    }
    signalEntries.push([signal.id, {
      description: signal.description,
      owner: signal.owner,
    }])
  }
  return Object.fromEntries(signalEntries.sort(([left], [right]) => left.localeCompare(right)))
}
