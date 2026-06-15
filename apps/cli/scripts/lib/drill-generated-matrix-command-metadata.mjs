import { isKnownDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import {
  drillGeneratedMatrixRepoForName,
  isKnownDrillGeneratedMatrixName,
} from "./drill-generated-matrix-names.mjs"
import { redactDrillSecretText } from "./drill-secrets.mjs"

export function validateDrillGeneratedMatrixCommandMetadata(command, source) {
  if (command.matrix !== undefined) {
    if (!nonEmptyString(command.matrix)) {
      throw new Error(`${source} has invalid matrix`)
    }
    if (redactDrillSecretText(command.matrix) !== command.matrix) {
      throw new Error(`${source}.matrix includes secret-looking generated matrix metadata`)
    }
    if (!isKnownDrillGeneratedMatrixName(command.matrix)) {
      throw new Error(`${source}.matrix has unknown generated matrix name ${JSON.stringify(command.matrix)}`)
    }
  }
  if (command.repo !== undefined) {
    if (!nonEmptyString(command.repo)) {
      throw new Error(`${source} has invalid repo`)
    }
    if (redactDrillSecretText(command.repo) !== command.repo) {
      throw new Error(`${source}.repo includes secret-looking generated matrix metadata`)
    }
    if (!isKnownDrillArtifactEvidenceRepo(command.repo)) {
      throw new Error(`${source}.repo has unknown evidence repo ${JSON.stringify(command.repo)}`)
    }
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
