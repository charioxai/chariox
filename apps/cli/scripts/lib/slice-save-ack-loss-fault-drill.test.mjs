import assert from "node:assert/strict"
import test from "node:test"

import {
  SLICE_SAVE_ACK_LOSS_CASE_IDS,
  SLICE_SAVE_ACK_LOSS_TEST_NAME,
  buildSliceSaveAckLossCargoArgs,
  parseSliceSaveAckLossProbe,
} from "./slice-save-ack-loss-fault-drill.mjs"

const probe = {
  schema: "chariox.slice_save_ack_loss_probe.v1",
  sameProcessReplay: true,
  restartReplay: true,
  savedStateRefPreserved: true,
  conflictingReuseRejected: true,
  cleanupComplete: true,
}

test("slice save acknowledgement-loss drill runs only the exact kernel library probe", () => {
  assert.deepEqual(buildSliceSaveAckLossCargoArgs(), [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    SLICE_SAVE_ACK_LOSS_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ])
  assert.deepEqual(SLICE_SAVE_ACK_LOSS_CASE_IDS, [
    "fault.response-loss",
    "replay.same-process",
    "replay.kernel-restart",
    "guard.command-conflict",
    "cleanup.resources",
  ])
})

test("slice save acknowledgement-loss drill requires exact replay and cleanup", () => {
  assert.deepEqual(
    parseSliceSaveAckLossProbe(`noise\nCHARIOX_SLICE_SAVE_ACK_LOSS_PROBE:${JSON.stringify(probe)}\n`),
    probe,
  )
  assert.throws(
    () => parseSliceSaveAckLossProbe(`CHARIOX_SLICE_SAVE_ACK_LOSS_PROBE:${JSON.stringify({ ...probe, restartReplay: false })}`),
    /restartReplay must be true/,
  )
  assert.throws(
    () => parseSliceSaveAckLossProbe("test result: ok"),
    /missing chariox\.slice_save_ack_loss_probe\.v1/,
  )
})
