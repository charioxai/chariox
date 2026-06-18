import {
  findDrillArtifactIndexPaths,
  summarizeDrillArtifactIndexes,
  verifyDrillArtifactIndex,
} from "./drill-artifacts.mjs"
import { parseDrillIsoTimestamp } from "./drill-time.mjs"

export async function artifactValidationGateCheck({ artifactIndexes, artifactRoots }, {
  maxDepth,
  requiredArtifactCoverageAreas = [],
  requiredArtifactSchemas = [],
  requiredArtifactKinds = [],
  requiredArtifactGeneratedEvidenceKinds = [],
  requiredArtifactGeneratedEvidenceRepos = [],
  requiredArtifactGeneratedMatrixArtifactIndexes = [],
  requiredArtifactGeneratedMatrixLimitations = [],
  requiredArtifactGeneratedMatrixNames = [],
  requiredArtifactGeneratedMatrixRepos = [],
  requiredArtifactGeneratedValidationSuiteArtifactIndexes = [],
  requiredArtifactGeneratedValidationSuiteFailureRoots = [],
  requiredArtifactEvidenceRepos = [],
  requiredArtifactProviderAccountAliases = [],
  requiredArtifactValidationPresets = [],
  requiredArtifactRuntimeAuthorityInvariants = [],
  requiredArtifactRuntimeSignals = [],
  requiredArtifactRuntimeSignalOwners = [],
  requiredArtifactOwners = [],
  requiredArtifactClassifications = [],
  requiredArtifactFailureClassifications = [],
  requiredArtifactPlannedOwners = [],
  requiredArtifactPlannedClassifications = [],
  requiredArtifactExitCriterionStatuses = [],
  requiredArtifactIncompleteExitCriterionStatuses = [],
  requiredArtifactMaxAgeMs = null,
  nowMs = Date.now(),
}) {
  if (artifactRoots.length === 0 && artifactIndexes.length === 0) {
    if (requiredArtifactSchemas.length > 0
      || requiredArtifactCoverageAreas.length > 0
      || requiredArtifactKinds.length > 0
      || requiredArtifactGeneratedEvidenceKinds.length > 0
      || requiredArtifactGeneratedEvidenceRepos.length > 0
      || requiredArtifactGeneratedMatrixArtifactIndexes.length > 0
      || requiredArtifactGeneratedMatrixLimitations.length > 0
      || requiredArtifactGeneratedMatrixNames.length > 0
      || requiredArtifactGeneratedMatrixRepos.length > 0
      || requiredArtifactGeneratedValidationSuiteArtifactIndexes.length > 0
      || requiredArtifactGeneratedValidationSuiteFailureRoots.length > 0
      || requiredArtifactEvidenceRepos.length > 0
      || requiredArtifactProviderAccountAliases.length > 0
      || requiredArtifactValidationPresets.length > 0
      || requiredArtifactRuntimeAuthorityInvariants.length > 0
      || requiredArtifactRuntimeSignals.length > 0
      || requiredArtifactRuntimeSignalOwners.length > 0
      || requiredArtifactOwners.length > 0
      || requiredArtifactClassifications.length > 0
      || requiredArtifactFailureClassifications.length > 0
      || requiredArtifactPlannedOwners.length > 0
      || requiredArtifactPlannedClassifications.length > 0
      || requiredArtifactExitCriterionStatuses.length > 0
      || requiredArtifactIncompleteExitCriterionStatuses.length > 0
      || requiredArtifactMaxAgeMs !== null) {
      return {
        status: "failed",
        roots: [],
        inputs: [],
        indexPaths: [],
        requiredArtifactMaxAgeMs,
        staleArtifactIndexes: [],
        requiredArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
        missingArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
        requiredArtifactSchemas: [...requiredArtifactSchemas],
        missingArtifactSchemas: [...requiredArtifactSchemas],
        requiredArtifactKinds: [...requiredArtifactKinds],
        missingArtifactKinds: [...requiredArtifactKinds],
        requiredArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
        missingArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
        requiredArtifactGeneratedEvidenceRepos: [...requiredArtifactGeneratedEvidenceRepos],
        missingArtifactGeneratedEvidenceRepos: [...requiredArtifactGeneratedEvidenceRepos],
        requiredArtifactGeneratedMatrixArtifactIndexes: [...requiredArtifactGeneratedMatrixArtifactIndexes],
        missingArtifactGeneratedMatrixArtifactIndexes: [...requiredArtifactGeneratedMatrixArtifactIndexes],
        requiredArtifactGeneratedMatrixLimitations: [...requiredArtifactGeneratedMatrixLimitations],
        missingArtifactGeneratedMatrixLimitations: [...requiredArtifactGeneratedMatrixLimitations],
        requiredArtifactGeneratedMatrixNames: [...requiredArtifactGeneratedMatrixNames],
        missingArtifactGeneratedMatrixNames: [...requiredArtifactGeneratedMatrixNames],
        requiredArtifactGeneratedMatrixRepos: [...requiredArtifactGeneratedMatrixRepos],
        missingArtifactGeneratedMatrixRepos: [...requiredArtifactGeneratedMatrixRepos],
        requiredArtifactGeneratedValidationSuiteArtifactIndexes: [...requiredArtifactGeneratedValidationSuiteArtifactIndexes],
        missingArtifactGeneratedValidationSuiteArtifactIndexes: [...requiredArtifactGeneratedValidationSuiteArtifactIndexes],
        requiredArtifactGeneratedValidationSuiteFailureRoots: [...requiredArtifactGeneratedValidationSuiteFailureRoots],
        missingArtifactGeneratedValidationSuiteFailureRoots: [...requiredArtifactGeneratedValidationSuiteFailureRoots],
        requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        missingArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        requiredArtifactProviderAccountAliases: [...requiredArtifactProviderAccountAliases],
        missingArtifactProviderAccountAliases: [...requiredArtifactProviderAccountAliases],
        requiredArtifactValidationPresets: [...requiredArtifactValidationPresets],
        missingArtifactValidationPresets: [...requiredArtifactValidationPresets],
        requiredArtifactRuntimeAuthorityInvariants: [...requiredArtifactRuntimeAuthorityInvariants],
        missingArtifactRuntimeAuthorityInvariants: [...requiredArtifactRuntimeAuthorityInvariants],
        requiredArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
        missingArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
        requiredArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
        missingArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
        requiredArtifactOwners: [...requiredArtifactOwners],
        missingArtifactOwners: [...requiredArtifactOwners],
        requiredArtifactClassifications: [...requiredArtifactClassifications],
        missingArtifactClassifications: [...requiredArtifactClassifications],
        requiredArtifactFailureClassifications: [...requiredArtifactFailureClassifications],
        missingArtifactFailureClassifications: [...requiredArtifactFailureClassifications],
        requiredArtifactPlannedOwners: [...requiredArtifactPlannedOwners],
        missingArtifactPlannedOwners: [...requiredArtifactPlannedOwners],
        requiredArtifactPlannedClassifications: [...requiredArtifactPlannedClassifications],
        missingArtifactPlannedClassifications: [...requiredArtifactPlannedClassifications],
        requiredArtifactExitCriterionStatuses: [...requiredArtifactExitCriterionStatuses],
        missingArtifactExitCriterionStatuses: [...requiredArtifactExitCriterionStatuses],
        requiredArtifactIncompleteExitCriterionStatuses: [...requiredArtifactIncompleteExitCriterionStatuses],
        missingArtifactIncompleteExitCriterionStatuses: [...requiredArtifactIncompleteExitCriterionStatuses],
        error: artifactRequirementError({
          missingArtifactCoverageAreas: requiredArtifactCoverageAreas,
          missingArtifactSchemas: requiredArtifactSchemas,
          missingArtifactKinds: requiredArtifactKinds,
          missingArtifactGeneratedEvidenceKinds: requiredArtifactGeneratedEvidenceKinds,
          missingArtifactGeneratedEvidenceRepos: requiredArtifactGeneratedEvidenceRepos,
          missingArtifactGeneratedMatrixArtifactIndexes: requiredArtifactGeneratedMatrixArtifactIndexes,
          missingArtifactGeneratedMatrixLimitations: requiredArtifactGeneratedMatrixLimitations,
          missingArtifactGeneratedMatrixNames: requiredArtifactGeneratedMatrixNames,
          missingArtifactGeneratedMatrixRepos: requiredArtifactGeneratedMatrixRepos,
          missingArtifactGeneratedValidationSuiteArtifactIndexes: requiredArtifactGeneratedValidationSuiteArtifactIndexes,
          missingArtifactGeneratedValidationSuiteFailureRoots: requiredArtifactGeneratedValidationSuiteFailureRoots,
          missingArtifactEvidenceRepos: requiredArtifactEvidenceRepos,
          missingArtifactProviderAccountAliases: requiredArtifactProviderAccountAliases,
          missingArtifactValidationPresets: requiredArtifactValidationPresets,
          missingArtifactRuntimeAuthorityInvariants: requiredArtifactRuntimeAuthorityInvariants,
          missingArtifactRuntimeSignals: requiredArtifactRuntimeSignals,
          missingArtifactRuntimeSignalOwners: requiredArtifactRuntimeSignalOwners,
          missingArtifactOwners: requiredArtifactOwners,
          missingArtifactClassifications: requiredArtifactClassifications,
          missingArtifactFailureClassifications: requiredArtifactFailureClassifications,
          missingArtifactPlannedOwners: requiredArtifactPlannedOwners,
          missingArtifactPlannedClassifications: requiredArtifactPlannedClassifications,
          missingArtifactExitCriterionStatuses: requiredArtifactExitCriterionStatuses,
          missingArtifactIncompleteExitCriterionStatuses: requiredArtifactIncompleteExitCriterionStatuses,
          staleArtifactIndexes: requiredArtifactMaxAgeMs !== null ? [] : undefined,
        }) || "no artifact indexes found",
      }
    }
    return {
      status: "skipped",
      roots: [],
      inputs: [],
      indexPaths: [],
      requiredArtifactMaxAgeMs,
      staleArtifactIndexes: [],
      requiredArtifactCoverageAreas: [],
      missingArtifactCoverageAreas: [],
      requiredArtifactSchemas: [],
      missingArtifactSchemas: [],
      requiredArtifactKinds: [],
      missingArtifactKinds: [],
      requiredArtifactGeneratedEvidenceKinds: [],
      missingArtifactGeneratedEvidenceKinds: [],
      requiredArtifactGeneratedEvidenceRepos: [],
      missingArtifactGeneratedEvidenceRepos: [],
      requiredArtifactGeneratedMatrixArtifactIndexes: [],
      missingArtifactGeneratedMatrixArtifactIndexes: [],
      requiredArtifactGeneratedMatrixLimitations: [],
      missingArtifactGeneratedMatrixLimitations: [],
      requiredArtifactGeneratedMatrixNames: [],
      missingArtifactGeneratedMatrixNames: [],
      requiredArtifactGeneratedMatrixRepos: [],
      missingArtifactGeneratedMatrixRepos: [],
      requiredArtifactGeneratedValidationSuiteArtifactIndexes: [],
      missingArtifactGeneratedValidationSuiteArtifactIndexes: [],
      requiredArtifactGeneratedValidationSuiteFailureRoots: [],
      missingArtifactGeneratedValidationSuiteFailureRoots: [],
      requiredArtifactEvidenceRepos: [],
      missingArtifactEvidenceRepos: [],
      requiredArtifactProviderAccountAliases: [],
      missingArtifactProviderAccountAliases: [],
      requiredArtifactValidationPresets: [],
      missingArtifactValidationPresets: [],
      requiredArtifactRuntimeAuthorityInvariants: [],
      missingArtifactRuntimeAuthorityInvariants: [],
      requiredArtifactRuntimeSignals: [],
      missingArtifactRuntimeSignals: [],
      requiredArtifactRuntimeSignalOwners: [],
      missingArtifactRuntimeSignalOwners: [],
      requiredArtifactOwners: [],
      missingArtifactOwners: [],
      requiredArtifactClassifications: [],
      missingArtifactClassifications: [],
      requiredArtifactFailureClassifications: [],
      missingArtifactFailureClassifications: [],
      requiredArtifactPlannedOwners: [],
      missingArtifactPlannedOwners: [],
      requiredArtifactPlannedClassifications: [],
      missingArtifactPlannedClassifications: [],
      requiredArtifactExitCriterionStatuses: [],
      missingArtifactExitCriterionStatuses: [],
      requiredArtifactIncompleteExitCriterionStatuses: [],
      missingArtifactIncompleteExitCriterionStatuses: [],
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
        requiredArtifactMaxAgeMs,
        staleArtifactIndexes: [],
        requiredArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
        missingArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
        requiredArtifactSchemas: [...requiredArtifactSchemas],
        missingArtifactSchemas: [...requiredArtifactSchemas],
        requiredArtifactKinds: [...requiredArtifactKinds],
        missingArtifactKinds: [...requiredArtifactKinds],
        requiredArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
        missingArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
        requiredArtifactGeneratedEvidenceRepos: [...requiredArtifactGeneratedEvidenceRepos],
        missingArtifactGeneratedEvidenceRepos: [...requiredArtifactGeneratedEvidenceRepos],
        requiredArtifactGeneratedMatrixArtifactIndexes: [...requiredArtifactGeneratedMatrixArtifactIndexes],
        missingArtifactGeneratedMatrixArtifactIndexes: [...requiredArtifactGeneratedMatrixArtifactIndexes],
        requiredArtifactGeneratedMatrixLimitations: [...requiredArtifactGeneratedMatrixLimitations],
        missingArtifactGeneratedMatrixLimitations: [...requiredArtifactGeneratedMatrixLimitations],
        requiredArtifactGeneratedMatrixNames: [...requiredArtifactGeneratedMatrixNames],
        missingArtifactGeneratedMatrixNames: [...requiredArtifactGeneratedMatrixNames],
        requiredArtifactGeneratedMatrixRepos: [...requiredArtifactGeneratedMatrixRepos],
        missingArtifactGeneratedMatrixRepos: [...requiredArtifactGeneratedMatrixRepos],
        requiredArtifactGeneratedValidationSuiteArtifactIndexes: [...requiredArtifactGeneratedValidationSuiteArtifactIndexes],
        missingArtifactGeneratedValidationSuiteArtifactIndexes: [...requiredArtifactGeneratedValidationSuiteArtifactIndexes],
        requiredArtifactGeneratedValidationSuiteFailureRoots: [...requiredArtifactGeneratedValidationSuiteFailureRoots],
        missingArtifactGeneratedValidationSuiteFailureRoots: [...requiredArtifactGeneratedValidationSuiteFailureRoots],
        requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        missingArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
        requiredArtifactProviderAccountAliases: [...requiredArtifactProviderAccountAliases],
        missingArtifactProviderAccountAliases: [...requiredArtifactProviderAccountAliases],
        requiredArtifactValidationPresets: [...requiredArtifactValidationPresets],
        missingArtifactValidationPresets: [...requiredArtifactValidationPresets],
        requiredArtifactRuntimeAuthorityInvariants: [...requiredArtifactRuntimeAuthorityInvariants],
        missingArtifactRuntimeAuthorityInvariants: [...requiredArtifactRuntimeAuthorityInvariants],
        requiredArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
        missingArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
        requiredArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
        missingArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
        requiredArtifactOwners: [...requiredArtifactOwners],
        missingArtifactOwners: [...requiredArtifactOwners],
        requiredArtifactClassifications: [...requiredArtifactClassifications],
        missingArtifactClassifications: [...requiredArtifactClassifications],
        requiredArtifactFailureClassifications: [...requiredArtifactFailureClassifications],
        missingArtifactFailureClassifications: [...requiredArtifactFailureClassifications],
        requiredArtifactPlannedOwners: [...requiredArtifactPlannedOwners],
        missingArtifactPlannedOwners: [...requiredArtifactPlannedOwners],
        requiredArtifactPlannedClassifications: [...requiredArtifactPlannedClassifications],
        missingArtifactPlannedClassifications: [...requiredArtifactPlannedClassifications],
        requiredArtifactExitCriterionStatuses: [...requiredArtifactExitCriterionStatuses],
        missingArtifactExitCriterionStatuses: [...requiredArtifactExitCriterionStatuses],
        requiredArtifactIncompleteExitCriterionStatuses: [...requiredArtifactIncompleteExitCriterionStatuses],
        missingArtifactIncompleteExitCriterionStatuses: [...requiredArtifactIncompleteExitCriterionStatuses],
        error: "no artifact indexes found",
      }
    }
    const indexes = await Promise.all(indexPaths.map((indexPath) => verifyDrillArtifactIndex(indexPath)))
    const aggregate = summarizeDrillArtifactIndexes(indexes, { sources: indexPaths })
    const staleArtifactIndexes = staleArtifactIndexesFor(indexes, indexPaths, {
      nowMs,
      requiredArtifactMaxAgeMs,
    })
    const missingArtifactCoverageAreas = requiredArtifactCoverageAreas.filter((area) => !Object.prototype.hasOwnProperty.call(aggregate.coverageAreas ?? {}, area))
    const missingArtifactSchemas = requiredArtifactSchemas.filter((schema) => !Object.prototype.hasOwnProperty.call(aggregate.schemas, schema))
    const missingArtifactKinds = requiredArtifactKinds.filter((kind) => !Object.prototype.hasOwnProperty.call(aggregate.artifactKinds ?? {}, kind))
    const missingArtifactGeneratedEvidenceKinds = requiredArtifactGeneratedEvidenceKinds.filter((kind) => !Object.prototype.hasOwnProperty.call(aggregate.generatedEvidenceKinds ?? {}, kind))
    const missingArtifactGeneratedEvidenceRepos = requiredArtifactGeneratedEvidenceRepos.filter((repo) => !Object.prototype.hasOwnProperty.call(aggregate.generatedEvidenceRepos ?? {}, repo))
    const missingArtifactGeneratedMatrixArtifactIndexes = requiredArtifactGeneratedMatrixArtifactIndexes.filter((indexPath) => !Object.prototype.hasOwnProperty.call(aggregate.generatedMatrixArtifactIndexes ?? {}, indexPath))
    const missingArtifactGeneratedMatrixLimitations = requiredArtifactGeneratedMatrixLimitations.filter((limitation) => !Object.prototype.hasOwnProperty.call(aggregate.generatedMatrixLimitations ?? {}, limitation))
    const missingArtifactGeneratedMatrixNames = requiredArtifactGeneratedMatrixNames.filter((matrixName) => !Object.prototype.hasOwnProperty.call(aggregate.generatedMatrixNames ?? {}, matrixName))
    const missingArtifactGeneratedMatrixRepos = requiredArtifactGeneratedMatrixRepos.filter((repo) => !Object.prototype.hasOwnProperty.call(aggregate.generatedMatrixRepos ?? {}, repo))
    const missingArtifactGeneratedValidationSuiteArtifactIndexes = requiredArtifactGeneratedValidationSuiteArtifactIndexes.filter((indexPath) => !Object.prototype.hasOwnProperty.call(aggregate.generatedValidationSuiteArtifactIndexes ?? {}, indexPath))
    const missingArtifactGeneratedValidationSuiteFailureRoots = requiredArtifactGeneratedValidationSuiteFailureRoots.filter((root) => !Object.prototype.hasOwnProperty.call(aggregate.generatedValidationSuiteFailureRoots ?? {}, root))
    const missingArtifactEvidenceRepos = requiredArtifactEvidenceRepos.filter((repo) => !Object.prototype.hasOwnProperty.call(aggregate.evidenceRepos ?? {}, repo))
    const missingArtifactProviderAccountAliases = requiredArtifactProviderAccountAliases.filter((alias) => !Object.prototype.hasOwnProperty.call(aggregate.providerAccountAliases ?? {}, alias))
    const missingArtifactValidationPresets = requiredArtifactValidationPresets.filter((preset) => !Object.prototype.hasOwnProperty.call(aggregate.validationPresets ?? {}, preset))
    const missingArtifactRuntimeAuthorityInvariants = requiredArtifactRuntimeAuthorityInvariants.filter((invariant) => !Object.prototype.hasOwnProperty.call(aggregate.runtimeAuthorityInvariants ?? {}, invariant))
    const missingArtifactRuntimeSignals = requiredArtifactRuntimeSignals.filter((signal) => !Object.prototype.hasOwnProperty.call(aggregate.runtimeSignals ?? {}, signal))
    const missingArtifactRuntimeSignalOwners = requiredArtifactRuntimeSignalOwners.filter((owner) => !Object.prototype.hasOwnProperty.call(aggregate.runtimeSignalOwners ?? {}, owner))
    const missingArtifactOwners = requiredArtifactOwners.filter((owner) => !Object.prototype.hasOwnProperty.call(aggregate.owners ?? {}, owner))
    const missingArtifactClassifications = requiredArtifactClassifications.filter((classification) => !Object.prototype.hasOwnProperty.call(aggregate.classifications ?? {}, classification))
    const missingArtifactFailureClassifications = requiredArtifactFailureClassifications.filter((classification) => !Object.prototype.hasOwnProperty.call(aggregate.requiredFailureClassifications ?? {}, classification))
    const missingArtifactPlannedOwners = requiredArtifactPlannedOwners.filter((owner) => !Object.prototype.hasOwnProperty.call(aggregate.plannedOwners ?? {}, owner))
    const missingArtifactPlannedClassifications = requiredArtifactPlannedClassifications.filter((classification) => !Object.prototype.hasOwnProperty.call(aggregate.plannedClassifications ?? {}, classification))
    const missingArtifactExitCriterionStatuses = requiredArtifactExitCriterionStatuses.filter((status) => !Object.prototype.hasOwnProperty.call(aggregate.exitCriterionStatuses ?? {}, status))
    const missingArtifactIncompleteExitCriterionStatuses = requiredArtifactIncompleteExitCriterionStatuses.filter((status) => !Object.prototype.hasOwnProperty.call(aggregate.incompleteExitCriterionStatuses ?? {}, status))
    const missingRequirements = missingArtifactCoverageAreas.length
      + missingArtifactSchemas.length
      + missingArtifactKinds.length
      + missingArtifactGeneratedEvidenceKinds.length
      + missingArtifactGeneratedEvidenceRepos.length
      + missingArtifactGeneratedMatrixArtifactIndexes.length
      + missingArtifactGeneratedMatrixLimitations.length
      + missingArtifactGeneratedMatrixNames.length
      + missingArtifactGeneratedMatrixRepos.length
      + missingArtifactGeneratedValidationSuiteArtifactIndexes.length
      + missingArtifactGeneratedValidationSuiteFailureRoots.length
      + missingArtifactEvidenceRepos.length
      + missingArtifactProviderAccountAliases.length
      + missingArtifactValidationPresets.length
      + missingArtifactRuntimeAuthorityInvariants.length
      + missingArtifactRuntimeSignals.length
      + missingArtifactRuntimeSignalOwners.length
      + missingArtifactOwners.length
      + missingArtifactClassifications.length
      + missingArtifactFailureClassifications.length
      + missingArtifactPlannedOwners.length
      + missingArtifactPlannedClassifications.length
      + missingArtifactExitCriterionStatuses.length
      + missingArtifactIncompleteExitCriterionStatuses.length
      + staleArtifactIndexes.length
    return {
      status: missingRequirements > 0 ? "failed" : "passed",
      roots: [...artifactRoots],
      inputs: [...artifactIndexes],
      indexPaths,
      requiredArtifactMaxAgeMs,
      staleArtifactIndexes,
      requiredArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
      missingArtifactCoverageAreas,
      requiredArtifactSchemas: [...requiredArtifactSchemas],
      missingArtifactSchemas,
      requiredArtifactKinds: [...requiredArtifactKinds],
      missingArtifactKinds,
      requiredArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
      missingArtifactGeneratedEvidenceKinds,
      requiredArtifactGeneratedEvidenceRepos: [...requiredArtifactGeneratedEvidenceRepos],
      missingArtifactGeneratedEvidenceRepos,
      requiredArtifactGeneratedMatrixArtifactIndexes: [...requiredArtifactGeneratedMatrixArtifactIndexes],
      missingArtifactGeneratedMatrixArtifactIndexes,
      requiredArtifactGeneratedMatrixLimitations: [...requiredArtifactGeneratedMatrixLimitations],
      missingArtifactGeneratedMatrixLimitations,
      requiredArtifactGeneratedMatrixNames: [...requiredArtifactGeneratedMatrixNames],
      missingArtifactGeneratedMatrixNames,
      requiredArtifactGeneratedMatrixRepos: [...requiredArtifactGeneratedMatrixRepos],
      missingArtifactGeneratedMatrixRepos,
      requiredArtifactGeneratedValidationSuiteArtifactIndexes: [...requiredArtifactGeneratedValidationSuiteArtifactIndexes],
      missingArtifactGeneratedValidationSuiteArtifactIndexes,
      requiredArtifactGeneratedValidationSuiteFailureRoots: [...requiredArtifactGeneratedValidationSuiteFailureRoots],
      missingArtifactGeneratedValidationSuiteFailureRoots,
      requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
      missingArtifactEvidenceRepos,
      requiredArtifactProviderAccountAliases: [...requiredArtifactProviderAccountAliases],
      missingArtifactProviderAccountAliases,
      requiredArtifactValidationPresets: [...requiredArtifactValidationPresets],
      missingArtifactValidationPresets,
      requiredArtifactRuntimeAuthorityInvariants: [...requiredArtifactRuntimeAuthorityInvariants],
      missingArtifactRuntimeAuthorityInvariants,
      requiredArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
      missingArtifactRuntimeSignals,
      requiredArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
      missingArtifactRuntimeSignalOwners,
      requiredArtifactOwners: [...requiredArtifactOwners],
      missingArtifactOwners,
      requiredArtifactClassifications: [...requiredArtifactClassifications],
      missingArtifactClassifications,
      requiredArtifactFailureClassifications: [...requiredArtifactFailureClassifications],
      missingArtifactFailureClassifications,
      requiredArtifactPlannedOwners: [...requiredArtifactPlannedOwners],
      missingArtifactPlannedOwners,
      requiredArtifactPlannedClassifications: [...requiredArtifactPlannedClassifications],
      missingArtifactPlannedClassifications,
      requiredArtifactExitCriterionStatuses: [...requiredArtifactExitCriterionStatuses],
      missingArtifactExitCriterionStatuses,
      requiredArtifactIncompleteExitCriterionStatuses: [...requiredArtifactIncompleteExitCriterionStatuses],
      missingArtifactIncompleteExitCriterionStatuses,
      aggregate,
      ...(missingRequirements > 0
        ? {
          error: artifactRequirementError({
            missingArtifactCoverageAreas,
            missingArtifactSchemas,
            missingArtifactKinds,
            missingArtifactGeneratedEvidenceKinds,
            missingArtifactGeneratedEvidenceRepos,
            missingArtifactGeneratedMatrixArtifactIndexes,
            missingArtifactGeneratedMatrixLimitations,
            missingArtifactGeneratedMatrixNames,
            missingArtifactGeneratedMatrixRepos,
            missingArtifactGeneratedValidationSuiteArtifactIndexes,
            missingArtifactGeneratedValidationSuiteFailureRoots,
            missingArtifactEvidenceRepos,
            missingArtifactProviderAccountAliases,
            missingArtifactValidationPresets,
            missingArtifactRuntimeAuthorityInvariants,
            missingArtifactRuntimeSignals,
            missingArtifactRuntimeSignalOwners,
            missingArtifactOwners,
            missingArtifactClassifications,
            missingArtifactFailureClassifications,
            missingArtifactPlannedOwners,
            missingArtifactPlannedClassifications,
            missingArtifactExitCriterionStatuses,
            missingArtifactIncompleteExitCriterionStatuses,
            staleArtifactIndexes,
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
      requiredArtifactMaxAgeMs,
      staleArtifactIndexes: [],
      requiredArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
      missingArtifactCoverageAreas: [...requiredArtifactCoverageAreas],
      requiredArtifactSchemas: [...requiredArtifactSchemas],
      missingArtifactSchemas: [...requiredArtifactSchemas],
      requiredArtifactKinds: [...requiredArtifactKinds],
      missingArtifactKinds: [...requiredArtifactKinds],
      requiredArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
      missingArtifactGeneratedEvidenceKinds: [...requiredArtifactGeneratedEvidenceKinds],
      requiredArtifactGeneratedEvidenceRepos: [...requiredArtifactGeneratedEvidenceRepos],
      missingArtifactGeneratedEvidenceRepos: [...requiredArtifactGeneratedEvidenceRepos],
      requiredArtifactGeneratedMatrixArtifactIndexes: [...requiredArtifactGeneratedMatrixArtifactIndexes],
      missingArtifactGeneratedMatrixArtifactIndexes: [...requiredArtifactGeneratedMatrixArtifactIndexes],
      requiredArtifactGeneratedMatrixLimitations: [...requiredArtifactGeneratedMatrixLimitations],
      missingArtifactGeneratedMatrixLimitations: [...requiredArtifactGeneratedMatrixLimitations],
      requiredArtifactGeneratedMatrixNames: [...requiredArtifactGeneratedMatrixNames],
      missingArtifactGeneratedMatrixNames: [...requiredArtifactGeneratedMatrixNames],
      requiredArtifactGeneratedMatrixRepos: [...requiredArtifactGeneratedMatrixRepos],
      missingArtifactGeneratedMatrixRepos: [...requiredArtifactGeneratedMatrixRepos],
      requiredArtifactGeneratedValidationSuiteArtifactIndexes: [...requiredArtifactGeneratedValidationSuiteArtifactIndexes],
      missingArtifactGeneratedValidationSuiteArtifactIndexes: [...requiredArtifactGeneratedValidationSuiteArtifactIndexes],
      requiredArtifactGeneratedValidationSuiteFailureRoots: [...requiredArtifactGeneratedValidationSuiteFailureRoots],
      missingArtifactGeneratedValidationSuiteFailureRoots: [...requiredArtifactGeneratedValidationSuiteFailureRoots],
      requiredArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
      missingArtifactEvidenceRepos: [...requiredArtifactEvidenceRepos],
      requiredArtifactProviderAccountAliases: [...requiredArtifactProviderAccountAliases],
      missingArtifactProviderAccountAliases: [...requiredArtifactProviderAccountAliases],
      requiredArtifactValidationPresets: [...requiredArtifactValidationPresets],
      missingArtifactValidationPresets: [...requiredArtifactValidationPresets],
      requiredArtifactRuntimeAuthorityInvariants: [...requiredArtifactRuntimeAuthorityInvariants],
      missingArtifactRuntimeAuthorityInvariants: [...requiredArtifactRuntimeAuthorityInvariants],
      requiredArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
      missingArtifactRuntimeSignals: [...requiredArtifactRuntimeSignals],
      requiredArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
      missingArtifactRuntimeSignalOwners: [...requiredArtifactRuntimeSignalOwners],
      requiredArtifactOwners: [...requiredArtifactOwners],
      missingArtifactOwners: [...requiredArtifactOwners],
      requiredArtifactClassifications: [...requiredArtifactClassifications],
      missingArtifactClassifications: [...requiredArtifactClassifications],
      requiredArtifactFailureClassifications: [...requiredArtifactFailureClassifications],
      missingArtifactFailureClassifications: [...requiredArtifactFailureClassifications],
      requiredArtifactPlannedOwners: [...requiredArtifactPlannedOwners],
      missingArtifactPlannedOwners: [...requiredArtifactPlannedOwners],
      requiredArtifactPlannedClassifications: [...requiredArtifactPlannedClassifications],
      missingArtifactPlannedClassifications: [...requiredArtifactPlannedClassifications],
      requiredArtifactExitCriterionStatuses: [...requiredArtifactExitCriterionStatuses],
      missingArtifactExitCriterionStatuses: [...requiredArtifactExitCriterionStatuses],
      requiredArtifactIncompleteExitCriterionStatuses: [...requiredArtifactIncompleteExitCriterionStatuses],
      missingArtifactIncompleteExitCriterionStatuses: [...requiredArtifactIncompleteExitCriterionStatuses],
      error: error instanceof Error ? error.message : String(error),
    }
  }
}

function artifactRequirementError({
  missingArtifactCoverageAreas,
  missingArtifactSchemas,
  missingArtifactKinds,
  missingArtifactGeneratedEvidenceKinds,
  missingArtifactGeneratedEvidenceRepos,
  missingArtifactGeneratedMatrixArtifactIndexes,
  missingArtifactGeneratedMatrixLimitations,
  missingArtifactGeneratedMatrixNames,
  missingArtifactGeneratedMatrixRepos,
  missingArtifactGeneratedValidationSuiteArtifactIndexes,
  missingArtifactGeneratedValidationSuiteFailureRoots,
  missingArtifactEvidenceRepos,
  missingArtifactProviderAccountAliases,
  missingArtifactValidationPresets,
  missingArtifactRuntimeAuthorityInvariants,
  missingArtifactRuntimeSignals,
  missingArtifactRuntimeSignalOwners,
  missingArtifactOwners,
  missingArtifactClassifications,
  missingArtifactFailureClassifications,
  missingArtifactPlannedOwners,
  missingArtifactPlannedClassifications,
  missingArtifactExitCriterionStatuses,
  missingArtifactIncompleteExitCriterionStatuses,
  staleArtifactIndexes,
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
  if (missingArtifactGeneratedEvidenceRepos.length > 0) {
    messages.push(`missing required artifact generated evidence repos: ${missingArtifactGeneratedEvidenceRepos.join(", ")}`)
  }
  if (missingArtifactGeneratedMatrixArtifactIndexes.length > 0) {
    messages.push(`missing required artifact generated matrix artifact indexes: ${missingArtifactGeneratedMatrixArtifactIndexes.join(", ")}`)
  }
  if (missingArtifactGeneratedMatrixLimitations.length > 0) {
    messages.push(`missing required artifact generated matrix limitations: ${missingArtifactGeneratedMatrixLimitations.join(", ")}`)
  }
  if (missingArtifactGeneratedMatrixNames.length > 0) {
    messages.push(`missing required artifact generated matrix names: ${missingArtifactGeneratedMatrixNames.join(", ")}`)
  }
  if (missingArtifactGeneratedMatrixRepos.length > 0) {
    messages.push(`missing required artifact generated matrix repos: ${missingArtifactGeneratedMatrixRepos.join(", ")}`)
  }
  if (missingArtifactGeneratedValidationSuiteArtifactIndexes.length > 0) {
    messages.push(`missing required artifact generated validation-suite artifact indexes: ${missingArtifactGeneratedValidationSuiteArtifactIndexes.join(", ")}`)
  }
  if (missingArtifactGeneratedValidationSuiteFailureRoots.length > 0) {
    messages.push(`missing required artifact generated validation-suite failure roots: ${missingArtifactGeneratedValidationSuiteFailureRoots.join(", ")}`)
  }
  if (missingArtifactEvidenceRepos.length > 0) {
    messages.push(`missing required artifact evidence repos: ${missingArtifactEvidenceRepos.join(", ")}`)
  }
  if (missingArtifactProviderAccountAliases.length > 0) {
    messages.push(`missing required artifact provider account aliases: ${missingArtifactProviderAccountAliases.join(", ")}`)
  }
  if (missingArtifactValidationPresets.length > 0) {
    messages.push(`missing required artifact validation presets: ${missingArtifactValidationPresets.join(", ")}`)
  }
  if (missingArtifactRuntimeAuthorityInvariants.length > 0) {
    messages.push(`missing required artifact runtime authority invariants: ${missingArtifactRuntimeAuthorityInvariants.join(", ")}`)
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
  if (missingArtifactFailureClassifications.length > 0) {
    messages.push(`missing required artifact failure classifications: ${missingArtifactFailureClassifications.join(", ")}`)
  }
  if (missingArtifactPlannedOwners.length > 0) {
    messages.push(`missing required artifact planned owners: ${missingArtifactPlannedOwners.join(", ")}`)
  }
  if (missingArtifactPlannedClassifications.length > 0) {
    messages.push(`missing required artifact planned classifications: ${missingArtifactPlannedClassifications.join(", ")}`)
  }
  if (missingArtifactExitCriterionStatuses.length > 0) {
    messages.push(`missing required artifact exit criterion statuses: ${missingArtifactExitCriterionStatuses.join(", ")}`)
  }
  if (missingArtifactIncompleteExitCriterionStatuses.length > 0) {
    messages.push(`missing required artifact incomplete exit criterion statuses: ${missingArtifactIncompleteExitCriterionStatuses.join(", ")}`)
  }
  if ((staleArtifactIndexes ?? []).length > 0) {
    messages.push(`stale artifact indexes: ${staleArtifactIndexes.map((index) => `${index.source} age_ms=${index.ageMs} max_age_ms=${index.maxAgeMs}`).join(", ")}`)
  }
  return messages.join("; ")
}

function staleArtifactIndexesFor(indexes, sources, { nowMs, requiredArtifactMaxAgeMs }) {
  if (requiredArtifactMaxAgeMs === null) return []
  if (!Number.isSafeInteger(requiredArtifactMaxAgeMs) || requiredArtifactMaxAgeMs < 0) {
    throw new Error("requiredArtifactMaxAgeMs must be a non-negative integer")
  }
  if (!Number.isFinite(nowMs)) {
    throw new Error("nowMs must be finite")
  }
  return indexes
    .map((index, position) => {
      const createdMs = parseDrillIsoTimestamp(index.createdAt, `artifact index ${sources[position] ?? position}.createdAt`)
      return {
        source: sources[position] ?? null,
        createdAt: index.createdAt,
        ageMs: Math.max(0, Math.floor(nowMs - createdMs)),
        maxAgeMs: requiredArtifactMaxAgeMs,
      }
    })
    .filter((entry) => entry.ageMs > requiredArtifactMaxAgeMs)
}
