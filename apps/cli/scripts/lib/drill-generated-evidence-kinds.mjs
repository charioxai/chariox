export const DRILL_GENERATED_EVIDENCE_KINDS = Object.freeze(["matrix-report", "validation-suite-run"])

export function isKnownDrillGeneratedEvidenceKind(kind) {
  return DRILL_GENERATED_EVIDENCE_KINDS.includes(kind)
}
