import assert from "node:assert/strict"
import test from "node:test"

import {
  SAVED_STATE_CORRUPTION_CASE_IDS,
  SAVED_STATE_CORRUPTION_TEST_NAME,
  buildSavedStateCorruptionCargoArgs,
  parseSavedStateCorruptionProbe,
} from "./saved-state-corruption-fault-drill.mjs"

test("saved-state corruption drill targets the exact quarantine probe", () => {
  assert.deepEqual(SAVED_STATE_CORRUPTION_CASE_IDS, [
    "fault.saved-state-corruption",
    "state.last-known-good",
    "cleanup.resources",
  ])
  assert.deepEqual(buildSavedStateCorruptionCargoArgs(), [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    SAVED_STATE_CORRUPTION_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ])
})

test("saved-state corruption drill requires quarantine and a restorable known-good backup", () => {
  const probe = {
    schema: "chariox.saved_state_corruption_probe.v1",
    corruptArchiveRejected: true,
    corruptArchiveQuarantined: true,
    restorePathCleared: true,
    knownGoodBackupRestorable: true,
    cleanupComplete: true,
  }

  assert.deepEqual(
    parseSavedStateCorruptionProbe(
      `noise\nCHARIOX_SAVED_STATE_CORRUPTION_PROBE:${JSON.stringify(probe)}\n`,
    ),
    probe,
  )
  assert.throws(
    () => parseSavedStateCorruptionProbe(
      `CHARIOX_SAVED_STATE_CORRUPTION_PROBE:${JSON.stringify({
        ...probe,
        knownGoodBackupRestorable: false,
      })}`,
    ),
    /knownGoodBackupRestorable must be true/,
  )
})
