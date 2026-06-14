export const DRILL_ARTIFACT_EVIDENCE_REPOS = Object.freeze(["cloud", "external", "oss"])

export function isKnownDrillArtifactEvidenceRepo(repo) {
  return DRILL_ARTIFACT_EVIDENCE_REPOS.includes(repo)
}
