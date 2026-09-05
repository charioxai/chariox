export const SLICE_RESTORE_INTERRUPTION_CASE_IDS = Object.freeze([
  "fault.restore-after-container-create",
  "journal.intent-before-mutation",
  "recovery.startup-rollback",
  "state.last-known-good",
  "cleanup.partial-runtime",
  "cleanup.resources",
])

export const SLICE_RESTORE_INTERRUPTION_TEST_NAME =
  "runtime_transport::tests::slice_backup_restore_interruption_after_container_creation_rolls_back_on_restart"

const PROBE_PREFIX = "CHARIOX_SLICE_RESTORE_INTERRUPTION_PROBE:"
const PROBE_SCHEMA = "chariox.slice_restore_interruption_probe.v1"
const BOOLEAN_FIELDS = Object.freeze([
  "childInterruptedAfterReplacement",
  "durableIntentSurvived",
  "rollbackRestoredOnRestart",
  "partialRuntimeRemoved",
  "priorGenerationRecoverable",
  "noCommittedRestore",
  "cleanupComplete",
])

export function buildSliceRestoreInterruptionCargoArgs() {
  return [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    SLICE_RESTORE_INTERRUPTION_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ]
}

export function parseSliceRestoreInterruptionProbe(output) {
  const line = String(output ?? "")
    .split("\n")
    .map((candidate) => candidate.trim())
    .findLast((candidate) => candidate.startsWith(PROBE_PREFIX))
  if (!line) throw new Error(`slice restore interruption output is missing ${PROBE_SCHEMA}`)

  let probe
  try {
    probe = JSON.parse(line.slice(PROBE_PREFIX.length))
  } catch {
    throw new Error("slice restore interruption probe is not valid JSON")
  }
  const expectedKeys = ["schema", "backendRestoreCount", "recoveredStateRef", ...BOOLEAN_FIELDS].sort()
  if (probe?.schema !== PROBE_SCHEMA) {
    throw new Error(`slice restore interruption probe schema must be ${PROBE_SCHEMA}`)
  }
  if (JSON.stringify(Object.keys(probe).sort()) !== JSON.stringify(expectedKeys)) {
    throw new Error("slice restore interruption probe fields do not match its schema")
  }
  for (const field of BOOLEAN_FIELDS) {
    if (probe[field] !== true) throw new Error(`slice restore interruption probe ${field} must be true`)
  }
  if (probe.backendRestoreCount !== 2) {
    throw new Error("slice restore interruption probe backendRestoreCount must be 2")
  }
  if (typeof probe.recoveredStateRef !== "string" || probe.recoveredStateRef.length === 0) {
    throw new Error("slice restore interruption probe recoveredStateRef must be non-empty")
  }
  return probe
}
