export const QUEUE_SATURATION_FAULT_CASE_IDS = Object.freeze([
  "fault.queue-saturation",
  "cleanup.resources",
])

export const QUEUE_SATURATION_FAULT_TEST_NAME =
  "server::connection::tests::queue_saturation_fault_probe"

const PROBE_SCHEMA = "chariox.queue_saturation_fault_probe.v1"
const PROBE_FIELDS = Object.freeze([
  "queueLimitReachedDeterministically",
  "clientRequestRejectedRetryably",
  "peerRequestRejectedRetryably",
  "slowSubscriberIsolated",
  "healthyReaderPreserved",
  "readerLaneRemainedLive",
  "backpressureMetricsRecorded",
])

export function buildQueueSaturationFaultCargoArgs() {
  return [
    "test",
    "-p",
    "chariox-relay",
    "--lib",
    QUEUE_SATURATION_FAULT_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ]
}

export function parseQueueSaturationFaultProbe(output) {
  const candidates = String(output ?? "").split("\n").map((line) => line.trim()).filter(Boolean)
  let probe = null
  for (const candidate of candidates) {
    if (!candidate.startsWith("{")) continue
    try {
      const parsed = JSON.parse(candidate)
      if (parsed?.schema === PROBE_SCHEMA) probe = parsed
    } catch {
      // Cargo output is mixed with the schema-tagged probe.
    }
  }
  if (!probe) throw new Error(`queue saturation output is missing ${PROBE_SCHEMA}`)
  const expectedKeys = ["schema", ...PROBE_FIELDS].sort()
  if (JSON.stringify(Object.keys(probe).sort()) !== JSON.stringify(expectedKeys)) {
    throw new Error("queue saturation probe fields do not match its schema")
  }
  for (const field of PROBE_FIELDS) {
    if (probe[field] !== true) throw new Error(`queue saturation probe ${field} must be true`)
  }
  return probe
}
