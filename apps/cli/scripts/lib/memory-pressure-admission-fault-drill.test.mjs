import assert from "node:assert/strict"
import test from "node:test"

import {
  MEMORY_PRESSURE_ADMISSION_CASE_IDS,
  MEMORY_PRESSURE_ADMISSION_TEST_NAME,
  buildMemoryPressureAdmissionCargoArgs,
  parseMemoryPressureAdmissionProbe,
} from "./memory-pressure-admission-fault-drill.mjs"

const probe = {
  schema: "chariox.memory_pressure_admission_probe.v1",
  admissionClosesBeforeOom: true,
  activeStateRemainsConsistent: true,
  engineLockExclusive: true,
  existingTargetLimitReserved: true,
  resourceRecoveryRecorded: true,
  unboundedSliceRejected: true,
  defaultSliceLimitBytes: 2 * 1024 * 1024 * 1024,
  reserveBytes: 512 * 1024 * 1024,
}

test("memory pressure drill runs only the exact kernel library probe", () => {
  assert.deepEqual(buildMemoryPressureAdmissionCargoArgs(), [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    MEMORY_PRESSURE_ADMISSION_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ])
  assert.deepEqual(MEMORY_PRESSURE_ADMISSION_CASE_IDS, [
    "fault.memory-pressure",
    "cleanup.resources",
  ])
})

test("memory pressure drill requires safe rejection and recovery", () => {
  assert.deepEqual(
    parseMemoryPressureAdmissionProbe(`noise\nCHARIOX_MEMORY_PRESSURE_PROBE:${JSON.stringify(probe)}\n`),
    probe,
  )
  assert.throws(
    () => parseMemoryPressureAdmissionProbe(`CHARIOX_MEMORY_PRESSURE_PROBE:${JSON.stringify({ ...probe, admissionClosesBeforeOom: false })}`),
    /admissionClosesBeforeOom must be true/,
  )
  assert.throws(
    () => parseMemoryPressureAdmissionProbe(`CHARIOX_MEMORY_PRESSURE_PROBE:${JSON.stringify({ ...probe, defaultSliceLimitBytes: 0 })}`),
    /2048 MiB default slice limit/,
  )
})
