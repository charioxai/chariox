export const DRILL_GENERATED_MATRIX_LIMITATIONS = Object.freeze(["dry-run-classification-coverage"])

export function isKnownDrillGeneratedMatrixLimitation(limitation) {
  return DRILL_GENERATED_MATRIX_LIMITATIONS.includes(limitation)
}
