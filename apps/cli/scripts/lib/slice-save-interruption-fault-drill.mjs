export const SLICE_SAVE_INTERRUPTION_CASE_IDS = Object.freeze([
  "fault.before-manifest-publication",
  "fault.after-manifest-rename",
  "state.last-known-good",
  "cleanup.resources",
])

export const SLICE_SAVE_INTERRUPTION_TEST_NAME =
  "slice::local_docker::tests::saved_state_publication_interruption_preserves_restorable_generations"

const PROBE_PREFIX = "CHARIOX_SLICE_SAVE_INTERRUPTION_PROBE:"
const PROBE_SCHEMA = "chariox.slice_save_interruption_probe.v1"
const BOOLEAN_FIELDS = Object.freeze([
  "preCommitFailurePreservedPrior",
  "unpublishedGenerationRemoved",
  "uncertainCommitRetainedPrior",
  "uncertainCommitRetainedNext",
  "bothGenerationsRestorable",
  "cleanupComplete",
])

export function buildSliceSaveInterruptionCargoArgs() {
  return [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    SLICE_SAVE_INTERRUPTION_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ]
}

export function parseSliceSaveInterruptionProbe(output) {
  const line = String(output ?? "")
    .split("\n")
    .map((candidate) => candidate.trim())
    .findLast((candidate) => candidate.startsWith(PROBE_PREFIX))
  if (!line) throw new Error(`slice save interruption output is missing ${PROBE_SCHEMA}`)

  let probe
  try {
    probe = JSON.parse(line.slice(PROBE_PREFIX.length))
  } catch {
    throw new Error("slice save interruption probe is not valid JSON")
  }
  const expectedKeys = ["schema", ...BOOLEAN_FIELDS].sort()
  if (probe?.schema !== PROBE_SCHEMA) {
    throw new Error(`slice save interruption probe schema must be ${PROBE_SCHEMA}`)
  }
  if (JSON.stringify(Object.keys(probe).sort()) !== JSON.stringify(expectedKeys)) {
    throw new Error("slice save interruption probe fields do not match its schema")
  }
  for (const field of BOOLEAN_FIELDS) {
    if (probe[field] !== true) throw new Error(`slice save interruption probe ${field} must be true`)
  }
  return probe
}
