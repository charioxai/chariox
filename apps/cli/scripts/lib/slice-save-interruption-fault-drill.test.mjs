import assert from "node:assert/strict"
import test from "node:test"

import {
  SLICE_SAVE_INTERRUPTION_CASE_IDS,
  SLICE_SAVE_INTERRUPTION_TEST_NAME,
  buildSliceSaveInterruptionCargoArgs,
  parseSliceSaveInterruptionProbe,
} from "./slice-save-interruption-fault-drill.mjs"

test("slice save interruption drill targets the exact publication-boundary probe", () => {
  assert.deepEqual(SLICE_SAVE_INTERRUPTION_CASE_IDS, [
    "fault.before-manifest-publication",
    "fault.after-manifest-rename",
    "state.last-known-good",
    "cleanup.resources",
  ])
  assert.deepEqual(buildSliceSaveInterruptionCargoArgs(), [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    SLICE_SAVE_INTERRUPTION_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ])
})

test("slice save interruption drill requires both restorable publication outcomes", () => {
  const probe = {
    schema: "chariox.slice_save_interruption_probe.v1",
    preCommitFailurePreservedPrior: true,
    unpublishedGenerationRemoved: true,
    uncertainCommitRetainedPrior: true,
    uncertainCommitRetainedNext: true,
    bothGenerationsRestorable: true,
    cleanupComplete: true,
  }

  assert.deepEqual(
    parseSliceSaveInterruptionProbe(`noise\nCHARIOX_SLICE_SAVE_INTERRUPTION_PROBE:${JSON.stringify(probe)}\n`),
    probe,
  )
  assert.throws(
    () => parseSliceSaveInterruptionProbe(`CHARIOX_SLICE_SAVE_INTERRUPTION_PROBE:${JSON.stringify({
      ...probe,
      uncertainCommitRetainedPrior: false,
    })}`),
    /uncertainCommitRetainedPrior must be true/,
  )
})
