import assert from "node:assert/strict"
import test from "node:test"

import {
  DISK_PRESSURE_ADMISSION_CASE_IDS,
  DISK_PRESSURE_ADMISSION_TEST_NAME,
  buildDiskPressureAdmissionCargoArgs,
  parseDiskPressureAdmissionProbe,
} from "./disk-pressure-admission-fault-drill.mjs"

const probe = {
  schema: "chariox.disk_pressure_admission_probe.v2",
  independentAdmissionRejectsLowCapacity: true,
  independentAdmissionAcceptsRecoveredCapacity: true,
  lastKnownGoodPreserved: true,
  unsupportedCaptureRefusesBeforeMutation: true,
  reserveBytes: 2 * 1024 * 1024 * 1024,
}

test("disk pressure drill runs only the exact kernel library probe", () => {
  assert.deepEqual(buildDiskPressureAdmissionCargoArgs(), [
    "test", "-p", "chariox-kernel", "--lib", DISK_PRESSURE_ADMISSION_TEST_NAME,
    "--", "--exact", "--nocapture",
  ])
  assert.deepEqual(DISK_PRESSURE_ADMISSION_CASE_IDS, [
    "fault.disk-pressure",
    "cleanup.resources",
  ])
})

test("disk pressure drill requires rejection, preservation, and recovery", () => {
  assert.deepEqual(
    parseDiskPressureAdmissionProbe(`noise\nCHARIOX_DISK_PRESSURE_PROBE:${JSON.stringify(probe)}\n`),
    probe,
  )
  // Single-threaded libtest prints the probe on the test's status line.
  assert.deepEqual(
    parseDiskPressureAdmissionProbe(`test ${DISK_PRESSURE_ADMISSION_TEST_NAME} ... CHARIOX_DISK_PRESSURE_PROBE:${JSON.stringify(probe)}\nok`),
    probe,
  )
  assert.throws(
    () => parseDiskPressureAdmissionProbe(`test ${DISK_PRESSURE_ADMISSION_TEST_NAME} ... CHARIOX_DISK_PRESSURE_PROBE:${JSON.stringify({ ...probe, schema: "chariox.disk_pressure_admission_probe.v1" })}`),
    /schema must be/,
  )
  assert.throws(
    () => parseDiskPressureAdmissionProbe(`CHARIOX_DISK_PRESSURE_PROBE:${JSON.stringify({ ...probe, lastKnownGoodPreserved: false })}`),
    /lastKnownGoodPreserved must be true/,
  )
  assert.throws(
    () => parseDiskPressureAdmissionProbe(`CHARIOX_DISK_PRESSURE_PROBE:${JSON.stringify({ ...probe, reserveBytes: 0 })}`),
    /2048 MiB storage reserve/,
  )
})
