import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_GENERATED_MATRIX_LIMITATIONS,
  DRILL_GENERATED_MATRIX_LIMITATIONS_SCHEMA,
  drillGeneratedMatrixLimitationsManifest,
  validateDrillGeneratedMatrixLimitation,
  validateDrillGeneratedMatrixLimitationsManifest,
} from "./drill-generated-matrix-limitations.mjs"

test("validates generated matrix limitation metadata", () => {
  assert.doesNotThrow(() => validateDrillGeneratedMatrixLimitation("dry-run-classification-coverage", "field[0]"))
  assert.throws(
    () => validateDrillGeneratedMatrixLimitation("dry-run-classification-covergae", "field[0]"),
    /field\[0\] has unknown generated matrix limitation "dry-run-classification-covergae"/,
  )
})

test("builds stable generated matrix limitation manifest", () => {
  const manifest = drillGeneratedMatrixLimitationsManifest()

  assert.equal(manifest.schema, DRILL_GENERATED_MATRIX_LIMITATIONS_SCHEMA)
  assert.deepEqual(manifest.limitations.map((limitation) => limitation.kind), DRILL_GENERATED_MATRIX_LIMITATIONS)
  validateDrillGeneratedMatrixLimitationsManifest(manifest)
})

test("rejects generated matrix limitation manifest drift", () => {
  const manifest = drillGeneratedMatrixLimitationsManifest()

  assert.throws(
    () => validateDrillGeneratedMatrixLimitationsManifest({ ...manifest, schema: "wrong.schema" }),
    /unsupported schema/,
  )
  assert.throws(
    () => validateDrillGeneratedMatrixLimitationsManifest({
      ...manifest,
      limitations: manifest.limitations.map((limitation) => ({ ...limitation, kind: "dry-run-classification-covergae" })),
    }),
    /limitations do not match generated matrix limitation taxonomy/,
  )
})
