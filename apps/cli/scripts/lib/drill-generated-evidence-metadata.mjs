import { isKnownDrillGeneratedEvidenceKind } from "./drill-generated-evidence-kinds.mjs"
import { redactDrillSecretText } from "./drill-secrets.mjs"

export function validateDrillGeneratedEvidenceKind(kind, source) {
  if (!isKnownDrillGeneratedEvidenceKind(kind)) {
    throw new Error(`${source} has unknown generated evidence kind ${JSON.stringify(kind)}`)
  }
}

export function validateDrillGeneratedEvidencePath(value, source) {
  if (redactDrillSecretText(value) !== value) {
    throw new Error(`${source} includes secret-looking generated evidence path`)
  }
}
