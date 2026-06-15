import assert from "node:assert/strict"
import test from "node:test"

import {
  validateDrillGeneratedMatrixName,
  validateDrillGeneratedMatrixNameRepoCounts,
  validateDrillGeneratedMatrixNameRepoMetadata,
} from "./drill-generated-matrix-metadata.mjs"

test("validates generated matrix names with caller-owned sources", () => {
  assert.doesNotThrow(() => validateDrillGeneratedMatrixName("workspace-live-sync-matrix", {
    secretSource: "field[0]",
    unknownSource: "field",
  }))
  assert.throws(
    () => validateDrillGeneratedMatrixName("Bearer abcdefghijklmnop", {
      secretSource: "field[0]",
      unknownSource: "field",
    }),
    /field\[0\] includes secret-looking generated matrix name/,
  )
  assert.throws(
    () => validateDrillGeneratedMatrixName("workspace-live-synch-matrix", {
      secretSource: "field[0]",
      unknownSource: "field",
    }),
    /field has unknown generated matrix name "workspace-live-synch-matrix"/,
  )
})

test("validates generated matrix name repo coverage", () => {
  assert.doesNotThrow(() => validateDrillGeneratedMatrixNameRepoCounts(
    { "workspace-live-sync-matrix": 1 },
    { oss: 1 },
    "diagnostics",
  ))
  assert.doesNotThrow(() => validateDrillGeneratedMatrixNameRepoMetadata(
    ["cloud-slice-runtime-matrix"],
    new Set(["cloud"]),
    "metadata.generatedMatrixNames",
  ))

  assert.throws(
    () => validateDrillGeneratedMatrixNameRepoCounts(
      { "cloud-slice-runtime-matrix": 1 },
      { oss: 1 },
      "diagnostics",
    ),
    /diagnostics has generated matrix "cloud-slice-runtime-matrix" without generated matrix repo "cloud"/,
  )
  assert.throws(
    () => validateDrillGeneratedMatrixNameRepoMetadata(
      ["cloud-slice-runtime-matrix"],
      new Set(["oss"]),
      "metadata.generatedMatrixNames",
    ),
    /metadata\.generatedMatrixNames has generated matrix "cloud-slice-runtime-matrix" without generated matrix repo "cloud"/,
  )
})
