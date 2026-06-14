import {
  findDrillArtifactIndexPaths,
  summarizeDrillArtifactIndexes,
  verifyDrillArtifactIndex,
} from "./drill-artifacts.mjs"

export async function artifactValidationGateCheck({ artifactIndexes, artifactRoots }, {
  maxDepth,
  requiredArtifactSchemas = [],
}) {
  if (artifactRoots.length === 0 && artifactIndexes.length === 0) {
    if (requiredArtifactSchemas.length > 0) {
      return {
        status: "failed",
        roots: [],
        inputs: [],
        indexPaths: [],
        requiredArtifactSchemas: [...requiredArtifactSchemas],
        missingArtifactSchemas: [...requiredArtifactSchemas],
        error: `missing required artifact schemas: ${requiredArtifactSchemas.join(", ")}`,
      }
    }
    return {
      status: "skipped",
      roots: [],
      inputs: [],
      indexPaths: [],
      requiredArtifactSchemas: [],
      missingArtifactSchemas: [],
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
        requiredArtifactSchemas: [...requiredArtifactSchemas],
        missingArtifactSchemas: [...requiredArtifactSchemas],
        error: "no artifact indexes found",
      }
    }
    const indexes = await Promise.all(indexPaths.map((indexPath) => verifyDrillArtifactIndex(indexPath)))
    const aggregate = summarizeDrillArtifactIndexes(indexes, { sources: indexPaths })
    const missingArtifactSchemas = requiredArtifactSchemas.filter((schema) => !Object.prototype.hasOwnProperty.call(aggregate.schemas, schema))
    return {
      status: missingArtifactSchemas.length > 0 ? "failed" : "passed",
      roots: [...artifactRoots],
      inputs: [...artifactIndexes],
      indexPaths,
      requiredArtifactSchemas: [...requiredArtifactSchemas],
      missingArtifactSchemas,
      aggregate,
      ...(missingArtifactSchemas.length > 0
        ? { error: `missing required artifact schemas: ${missingArtifactSchemas.join(", ")}` }
        : {}),
    }
  } catch (error) {
    return {
      status: "failed",
      roots: [...artifactRoots],
      inputs: [...artifactIndexes],
      indexPaths: [],
      requiredArtifactSchemas: [...requiredArtifactSchemas],
      missingArtifactSchemas: [...requiredArtifactSchemas],
      error: error instanceof Error ? error.message : String(error),
    }
  }
}
