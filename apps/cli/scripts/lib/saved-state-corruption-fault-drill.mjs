export const SAVED_STATE_CORRUPTION_CASE_IDS = Object.freeze([
  "fault.saved-state-corruption",
  "state.last-known-good",
  "cleanup.resources",
])

export const SAVED_STATE_CORRUPTION_TEST_NAME =
  "slice::local_docker::tests::backup_restore_quarantines_a_corrupt_archive_without_touching_known_good_state"

const PROBE_PREFIX = "CHARIOX_SAVED_STATE_CORRUPTION_PROBE:"
const PROBE_SCHEMA = "chariox.saved_state_corruption_probe.v1"
const BOOLEAN_FIELDS = Object.freeze([
  "corruptArchiveRejected",
  "corruptArchiveQuarantined",
  "restorePathCleared",
  "knownGoodBackupRestorable",
  "cleanupComplete",
])

export function buildSavedStateCorruptionCargoArgs() {
  return [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    SAVED_STATE_CORRUPTION_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ]
}

export function parseSavedStateCorruptionProbe(output) {
  const line = String(output ?? "")
    .split("\n")
    .map((candidate) => candidate.trim())
    .findLast((candidate) => candidate.startsWith(PROBE_PREFIX))
  if (!line) throw new Error(`saved-state corruption output is missing ${PROBE_SCHEMA}`)

  let probe
  try {
    probe = JSON.parse(line.slice(PROBE_PREFIX.length))
  } catch {
    throw new Error("saved-state corruption probe is not valid JSON")
  }
  const expectedKeys = ["schema", ...BOOLEAN_FIELDS].sort()
  if (probe?.schema !== PROBE_SCHEMA) {
    throw new Error(`saved-state corruption probe schema must be ${PROBE_SCHEMA}`)
  }
  if (JSON.stringify(Object.keys(probe).sort()) !== JSON.stringify(expectedKeys)) {
    throw new Error("saved-state corruption probe fields do not match its schema")
  }
  for (const field of BOOLEAN_FIELDS) {
    if (probe[field] !== true) throw new Error(`saved-state corruption probe ${field} must be true`)
  }
  return probe
}
