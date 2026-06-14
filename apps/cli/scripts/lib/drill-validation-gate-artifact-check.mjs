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
  requiredArtifactGeneratedEvidenceKinds = [],
  requiredArtifactEvidenceRepos = [],
  requiredArtifactRuntimeSignals = [],
  requiredArtifactRuntimeSignalOwners = [],
  requiredArtifactOwners = [],
  requiredArtifactClassifications = [],
}) {
  if (artifactRoots.length === 0 && artifactIndexes.length === 0) {
    if (requiredArtifactSchemas.length > 0
      || requiredArtifactCoverageAreas.length > 0
      || requiredArtifactKinds.length > 0
      || requiredArtifactGeneratedEvidenceKinds.length > 0
      || requiredArtifactEvidenceRepos.length > 0
      || requiredArtifactRuntimeSignals.length > 0
      || requiredArtifactRuntimeSignalOwners.length > 0
      || requiredArtifactOwners.length > 0
      || requiredArtifactClassifications.length > 0) {
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
        requiredArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
        missingArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
        requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        missingArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        requiredArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
        missingArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
        requiredArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
        missingArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
        requiredArtifactOwners: [...requiredArtifactOwners],
        missingArtifactOwners: [...requiredArtifactOwners],
        requiredArtifactClassifications: [...requiredArtifactClassifications],
        missingArtifactClassifications: [...requiredArtifactClassifications],
        error: artifactRequirementError({
          missingArtifactCoverageAreas: requiredArtifactCoverageAreas,
          missingArtifactSchemas: requiredArtifactSchemas,
          missingArtifactKinds: requiredArtifactKinds,
          missingArtifactGeneratedEvidenceKinds: requiredArtifactGeneratedEvidenceKinds,
          missingArtifactEvidenceRepos: requiredArtifactEvidenceRepos,
          missingArtifactRuntimeSignals: requiredArtifactRuntimeSignals,
          missingArtifactRuntimeSignalOwners: requiredArtifactRuntimeSignalOwners,
          missingArtifactOwners: requiredArtifactOwners,
          missingArtifactClassifications: requiredArtifactClassifications,
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
      requiredArtifactGeneratedEvidenceKinds: [],
      missingArtifactGeneratedEvidenceKinds: [],
      requiredArtifactEvidenceRepos: [],
      missingArtifactEvidenceRepos: [],
      requiredArtifactRuntimeSignals: [],
      missingArtifactRuntimeSignals: [],
      requiredArtifactRuntimeSignalOwners: [],
      missingArtifactRuntimeSignalOwners: [],
      requiredArtifactOwners: [],
      missingArtifactOwners: [],
      requiredArtifactClassifications: [],
      missingArtifactClassifications: [],
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
        requiredArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
        missingArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
        requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        missingArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        requiredArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
        missingArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
        requiredArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
        missingArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
        requiredArtifactOwners: [...requiredArtifactOwners],
        missingArtifactOwners: [...requiredArtifactOwners],
        requiredArtifactClassifications: [...requiredArtifactClassifications],
        missingArtifactClassifications: [...requiredArtifactClassifications],
        error: "no artifact indexes found",
      }
    }
    const indexes = await Promise.all(indexPaths.map((indexPath) => verifyDrillArtifactIndex(indexPath)))
    const aggregate = summarizeDrillArtifactIndexes(indexes, { sources: indexPaths })
    const missingArtifactCoverageAreas = requiredArtifactCoverageAreas.filter((area) => !Object.prototype.hasOwnProperty.call(aggregate.coverageAreas ?? {}, area))
    const missingArtifactSchemas = requiredArtifactSchemas.filter((schema) => !Object.prototype.hasOwnProperty.call(aggregate.schemas, schema))
    const missingArtifactKinds = requiredArtifactKinds.filter((kind) => !Object.prototype.hasOwnProperty.call(aggregate.artifactKinds ?? {}, kind))
    const missingArtifactGeneratedEvidenceKinds = requiredArtifactGeneratedEvidenceKinds.filter((kind) => !Object.prototype.hasOwnProperty.call(aggregate.generatedEvidenceKinds ?? {}, kind))
    const missingArtifactEvidenceRepos = requiredArtifactEvidenceRepos.filter((repo) => !Object.prototype.hasOwnProperty.call(aggregate.evidenceRepos ?? {}, repo))
    const missingArtifactRuntimeSignals = requiredArtifactRuntimeSignals.filter((signal) => !Object.prototype.hasOwnProperty.call(aggregate.runtimeSignals ?? {}, signal))
    const missingArtifactRuntimeSignalOwners = requiredArtifactRuntimeSignalOwners.filter((owner) => !Object.prototype.hasOwnProperty.call(aggregate.runtimeSignalOwners ?? {}, owner))
    const missingArtifactOwners = requiredArtifactOwners.filter((owner) => !Object.prototype.hasOwnProperty.call(aggregate.owners ?? {}, owner))
    const missingArtifactClassifications = requiredArtifactClassifications.filter((classification) => !Object.prototype.hasOwnProperty.call(aggregate.classifications ?? {}, classification))
    const missingRequirements = missingArtifactCoverageAreas.length
      + missingArtifactSchemas.length
      + missingArtifactKinds.length
      + missingArtifactGeneratedEvidenceKinds.length
      + missingArtifactEvidenceRepos.length
      + missingArtifactRuntimeSignals.length
      + missingArtifactRuntimeSignalOwners.length
      + missingArtifactOwners.length
      + missingArtifactClassifications.length
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
      requiredArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
      missingArtifactGeneratedEvidenceKinds,
      requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
      missingArtifactEvidenceRepos,
      requiredArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
      missingArtifactRuntimeSignals,
      requiredArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
      missingArtifactRuntimeSignalOwners,
      requiredArtifactOwners: [...requiredArtifactOwners],
      missingArtifactOwners,
      requiredArtifactClassifications: [...requiredArtifactClassifications],
      missingArtifactClassifications,
      aggregate,
      ...(missingRequirements > 0
        ? {
          error: artifactRequirementError({
            missingArtifactCoverageAreas,
            missingArtifactSchemas,
            missingArtifactKinds,
            missingArtifactGeneratedEvidenceKinds,
            missingArtifactEvidenceRepos,
            missingArtifactRuntimeSignals,
            missingArtifactRuntimeSignalOwners,
            missingArtifactOwners,
            missingArtifactClassifications,
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
      requiredArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
      missingArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
      requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
      missingArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
      requiredArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
      missingArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
      requiredArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
      missingArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
      requiredArtifactOwners: [...requiredArtifactOwners],
      missingArtifactOwners: [...requiredArtifactOwners],
      requiredArtifactClassifications: [...requiredArtifactClassifications],
      missingArtifactClassifications: [...requiredArtifactClassifications],
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

function artifactRequirementError({
  missingArtifactCoverageAreas,
  missingArtifactSchemas,
  missingArtifactKinds,
  missingArtifactGeneratedEvidenceKinds,
  missingArtifactEvidenceRepos,
  missingArtifactRuntimeSignals,
  missingArtifactRuntimeSignalOwners,
  missingArtifactOwners,
  missingArtifactClassifications,
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
  if (missingArtifactGeneratedEvidenceKinds.length > 0) {
    messages.push(`missing required artifact generated evidence kinds: ${missingArtifactGeneratedEvidenceKinds.join(", ")}`)
  }
  if (missingArtifactEvidenceRepos.length > 0) {
    messages.push(`missing required artifact evidence repos: ${missingArtifactEvidenceRepos.join(", ")}`)
  }
  if (missingArtifactRuntimeSignals.length > 0) {
    messages.push(`missing required artifact runtime signals: ${missingArtifactRuntimeSignals.join(", ")}`)
  }
  if (missingArtifactRuntimeSignalOwners.length > 0) {
    messages.push(`missing required artifact runtime signal owners: ${missingArtifactRuntimeSignalOwners.join(", ")}`)
  }
  if (missingArtifactOwners.length > 0) {
    messages.push(`missing required artifact owners: ${missingArtifactOwners.join(", ")}`)
  }
  if (missingArtifactClassifications.length > 0) {
    messages.push(`missing required artifact classifications: ${missingArtifactClassifications.join(", ")}`)
  }
  return messages.join("; ")
}
