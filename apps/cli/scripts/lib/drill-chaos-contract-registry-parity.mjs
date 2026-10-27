import path from "node:path"
import { pathToFileURL } from "node:url"

import {
  DRILL_CHAOS_CONTRACT_SCHEMA,
  drillChaosContractManifest,
} from "./drill-chaos-contract.mjs"

export async function verifyDrillChaosContractRegistryParity({ cloudRoot }) {
  const cloudRegistryPath = path.join(cloudRoot, "scripts", "lib", "cloud-chaos-contract.mjs")
  let cloudModule
  try {
    cloudModule = await import(pathToFileURL(cloudRegistryPath).href)
  } catch (error) {
    throw new Error(`chaos contract registry parity requires Cloud registry at ${cloudRegistryPath}: ${error.message}`)
  }
  if (typeof cloudModule.cloudDrillChaosContractManifest !== "function") {
    throw new Error(`chaos contract registry parity requires cloudDrillChaosContractManifest in ${cloudRegistryPath}`)
  }
  const ossContract = normalizeChaosContractManifest(
    drillChaosContractManifest(),
    "OSS chaos contract registry",
  )
  const cloudContract = normalizeChaosContractManifest(
    cloudModule.cloudDrillChaosContractManifest(),
    "Cloud chaos contract registry",
  )
  if (JSON.stringify(ossContract) !== JSON.stringify(cloudContract)) {
    throw new Error(
      "chaos contract registry parity failed: "
        + `OSS=${JSON.stringify(ossContract)} Cloud=${JSON.stringify(cloudContract)}`,
    )
  }
}

function normalizeChaosContractManifest(manifest, source) {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`${source} is not an object`)
  }
  if (manifest.schema !== DRILL_CHAOS_CONTRACT_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  return {
    schema: manifest.schema,
    replaySchema: requireText(manifest.replaySchema, `${source}.replaySchema`),
    invariantsSchema: requireText(manifest.invariantsSchema, `${source}.invariantsSchema`),
    faultKinds: normalizeUniqueTextList(manifest.faultKinds, `${source}.faultKinds`),
    invariantIds: normalizeUniqueTextList(manifest.invariantIds, `${source}.invariantIds`),
  }
}

function normalizeUniqueTextList(value, source) {
  if (!Array.isArray(value)) throw new Error(`${source} is not an array`)
  const normalized = value.map((item, index) => requireText(item, `${source}[${index}]`))
  const unique = new Set(normalized)
  if (unique.size !== normalized.length) throw new Error(`${source} contains duplicate values`)
  return [...unique].sort()
}

function requireText(value, source) {
  if (typeof value !== "string" || value.length === 0) throw new Error(`${source} is invalid`)
  return value
}
