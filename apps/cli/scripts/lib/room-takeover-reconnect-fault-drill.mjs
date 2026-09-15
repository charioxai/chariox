export const ROOM_TAKEOVER_RECONNECT_CASE_IDS = Object.freeze([
  "fault.takeover-response-loss",
  "reconnect.command-replay",
  "authority.human-retained",
  "authority.agent-blocked",
  "authority.explicit-release",
  "effect.takeover-exactly-once",
  "cleanup.resources",
])

export const ROOM_TAKEOVER_RECONNECT_TEST_NAME =
  "runtime_transport::tests::room_takeover_response_loss_and_reconnect_retain_human_input_authority"

const PROBE_PREFIX = "CHARIOX_ROOM_TAKEOVER_RECONNECT_PROBE:"
const PROBE_SCHEMA = "chariox.room_takeover_reconnect_probe.v1"
const BOOLEAN_FIELDS = Object.freeze([
  "responseLostAfterCommit",
  "replayedResponseMatched",
  "humanOwnershipRetained",
  "agentMutationBlocked",
  "takeoverAppliedExactlyOnce",
  "explicitReleaseRequired",
  "agentMutationAdmittedAfterRelease",
  "cleanupComplete",
])

export function buildRoomTakeoverReconnectCargoArgs() {
  return [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    ROOM_TAKEOVER_RECONNECT_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ]
}

export function parseRoomTakeoverReconnectProbe(output) {
  const line = String(output ?? "")
    .split("\n")
    .map((candidate) => candidate.trim())
    .findLast((candidate) => candidate.startsWith(PROBE_PREFIX))
  if (!line) throw new Error(`Room takeover reconnect output is missing ${PROBE_SCHEMA}`)

  let probe
  try {
    probe = JSON.parse(line.slice(PROBE_PREFIX.length))
  } catch {
    throw new Error("Room takeover reconnect probe is not valid JSON")
  }
  const expectedKeys = ["schema", "takeoverEventCount", ...BOOLEAN_FIELDS].sort()
  if (probe?.schema !== PROBE_SCHEMA) {
    throw new Error(`Room takeover reconnect probe schema must be ${PROBE_SCHEMA}`)
  }
  if (JSON.stringify(Object.keys(probe).sort()) !== JSON.stringify(expectedKeys)) {
    throw new Error("Room takeover reconnect probe fields do not match its schema")
  }
  for (const field of BOOLEAN_FIELDS) {
    if (probe[field] !== true) throw new Error(`Room takeover reconnect probe ${field} must be true`)
  }
  if (probe.takeoverEventCount !== 1) {
    throw new Error("Room takeover reconnect probe takeoverEventCount must be 1")
  }
  return probe
}
