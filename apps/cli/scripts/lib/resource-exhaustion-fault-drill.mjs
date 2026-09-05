export const RESOURCE_EXHAUSTION_CASE_IDS = Object.freeze([
  "fault.resource-exhaustion",
  "cleanup.resources",
])

const PROBE_SCHEMA = "chariox.resource_exhaustion_probe.v1"
const EXPECTED_MODES = Object.freeze(["file-descriptor", "process"])
const EXPECTED_CODES = Object.freeze({
  "file-descriptor": new Set(["EMFILE", "ENFILE"]),
  process: new Set(["EAGAIN"]),
})

export function parseResourceExhaustionProbes(outputs) {
  if (!Array.isArray(outputs) || outputs.length !== EXPECTED_MODES.length) {
    throw new Error("resource exhaustion drill must return one file-descriptor and one process probe")
  }
  const probes = new Map()
  for (const output of outputs) {
    let probe
    try {
      probe = JSON.parse(String(output).trim().split("\n").at(-1))
    } catch {
      throw new Error("resource exhaustion probe is not valid JSON")
    }
    const expectedKeys = ["cleanupComplete", "errorCode", "exhausted", "mode", "schema", "terminalLaneLive"]
    if (probe?.schema !== PROBE_SCHEMA || JSON.stringify(Object.keys(probe).sort()) !== JSON.stringify(expectedKeys)) {
      throw new Error("resource exhaustion probe fields do not match its schema")
    }
    if (!EXPECTED_MODES.includes(probe.mode) || probes.has(probe.mode)) {
      throw new Error(`resource exhaustion probe mode is invalid: ${probe.mode}`)
    }
    if (probe.exhausted !== true || !EXPECTED_CODES[probe.mode].has(probe.errorCode)) {
      throw new Error(`${probe.mode} exhaustion did not emit its actionable operating-system diagnostic`)
    }
    if (probe.terminalLaneLive !== true) {
      throw new Error(`${probe.mode} exhaustion did not preserve the terminal lane`)
    }
    if (probe.cleanupComplete !== true) {
      throw new Error(`${probe.mode} exhaustion did not clean up its owned resources`)
    }
    probes.set(probe.mode, probe)
  }
  for (const mode of EXPECTED_MODES) {
    if (!probes.has(mode)) throw new Error(`resource exhaustion drill is missing the ${mode} probe`)
  }
  return {
    schema: "chariox.resource_exhaustion_fault_result.v1",
    fileDescriptorLimitEnforced: true,
    processLimitEnforced: true,
    terminalLaneLive: true,
    actionableDiagnostics: Object.fromEntries(EXPECTED_MODES.map((mode) => [mode, probes.get(mode).errorCode])),
    cleanupComplete: true,
  }
}
