import {
  findDrillFailureManifestPaths,
  readDrillFailureManifest,
  resolveFailureManifestPath,
  summarizeDrillFailureManifests,
} from "./drill-failure-manifest.mjs"

export async function failureValidationGateCheck({ failureInputs, failureRoots }, { maxDepth }) {
  if (failureRoots.length === 0 && failureInputs.length === 0) {
    return {
      status: "skipped",
      roots: [],
      inputs: [],
      manifestPaths: [],
    }
  }
  try {
    const discovered = failureRoots.length > 0
      ? await findDrillFailureManifestPaths(failureRoots, { maxDepth })
      : []
    const inputManifestPaths = await Promise.all(failureInputs.map((input) => resolveFailureManifestPath(input)))
    const manifestPaths = [...new Set([...inputManifestPaths, ...discovered])].sort()
    const manifests = await Promise.all(manifestPaths.map((manifestPath) => readDrillFailureManifest(manifestPath)))
    const aggregate = summarizeDrillFailureManifests(manifests, { sources: manifestPaths })
    return {
      status: aggregate.total === 0 ? "passed" : "failed",
      roots: [...failureRoots],
      inputs: [...failureInputs],
      manifestPaths,
      aggregate,
    }
  } catch (error) {
    return {
      status: "failed",
      roots: [...failureRoots],
      inputs: [...failureInputs],
      manifestPaths: [],
      error: error instanceof Error ? error.message : String(error),
    }
  }
}
