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
const EVIDENCE_FIELDS = Object.freeze([
  "runningActionId", "queuedActionId", "freshActionId", "runningError", "queuedError",
  "clickCountBeforeFault", "clickCountAfterFault", "clickCountAfterFreshClick",
  "runningControllerRequest", "freshControllerRequest", "faultBoundaryControllerRequests",
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
  const expectedKeys = ["schema", ...PROBE_FIELDS, ...EVIDENCE_FIELDS].sort()
  const actualKeys = Object.keys(probe).sort()
  if (JSON.stringify(actualKeys) !== JSON.stringify(expectedKeys)) {
    throw new Error("controller fault probe fields do not match its schema")
  }
  for (const field of PROBE_FIELDS) {
    if (probe[field] !== true) throw new Error(`controller fault probe ${field} must be true`)
  }
  for (const field of ["runningActionId", "queuedActionId", "freshActionId", "runningError", "queuedError"]) {
    if (typeof probe[field] !== "string" || !probe[field].trim()) {
      throw new Error(`controller fault probe ${field} must be non-empty`)
    }
  }
  if (new Set([probe.runningActionId, probe.queuedActionId, probe.freshActionId]).size !== 3) {
    throw new Error("controller fault probe action identities must be distinct")
  }
  if (![probe.clickCountBeforeFault, probe.clickCountAfterFault, probe.clickCountAfterFreshClick]
    .every((value) => Number.isSafeInteger(value) && value >= 0)
    || probe.clickCountAfterFault !== probe.clickCountBeforeFault
    || probe.clickCountAfterFreshClick !== probe.clickCountAfterFault + 1) {
    throw new Error("controller fault probe click counts must prove no replay and one fresh mutation")
  }
  for (const field of ["runningControllerRequest", "freshControllerRequest"]) {
    const request = probe[field]
    if (request?.event !== "controller_request_entered" || request.method !== "browser.action"
      || typeof request.request_ref !== "string" || !request.request_ref) {
      throw new Error(`controller fault probe ${field} must identify a browser action request`)
    }
  }
  if (probe.runningControllerRequest.request_ref === probe.freshControllerRequest.request_ref
    || !Array.isArray(probe.faultBoundaryControllerRequests)
    || probe.faultBoundaryControllerRequests.filter((event) =>
      event?.request_ref === probe.runningControllerRequest.request_ref
      && event.event === "controller_request_entered").length !== 1
    || probe.faultBoundaryControllerRequests.some((event) =>
      event?.request_ref === probe.runningControllerRequest.request_ref
      && event.event === "controller_request_completed")) {
    throw new Error("controller fault probe running controller request must enter once without completing before the fault")
  }
  return probe
}
