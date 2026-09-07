import assert from "node:assert/strict"
import test from "node:test"

import {
  DISK_PRESSURE_ADMISSION_CASE_IDS,
  DISK_PRESSURE_ADMISSION_TEST_NAME,
  buildDiskPressureAdmissionCargoArgs,
  parseDiskPressureAdmissionProbe,
} from "./disk-pressure-admission-fault-drill.mjs"

const probe = {
  schema: "chariox.disk_pressure_admission_probe.v1",
  activeStateRemainsConsistent: true,
  admissionClosesBeforeEnospc: true,
  lastKnownGoodPreserved: true,
  resourceRecoveryRecorded: true,
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
  assert.throws(
    () => parseDiskPressureAdmissionProbe(`CHARIOX_DISK_PRESSURE_PROBE:${JSON.stringify({ ...probe, lastKnownGoodPreserved: false })}`),
    /lastKnownGoodPreserved must be true/,
  )
  assert.throws(
    () => parseDiskPressureAdmissionProbe(`CHARIOX_DISK_PRESSURE_PROBE:${JSON.stringify({ ...probe, reserveBytes: 0 })}`),
    /2048 MiB storage reserve/,
  )
})
