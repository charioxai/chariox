import {
  findDrillArtifactIndexPaths,
  summarizeDrillArtifactIndexes,
  verifyDrillArtifactIndex,
} from "./drill-artifacts.mjs"

export async function artifactValidationGateCheck({ artifactIndexes, artifactRoots }, { maxDepth }) {
  if (artifactRoots.length === 0 && artifactIndexes.length === 0) {
    return {
      status: "skipped",
      roots: [],
      inputs: [],
      indexPaths: [],
    }
  }
  try {
    const discovered = artifactRoots.length > 0
      ? await findDrillArtifactIndexPaths(artifactRoots, { maxDepth })
      : []
    const indexPaths = [...new Set([...artifactIndexes, ...discovered])].sort()
    if (indexPaths.length === 0) {
      return {
        status: "failed",
        roots: [...artifactRoots],
        inputs: [...artifactIndexes],
        indexPaths,
        error: "no artifact indexes found",
      }
    }
    const indexes = await Promise.all(indexPaths.map((indexPath) => verifyDrillArtifactIndex(indexPath)))
    const aggregate = summarizeDrillArtifactIndexes(indexes, { sources: indexPaths })
    return {
      status: "passed",
      roots: [...artifactRoots],
      inputs: [...artifactIndexes],
      indexPaths,
      aggregate,
    }
  } catch (error) {
    return {
      status: "failed",
      roots: [...artifactRoots],
      inputs: [...artifactIndexes],
      indexPaths: [],
      error: error instanceof Error ? error.message : String(error),
    }
  }
}
