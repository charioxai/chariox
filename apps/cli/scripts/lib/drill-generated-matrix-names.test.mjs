import assert from "node:assert/strict"
import test from "node:test"

import {
  DRILL_GENERATED_MATRIX_NAMES,
  DRILL_GENERATED_MATRIX_NAMES_SCHEMA,
  drillGeneratedMatrixNamesManifest,
  isKnownDrillGeneratedMatrixName,
  validateDrillGeneratedMatrixNamesManifest,
} from "./drill-generated-matrix-names.mjs"

test("builds stable generated matrix names manifest", () => {
  const manifest = drillGeneratedMatrixNamesManifest()

  assert.equal(manifest.schema, DRILL_GENERATED_MATRIX_NAMES_SCHEMA)
  assert.deepEqual(manifest.matrices, [
    { name: "cloud-slice-runtime-matrix", repo: "cloud" },
    { name: "native-provider-tui-matrix", repo: "oss" },
    { name: "remote-agent-runtime-matrix", repo: "oss" },
    { name: "remote-home-extension-matrix", repo: "oss" },
    { name: "slice-runtime-matrix", repo: "oss" },
    { name: "workspace-live-sync-matrix", repo: "oss" },
  ])
  assert.deepEqual(manifest.matrices.map((matrix) => matrix.name), DRILL_GENERATED_MATRIX_NAMES)
  validateDrillGeneratedMatrixNamesManifest(manifest)
})

test("validates generated matrix name membership", () => {
  assert.equal(isKnownDrillGeneratedMatrixName("workspace-live-sync-matrix"), true)
  assert.equal(isKnownDrillGeneratedMatrixName("workspace-live-synch-matrix"), false)
})

test("rejects generated matrix name manifest drift", () => {
  const manifest = drillGeneratedMatrixNamesManifest()

  assert.throws(
    () => validateDrillGeneratedMatrixNamesManifest({ ...manifest, schema: "wrong.schema" }),
    /unsupported schema/,
  )
  assert.throws(
    () => validateDrillGeneratedMatrixNamesManifest({
      ...manifest,
      matrices: manifest.matrices.filter((matrix) => matrix.name !== "workspace-live-sync-matrix"),
    }),
    /matrices do not match generated matrix name registry/,
  )
  assert.throws(
    () => validateDrillGeneratedMatrixNamesManifest({
      ...manifest,
      matrices: manifest.matrices.map((matrix) => matrix.name === "cloud-slice-runtime-matrix"
        ? { ...matrix, repo: "oss" }
        : matrix),
    }),
    /matrices\[0\] has invalid repo "oss"/,
  )
})
