import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_ARTIFACT_EVIDENCE_REPOS,
  isKnownDrillArtifactEvidenceRepo,
  validateDrillArtifactEvidenceRepo,
} from "./drill-evidence-repos.mjs"

test("drill artifact evidence repos are stable", () => {
  assert.deepEqual(DRILL_ARTIFACT_EVIDENCE_REPOS, ["cloud", "external", "oss"])
  assert.equal(isKnownDrillArtifactEvidenceRepo("oss"), true)
  assert.equal(isKnownDrillArtifactEvidenceRepo("clodu"), false)
})

test("validates drill artifact evidence repos with caller context", () => {
  assert.doesNotThrow(() => validateDrillArtifactEvidenceRepo("cloud", "artifact.metadata.evidenceRepos"))
  assert.throws(
    () => validateDrillArtifactEvidenceRepo("clodu", "artifact.metadata.evidenceRepos"),
    /artifact\.metadata\.evidenceRepos has unknown evidence repo "clodu"/,
  )
  assert.throws(
    () => validateDrillArtifactEvidenceRepo("clodu", "preset.requiredArtifactEvidenceRepos[0]", {
      label: "artifact evidence repo",
    }),
    /preset\.requiredArtifactEvidenceRepos\[0\] has unknown artifact evidence repo "clodu"/,
  )
  assert.throws(
    () => validateDrillArtifactEvidenceRepo("clodu", "required artifact evidence repos", {
      message: (repo) => `unknown required artifact evidence repo: ${repo}`,
    }),
    /unknown required artifact evidence repo: clodu/,
  )
})
