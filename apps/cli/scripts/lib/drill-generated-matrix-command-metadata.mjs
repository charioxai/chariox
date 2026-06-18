import { validateDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import { validateDrillGeneratedMatrixName } from "./drill-generated-matrix-metadata.mjs"
import {
  drillGeneratedMatrixRepoForName,
} from "./drill-generated-matrix-names.mjs"
import { redactDrillSecretText } from "./drill-secrets.mjs"

const GENERATED_MATRIX_ARTIFACT_INDEX_FLAGS = Object.freeze([
  "--artifact-index",
  "--output-artifact-index",
])

export function validateDrillGeneratedMatrixCommandMetadata(command, source) {
  if (command.artifactIndexFlag !== undefined) {
    if (!GENERATED_MATRIX_ARTIFACT_INDEX_FLAGS.includes(command.artifactIndexFlag)) {
      throw new Error(`${source}.artifactIndexFlag has unknown generated matrix artifact index flag ${JSON.stringify(command.artifactIndexFlag)}`)
    }
  }
  if (command.matrix !== undefined) {
    if (!nonEmptyString(command.matrix)) {
      throw new Error(`${source} has invalid matrix`)
    }
    validateDrillGeneratedMatrixName(command.matrix, {
      secretDescription: "generated matrix metadata",
      secretSource: `${source}.matrix`,
      unknownSource: `${source}.matrix`,
    })
  }
  if (command.repo !== undefined) {
    if (!nonEmptyString(command.repo)) {
      throw new Error(`${source} has invalid repo`)
    }
    if (redactDrillSecretText(command.repo) !== command.repo) {
      throw new Error(`${source}.repo includes secret-looking generated matrix metadata`)
    }
    validateDrillArtifactEvidenceRepo(command.repo, `${source}.repo`)
  }
  if (command.matrix !== undefined && command.repo !== undefined) {
    const expectedRepo = drillGeneratedMatrixRepoForName(command.matrix)
    if (command.repo !== expectedRepo) {
      throw new Error(`${source}.repo does not match generated matrix ${JSON.stringify(command.matrix)}`)
    }
  }
}

function nonEmptyString(value) {
  return typeof value === "string" && value.length > 0
}
