export const SLICE_SAVE_ACK_LOSS_CASE_IDS = Object.freeze([
  "fault.response-loss",
  "replay.same-process",
  "replay.kernel-restart",
  "guard.command-conflict",
  "cleanup.resources",
])

export const SLICE_SAVE_ACK_LOSS_TEST_NAME =
  "runtime_transport::command_cache::tests::slice_state_save_acknowledgement_replays_without_a_second_dispatch"

const PROBE_PREFIX = "CHARIOX_SLICE_SAVE_ACK_LOSS_PROBE:"
const PROBE_SCHEMA = "chariox.slice_save_ack_loss_probe.v1"
const BOOLEAN_FIELDS = Object.freeze([
  "sameProcessReplay",
  "restartReplay",
  "savedStateRefPreserved",
  "conflictingReuseRejected",
  "cleanupComplete",
])

export function buildSliceSaveAckLossCargoArgs() {
  return [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    SLICE_SAVE_ACK_LOSS_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ]
}

export function parseSliceSaveAckLossProbe(output) {
  const line = String(output ?? "")
    .split("\n")
    .map((candidate) => candidate.trim())
    .findLast((candidate) => candidate.startsWith(PROBE_PREFIX))
  if (!line) throw new Error(`slice save acknowledgement-loss output is missing ${PROBE_SCHEMA}`)

  let probe
  try {
    probe = JSON.parse(line.slice(PROBE_PREFIX.length))
  } catch {
    throw new Error("slice save acknowledgement-loss probe is not valid JSON")
  }
  const expectedKeys = ["schema", ...BOOLEAN_FIELDS].sort()
  if (probe?.schema !== PROBE_SCHEMA) {
    throw new Error(`slice save acknowledgement-loss probe schema must be ${PROBE_SCHEMA}`)
  }
  if (JSON.stringify(Object.keys(probe).sort()) !== JSON.stringify(expectedKeys)) {
    throw new Error("slice save acknowledgement-loss probe fields do not match its schema")
  }
  for (const field of BOOLEAN_FIELDS) {
    if (probe[field] !== true) {
      throw new Error(`slice save acknowledgement-loss probe ${field} must be true`)
    }
  }
  return probe
}
