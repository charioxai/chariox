import assert from "node:assert/strict"
import test from "node:test"

import { validateDrillGeneratedMatrixCommandMetadata } from "./drill-generated-matrix-command-metadata.mjs"

test("validates generated matrix command metadata", () => {
  assert.doesNotThrow(() => validateDrillGeneratedMatrixCommandMetadata({
    matrix: "workspace-live-sync-matrix",
    repo: "oss",
  }, "command"))
  assert.doesNotThrow(() => validateDrillGeneratedMatrixCommandMetadata({}, "command"))

  assert.throws(
    () => validateDrillGeneratedMatrixCommandMetadata({ matrix: "" }, "command"),
    /command has invalid matrix/,
  )
  assert.throws(
    () => validateDrillGeneratedMatrixCommandMetadata({ matrix: "workspace-live-synch-matrix" }, "command"),
    /command\.matrix has unknown generated matrix name "workspace-live-synch-matrix"/,
  )
  assert.throws(
    () => validateDrillGeneratedMatrixCommandMetadata({ repo: "" }, "command"),
    /command has invalid repo/,
  )
  assert.throws(
    () => validateDrillGeneratedMatrixCommandMetadata({ repo: "osz" }, "command"),
    /command\.repo has unknown evidence repo "osz"/,
  )
  assert.throws(
    () => validateDrillGeneratedMatrixCommandMetadata({
      matrix: "cloud-slice-runtime-matrix",
      repo: "oss",
    }, "command"),
    /command\.repo does not match generated matrix "cloud-slice-runtime-matrix"/,
  )
  assert.throws(
    () => validateDrillGeneratedMatrixCommandMetadata({ matrix: "Bearer abcdefghijklmnop" }, "command"),
    /command\.matrix includes secret-looking generated matrix metadata/,
  )
})
