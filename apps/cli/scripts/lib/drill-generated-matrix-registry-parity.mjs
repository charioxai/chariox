import path from "node:path"
import { pathToFileURL } from "node:url"

import {
  drillGeneratedMatrixNamesManifest,
} from "./drill-generated-matrix-names.mjs"

export async function verifyDrillGeneratedMatrixRegistryParity({ cloudRoot }) {
  const cloudRegistryPath = path.join(cloudRoot, "scripts", "lib", "cloud-drill-generated-matrix-names.mjs")
  let cloudModule
  try {
    cloudModule = await import(pathToFileURL(cloudRegistryPath).href)
  } catch (error) {
    throw new Error(`generated matrix registry parity requires Cloud registry at ${cloudRegistryPath}: ${error.message}`)
  }
  if (typeof cloudModule.cloudDrillGeneratedMatrixNamesManifest !== "function") {
    throw new Error(`generated matrix registry parity requires cloudDrillGeneratedMatrixNamesManifest in ${cloudRegistryPath}`)
  }
  const ossRegistry = generatedMatrixRepoMap(drillGeneratedMatrixNamesManifest(), "OSS generated matrix registry")
  const cloudRegistry = generatedMatrixRepoMap(
    cloudModule.cloudDrillGeneratedMatrixNamesManifest(),
    "Cloud generated matrix registry",
  )
  if (JSON.stringify(ossRegistry) !== JSON.stringify(cloudRegistry)) {
    throw new Error(
      "generated matrix registry parity failed: "
        + `OSS=${JSON.stringify(ossRegistry)} Cloud=${JSON.stringify(cloudRegistry)}`,
    )
  }
}

function generatedMatrixRepoMap(manifest, source) {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`${source} is not an object`)
  }
  if (!Array.isArray(manifest.matrices)) {
    throw new Error(`${source} has invalid matrices`)
  }
  const matrixEntries = []
  const seenNames = new Set()
  for (const [index, matrix] of manifest.matrices.entries()) {
    if (!matrix || typeof matrix !== "object" || Array.isArray(matrix)) {
      throw new Error(`${source}.matrices[${index}] is not an object`)
    }
    if (typeof matrix.name !== "string" || matrix.name.length === 0) {
      throw new Error(`${source}.matrices[${index}] has invalid name`)
    }
    if (seenNames.has(matrix.name)) {
      throw new Error(`${source}.matrices[${index}] duplicates matrix ${JSON.stringify(matrix.name)}`)
    }
    seenNames.add(matrix.name)
    if (typeof matrix.repo !== "string" || matrix.repo.length === 0) {
      throw new Error(`${source}.matrices[${index}] has invalid repo`)
    }
    matrixEntries.push([matrix.name, matrix.repo])
  }
  return Object.fromEntries(matrixEntries.sort(([left], [right]) => left.localeCompare(right)))
}
