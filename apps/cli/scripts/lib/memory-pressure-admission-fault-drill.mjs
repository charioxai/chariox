export const MEMORY_PRESSURE_ADMISSION_CASE_IDS = Object.freeze([
  "fault.memory-pressure",
  "cleanup.resources",
])

export const MEMORY_PRESSURE_ADMISSION_TEST_NAME =
  "slice::local_docker::memory_admission::tests::memory_pressure_admission_fault_probe"

const PROBE_PREFIX = "CHARIOX_MEMORY_PRESSURE_PROBE:"
const PROBE_SCHEMA = "chariox.memory_pressure_admission_probe.v1"
const EXPECTED_DEFAULT_LIMIT_BYTES = 2 * 1024 * 1024 * 1024
const EXPECTED_RESERVE_BYTES = 512 * 1024 * 1024
const BOOLEAN_FIELDS = Object.freeze([
  "admissionClosesBeforeOom",
  "activeStateRemainsConsistent",
  "resourceRecoveryRecorded",
])

export function buildMemoryPressureAdmissionCargoArgs() {
  return [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    MEMORY_PRESSURE_ADMISSION_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ]
}

export function parseMemoryPressureAdmissionProbe(output) {
  const line = String(output ?? "")
    .split("\n")
    .map((candidate) => candidate.trim())
    .findLast((candidate) => candidate.startsWith(PROBE_PREFIX))
  if (!line) throw new Error(`memory pressure output is missing ${PROBE_SCHEMA}`)

  let probe
  try {
    probe = JSON.parse(line.slice(PROBE_PREFIX.length))
  } catch {
    throw new Error("memory pressure probe is not valid JSON")
  }
  const expectedKeys = [
    "schema",
    ...BOOLEAN_FIELDS,
    "defaultSliceLimitBytes",
    "reserveBytes",
  ].sort()
  if (probe?.schema !== PROBE_SCHEMA) throw new Error(`memory pressure probe schema must be ${PROBE_SCHEMA}`)
  if (JSON.stringify(Object.keys(probe).sort()) !== JSON.stringify(expectedKeys)) {
    throw new Error("memory pressure probe fields do not match its schema")
  }
  for (const field of BOOLEAN_FIELDS) {
    if (probe[field] !== true) throw new Error(`memory pressure probe ${field} must be true`)
  }
  if (probe.defaultSliceLimitBytes !== EXPECTED_DEFAULT_LIMIT_BYTES) {
    throw new Error("memory pressure probe must enforce the 2048 MiB default slice limit")
  }
  if (probe.reserveBytes !== EXPECTED_RESERVE_BYTES) {
    throw new Error("memory pressure probe must retain the 512 MiB Docker engine reserve")
  }
  return probe
}
