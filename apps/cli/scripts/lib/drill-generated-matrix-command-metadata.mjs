import { isKnownDrillArtifactEvidenceRepo } from "./drill-evidence-repos.mjs"
import { validateDrillGeneratedMatrixName } from "./drill-generated-matrix-metadata.mjs"
import {
  drillGeneratedMatrixRepoForName,
} from "./drill-generated-matrix-names.mjs"
import { redactDrillSecretText } from "./drill-secrets.mjs"

export function validateDrillGeneratedMatrixCommandMetadata(command, source) {
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
