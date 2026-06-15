export const DRILL_ARTIFACT_EVIDENCE_REPOS = Object.freeze(["cloud", "external", "oss"])

export function isKnownDrillArtifactEvidenceRepo(repo) {
  return DRILL_ARTIFACT_EVIDENCE_REPOS.includes(repo)
}

export function validateDrillArtifactEvidenceRepo(repo, source, { label = "evidence repo", message } = {}) {
  if (!isKnownDrillArtifactEvidenceRepo(repo)) {
    if (message !== undefined) {
      throw new Error(typeof message === "function" ? message(repo) : message)
    }
    throw new Error(`${source} has unknown ${label} ${JSON.stringify(repo)}`)
  }
}
