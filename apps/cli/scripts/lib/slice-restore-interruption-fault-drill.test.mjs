import assert from "node:assert/strict"
import test from "node:test"

import {
  SLICE_RESTORE_INTERRUPTION_CASE_IDS,
  SLICE_RESTORE_INTERRUPTION_TEST_NAME,
  buildSliceRestoreInterruptionCargoArgs,
  parseSliceRestoreInterruptionProbe,
} from "./slice-restore-interruption-fault-drill.mjs"

const probe = {
  schema: "chariox.slice_restore_interruption_probe.v2",
  childInterruptedDuringStartupRecovery: true,
  postTargetCreationInterruptionStillRequiresProtectedLiveFixture: true,
  durableIntentSurvived: true,
  rollbackRestoredOnRestart: true,
  partialRuntimeRemoved: true,
  priorGenerationRecoverable: true,
  noCommittedRestore: true,
  cleanupComplete: true,
  backendRestoreCount: 2,
  recoveredStateRef: "restore-interruption",
}

test("slice restore interruption drill runs only the exact kernel library probe", () => {
  assert.deepEqual(buildSliceRestoreInterruptionCargoArgs(), [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    SLICE_RESTORE_INTERRUPTION_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ])
  assert.deepEqual(SLICE_RESTORE_INTERRUPTION_CASE_IDS, [
    "fault.interrupted-startup-recovery",
    "journal.retained-pending-intent",
    "recovery.startup-rollback",
    "state.last-known-good",
    "cleanup.partial-runtime",
    "cleanup.resources",
  ])
})

test("slice restore interruption drill requires durable rollback and exact cleanup", () => {
  assert.deepEqual(
    parseSliceRestoreInterruptionProbe(`noise\nCHARIOX_SLICE_RESTORE_INTERRUPTION_PROBE:${JSON.stringify(probe)}\n`),
    probe,
  )
  assert.throws(
    () => parseSliceRestoreInterruptionProbe(`CHARIOX_SLICE_RESTORE_INTERRUPTION_PROBE:${JSON.stringify({ ...probe, durableIntentSurvived: false })}`),
    /durableIntentSurvived must be true/,
  )
  assert.throws(
    () => parseSliceRestoreInterruptionProbe(`CHARIOX_SLICE_RESTORE_INTERRUPTION_PROBE:${JSON.stringify({ ...probe, backendRestoreCount: 1 })}`),
    /backendRestoreCount must be 2/,
  )
  assert.throws(
    () => parseSliceRestoreInterruptionProbe("test result: ok"),
    /missing chariox\.slice_restore_interruption_probe\.v2/,
  )
})

// libtest may write the test name on the same line as --nocapture output.
test("probe accepts the actual libtest prefix and rejects stale or broadened evidence", () => {
  assert.deepEqual(parseSliceRestoreInterruptionProbe(`test ${SLICE_RESTORE_INTERRUPTION_TEST_NAME} ... CHARIOX_SLICE_RESTORE_INTERRUPTION_PROBE:${JSON.stringify(probe)}\nok`), probe)
  assert.throws(() => parseSliceRestoreInterruptionProbe(`CHARIOX_SLICE_RESTORE_INTERRUPTION_PROBE:${JSON.stringify({ ...probe, schema: "chariox.slice_restore_interruption_probe.v1" })}`), /schema must be/)
  assert.throws(() => parseSliceRestoreInterruptionProbe(`CHARIOX_SLICE_RESTORE_INTERRUPTION_PROBE:${JSON.stringify({ ...probe, postTargetCreationInterruptionStillRequiresProtectedLiveFixture: false })}`), /postTargetCreationInterruptionStillRequiresProtectedLiveFixture must be true/)
  assert.throws(() => parseSliceRestoreInterruptionProbe(`CHARIOX_SLICE_RESTORE_INTERRUPTION_PROBE:${JSON.stringify({ ...probe, claimedFullCapture: true })}`), /fields do not match/)
})
