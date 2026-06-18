import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_ARTIFACT_KINDS,
  isKnownDrillArtifactKind,
  validateDrillArtifactKind,
} from "./drill-artifact-kinds.mjs"

test("drill artifact kinds include stable validation artifacts", () => {
  assert(DRILL_ARTIFACT_KINDS.includes("artifact-index"))
  assert(DRILL_ARTIFACT_KINDS.includes("focused-runtime-gate"))
  assert(DRILL_ARTIFACT_KINDS.includes("validation-suite-run"))
  assert.equal(isKnownDrillArtifactKind("validation-gate"), true)
  assert.equal(isKnownDrillArtifactKind("validation-sutie"), false)
})

test("validates drill artifact kinds with caller context", () => {
  assert.doesNotThrow(() => validateDrillArtifactKind("matrix-report", "artifact.metadata.artifactKinds"))
  assert.throws(
    () => validateDrillArtifactKind("validation-sutie", "artifact.metadata.artifactKinds"),
    /artifact\.metadata\.artifactKinds has unknown artifact kind "validation-sutie"/,
  )
  assert.throws(
    () => validateDrillArtifactKind("validation-sutie", "required artifact kinds", {
      message: (kind) => `unknown required artifact kind: ${kind}`,
    }),
    /unknown required artifact kind: validation-sutie/,
  )
})
