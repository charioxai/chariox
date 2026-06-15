export const DRILL_ARTIFACT_KINDS = Object.freeze([
  "artifact-index",
  "generated-matrix-artifact-index",
  "generated-matrix-root",
  "generated-validation-suite-root",
  "matrix-report",
  "staging-smoke-report",
  "validation-gate",
  "validation-gate-aggregate",
  "validation-gate-report",
  "validation-suite",
  "validation-suite-run",
  "validation-suite-run-report",
])

export function isKnownDrillArtifactKind(kind) {
  return DRILL_ARTIFACT_KINDS.includes(kind)
}
