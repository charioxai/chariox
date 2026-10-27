import assert from "node:assert/strict"
import test from "node:test"

import {
  validateDrillGeneratedEvidenceKind,
  validateDrillGeneratedEvidencePath,
} from "./drill-generated-evidence-metadata.mjs"

test("validates generated evidence metadata with caller-owned sources", () => {
  assert.doesNotThrow(() => validateDrillGeneratedEvidenceKind("matrix-report", "field[0]"))
  assert.doesNotThrow(() => validateDrillGeneratedEvidencePath("/tmp/report.json", "field[0]"))

  assert.throws(
    () => validateDrillGeneratedEvidenceKind("matrix-reprot", "field[0]"),
    /field\[0\] has unknown generated evidence kind "matrix-reprot"/,
  )
  assert.throws(
    () => validateDrillGeneratedEvidenceKind("matrix-reprot", "requiredGeneratedEvidenceKinds", {
      message: (kind) => `unknown required generated evidence kind: ${kind}`,
    }),
    /unknown required generated evidence kind: matrix-reprot/,
  )
  assert.throws(
    () => validateDrillGeneratedEvidencePath("/tmp/Bearer abcdefghijklmnop.json", "field[0]"),
    /field\[0\] includes secret-looking generated evidence path/,
  )
})
