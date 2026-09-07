export const BROWSER_CONTROLLER_FAULT_CASE_IDS = Object.freeze([
  "fault.controller-crash",
  "fault.controller-crash-during-queued-mutations",
  "cleanup.resources",
])

export const BROWSER_CONTROLLER_FAULT_TEST_NAME =
  "runtime::router::tests::room_environment_placement::live_worker::controller::room_environment_controller_uses_its_slice_without_worker_agents"

const PROBE_SCHEMA = "chariox.browser_controller_fault_probe.v2"
const PROBE_FIELDS = Object.freeze([
  "faultTriggered",
  "processLostAttributed",
  "staleReferenceRejected",
  "processReplaced",
  "tabsPreserved",
  "authorityPreserved",
  "postRecoveryActionExactlyOnce",
  "runningMutationNotRepeated",
  "queuedMutationSettled",
  "freshMutationExactlyOnce",
])

export function buildBrowserControllerFaultCargoArgs() {
  return [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    BROWSER_CONTROLLER_FAULT_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ]
}

export function parseBrowserControllerFaultProbe(output) {
  const candidates = String(output ?? "").split("\n").map((line) => line.trim()).filter(Boolean)
  let probe = null
  for (const candidate of candidates) {
    if (!candidate.startsWith("{")) continue
    try {
      const parsed = JSON.parse(candidate)
      if (parsed?.schema === PROBE_SCHEMA) probe = parsed
    } catch {
      // Cargo output contains non-JSON lines; only the schema-tagged probe matters.
    }
  }
  if (!probe) throw new Error(`controller fault output is missing ${PROBE_SCHEMA}`)
  const expectedKeys = ["schema", ...PROBE_FIELDS].sort()
  const actualKeys = Object.keys(probe).sort()
  if (JSON.stringify(actualKeys) !== JSON.stringify(expectedKeys)) {
    throw new Error("controller fault probe fields do not match its schema")
  }
  for (const field of PROBE_FIELDS) {
    if (probe[field] !== true) throw new Error(`controller fault probe ${field} must be true`)
  }
  return probe
}
