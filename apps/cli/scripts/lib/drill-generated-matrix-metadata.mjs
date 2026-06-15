import {
  drillGeneratedMatrixRepoForName,
  isKnownDrillGeneratedMatrixName,
} from "./drill-generated-matrix-names.mjs"
import { redactDrillSecretText } from "./drill-secrets.mjs"

export function validateDrillGeneratedMatrixName(matrixName, {
  secretDescription = "generated matrix name",
  secretSource,
  unknownSource,
}) {
  if (redactDrillSecretText(matrixName) !== matrixName) {
    throw new Error(`${secretSource} includes secret-looking ${secretDescription}`)
  }
  if (!isKnownDrillGeneratedMatrixName(matrixName)) {
    throw new Error(`${unknownSource} has unknown generated matrix name ${JSON.stringify(matrixName)}`)
  }
}

export function validateDrillGeneratedMatrixNameRepoCounts(generatedMatrixNames, generatedMatrixRepos, source) {
  if (!generatedMatrixNames || typeof generatedMatrixNames !== "object" || Array.isArray(generatedMatrixNames)) return
  if (!generatedMatrixRepos || typeof generatedMatrixRepos !== "object" || Array.isArray(generatedMatrixRepos)) return
  validateGeneratedMatrixRepoCoverage(Object.keys(generatedMatrixNames), new Set(Object.keys(generatedMatrixRepos)), source)
}

export function validateDrillGeneratedMatrixNameRepoMetadata(matrixNames, matrixRepos, source) {
  if (!Array.isArray(matrixNames) || matrixNames.length === 0) return
  if (!(matrixRepos instanceof Set) || matrixRepos.size === 0) return
  validateGeneratedMatrixRepoCoverage(matrixNames, matrixRepos, source)
}

function validateGeneratedMatrixRepoCoverage(matrixNames, matrixRepos, source) {
  for (const matrixName of matrixNames) {
    const expectedRepo = drillGeneratedMatrixRepoForName(matrixName)
    if (expectedRepo && !matrixRepos.has(expectedRepo)) {
      throw new Error(`${source} has generated matrix ${JSON.stringify(matrixName)} without generated matrix repo ${JSON.stringify(expectedRepo)}`)
    }
  }
}
