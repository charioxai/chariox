import {
  findDrillArtifactIndexPaths,
  summarizeDrillArtifactIndexes,
  verifyDrillArtifactIndex,
} from "./drill-artifacts.mjs"

export async function artifactValidationGateCheck({ artifactIndexes, artifactRoots }, {
  maxDepth,
  requiredArtifactCoverageAreas = [],
  requiredArtifactSchemas = [],
  requiredArtifactKinds = [],
  requiredArtifactEvidenceRepos = [],
}) {
  if (artifactRoots.length === 0 && artifactIndexes.length === 0) {
    if (requiredArtifactSchemas.length > 0
      || requiredArtifactCoverageAreas.length > 0
      || requiredArtifactKinds.length > 0
      || requiredArtifactEvidenceRepos.length > 0) {
      return {
        status: "failed",
        roots: [],
        inputs: [],
        indexPaths: [],
        requiredArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
        missingArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
        requiredArtifactSchemas: [...requiredArtifactSchemas],
        missingArtifactSchemas: [...requiredArtifactSchemas],
        requiredArtifactKinds: [...requiredArtifactKinds],
        missingArtifactKinds: [...requiredArtifactKinds],
        requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        missingArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        error: artifactRequirementError({
          missingArtifactCoverageAreas: requiredArtifactCoverageAreas,
          missingArtifactSchemas: requiredArtifactSchemas,
          missingArtifactKinds: requiredArtifactKinds,
          missingArtifactEvidenceRepos: requiredArtifactEvidenceRepos,
        }),
      }
    }
    return {
      status: "skipped",
      roots: [],
      inputs: [],
      indexPaths: [],
      requiredArtifactCoverageAreas: [],
      missingArtifactCoverageAreas: [],
      requiredArtifactSchemas: [],
      missingArtifactSchemas: [],
      requiredArtifactKinds: [],
      missingArtifactKinds: [],
      requiredArtifactEvidenceRepos: [],
      missingArtifactEvidenceRepos: [],
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
        requiredArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
        missingArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
        requiredArtifactSchemas: [...requiredArtifactSchemas],
        missingArtifactSchemas: [...requiredArtifactSchemas],
        requiredArtifactKinds: [...requiredArtifactKinds],
        missingArtifactKinds: [...requiredArtifactKinds],
        requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        missingArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        error: "no artifact indexes found",
      }
    }
    const indexes = await Promise.all(indexPaths.map((indexPath) => verifyDrillArtifactIndex(indexPath)))
    const aggregate = summarizeDrillArtifactIndexes(indexes, { sources: indexPaths })
    const missingArtifactCoverageAreas = requiredArtifactCoverageAreas.filter((area) => !Object.prototype.hasOwnProperty.call(aggregate.coverageAreas ?? {}, area))
    const missingArtifactSchemas = requiredArtifactSchemas.filter((schema) => !Object.prototype.hasOwnProperty.call(aggregate.schemas, schema))
    const missingArtifactKinds = requiredArtifactKinds.filter((kind) => !Object.prototype.hasOwnProperty.call(aggregate.artifactKinds ?? {}, kind))
    const missingArtifactEvidenceRepos = requiredArtifactEvidenceRepos.filter((repo) => !Object.prototype.hasOwnProperty.call(aggregate.evidenceRepos ?? {}, repo))
    const missingRequirements = missingArtifactCoverageAreas.length
      + missingArtifactSchemas.length
      + missingArtifactKinds.length
      + missingArtifactEvidenceRepos.length
    return {
      status: missingRequirements > 0 ? "failed" : "passed",
      roots: [...artifactRoots],
      inputs: [...artifactIndexes],
      indexPaths,
      requiredArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
      missingArtifactCoverageAreas,
      requiredArtifactSchemas: [...requiredArtifactSchemas],
      missingArtifactSchemas,
      requiredArtifactKinds: [...requiredArtifactKinds],
      missingArtifactKinds,
      requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
      missingArtifactEvidenceRepos,
      aggregate,
      ...(missingRequirements > 0
        ? {
          error: artifactRequirementError({
            missingArtifactCoverageAreas,
            missingArtifactSchemas,
            missingArtifactKinds,
            missingArtifactEvidenceRepos,
          }),
        }
        : {}),
    }
  } catch (error) {
    return {
      status: "failed",
      roots: [...artifactRoots],
      inputs: [...artifactIndexes],
      indexPaths: [],
      requiredArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
      missingArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
      requiredArtifactSchemas: [...requiredArtifactSchemas],
      missingArtifactSchemas: [...requiredArtifactSchemas],
      requiredArtifactKinds: [...requiredArtifactKinds],
      missingArtifactKinds: [...requiredArtifactKinds],
      requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
      missingArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

function artifactRequirementError({
  missingArtifactCoverageAreas,
  missingArtifactSchemas,
  missingArtifactKinds,
  missingArtifactEvidenceRepos,
}) {
  const messages = []
  if (missingArtifactCoverageAreas.length > 0) {
    messages.push(`missing required artifact coverage areas: ${missingArtifactCoverageAreas.join(", ")}`)
  }
  if (missingArtifactSchemas.length > 0) {
    messages.push(`missing required artifact schemas: ${missingArtifactSchemas.join(", ")}`)
  }
  if (missingArtifactKinds.length > 0) {
    messages.push(`missing required artifact kinds: ${missingArtifactKinds.join(", ")}`)
  }
  if (missingArtifactEvidenceRepos.length > 0) {
    messages.push(`missing required artifact evidence repos: ${missingArtifactEvidenceRepos.join(", ")}`)
  }
  return messages.join("; ")
}
