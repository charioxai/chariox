export const DRILL_GENERATED_MATRIX_LIMITATIONS = Object.freeze(["dry-run-classification-coverage"])
export const DRILL_GENERATED_MATRIX_LIMITATIONS_SCHEMA = "chariox.drill.generated_matrix_limitations.v1"

export function drillGeneratedMatrixLimitationsManifest() {
  return {
    schema: DRILL_GENERATED_MATRIX_LIMITATIONS_SCHEMA,
    limitations: [{
      kind: "dry-run-classification-coverage",
      owner: "validation-harness",
      description: "Generated matrix reports ran in dry-run mode, so scenario classifications and exit criteria are covered as command contracts rather than live runtime evidence.",
      nextAction: "rerun generated matrix reports without --matrix-dry-run before treating required matrix classifications as release evidence",
    }],
  }
}

export function isKnownDrillGeneratedMatrixLimitation(limitation) {
  return DRILL_GENERATED_MATRIX_LIMITATIONS.includes(limitation)
}

export function validateDrillGeneratedMatrixLimitation(limitation, source, { message } = {}) {
  if (!isKnownDrillGeneratedMatrixLimitation(limitation)) {
    if (message !== undefined) {
      throw new Error(typeof message === "function" ? message(limitation) : message)
    }
    throw new Error(`${source} has unknown generated matrix limitation ${JSON.stringify(limitation)}`)
  }
}

export function validateDrillGeneratedMatrixLimitationsManifest(manifest, source = "generated matrix limitations manifest") {
  if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
    throw new Error(`${source} is not an object`)
  }
  if (manifest.schema !== DRILL_GENERATED_MATRIX_LIMITATIONS_SCHEMA) {
    throw new Error(`${source} has unsupported schema ${JSON.stringify(manifest.schema)}`)
  }
  if (!Array.isArray(manifest.limitations)) {
    throw new Error(`${source} has invalid limitations`)
  }
  const kinds = manifest.limitations.map((limitation) => limitation?.kind).sort()
  if (JSON.stringify(kinds) !== JSON.stringify(DRILL_GENERATED_MATRIX_LIMITATIONS)) {
    throw new Error(`${source} limitations do not match generated matrix limitation taxonomy`)
  }
  for (const [index, limitation] of manifest.limitations.entries()) {
    const limitationSource = `${source}.limitations[${index}]`
    if (!limitation || typeof limitation !== "object" || Array.isArray(limitation)) {
      throw new Error(`${limitationSource} is not an object`)
    }
    for (const key of ["kind", "owner", "description", "nextAction"]) {
      if (typeof limitation[key] !== "string" || limitation[key].trim().length === 0) {
        throw new Error(`${limitationSource} has invalid ${key}`)
      }
    }
  }
}
