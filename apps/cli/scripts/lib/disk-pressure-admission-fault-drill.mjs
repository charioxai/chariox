export const DISK_PRESSURE_ADMISSION_CASE_IDS = Object.freeze([
  "fault.disk-pressure",
  "cleanup.resources",
])

export const DISK_PRESSURE_ADMISSION_TEST_NAME =
  "slice::local_docker::disk_admission::tests::disk_pressure_admission_fault_probe"

const PROBE_PREFIX = "CHARIOX_DISK_PRESSURE_PROBE:"
const PROBE_SCHEMA = "chariox.disk_pressure_admission_probe.v1"
const EXPECTED_RESERVE_BYTES = 2 * 1024 * 1024 * 1024
const BOOLEAN_FIELDS = Object.freeze([
  "activeStateRemainsConsistent",
  "admissionClosesBeforeEnospc",
  "lastKnownGoodPreserved",
  "resourceRecoveryRecorded",
])

export function buildDiskPressureAdmissionCargoArgs() {
  return [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    DISK_PRESSURE_ADMISSION_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ]
}

export function parseDiskPressureAdmissionProbe(output) {
  const line = String(output ?? "")
    .split("\n")
    .map((candidate) => candidate.trim())
    .findLast((candidate) => candidate.startsWith(PROBE_PREFIX))
  if (!line) throw new Error(`disk pressure output is missing ${PROBE_SCHEMA}`)

  let probe
  try {
    probe = JSON.parse(line.slice(PROBE_PREFIX.length))
  } catch {
    throw new Error("disk pressure probe is not valid JSON")
  }
  const expectedKeys = ["schema", ...BOOLEAN_FIELDS, "reserveBytes"].sort()
  if (probe?.schema !== PROBE_SCHEMA) throw new Error(`disk pressure probe schema must be ${PROBE_SCHEMA}`)
  if (JSON.stringify(Object.keys(probe).sort()) !== JSON.stringify(expectedKeys)) {
    throw new Error("disk pressure probe fields do not match its schema")
  }
  for (const field of BOOLEAN_FIELDS) {
    if (probe[field] !== true) throw new Error(`disk pressure probe ${field} must be true`)
  }
  if (probe.reserveBytes !== EXPECTED_RESERVE_BYTES) {
    throw new Error("disk pressure probe must retain the 2048 MiB storage reserve")
  }
  return probe
}
