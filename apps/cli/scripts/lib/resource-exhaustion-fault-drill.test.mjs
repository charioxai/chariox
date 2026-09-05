import assert from "node:assert/strict"
import test from "node:test"

import {
  RESOURCE_EXHAUSTION_CASE_IDS,
  parseResourceExhaustionProbes,
} from "./resource-exhaustion-fault-drill.mjs"

test("resource exhaustion drill declares the bounded failure and cleanup cases", () => {
  assert.deepEqual(RESOURCE_EXHAUSTION_CASE_IDS, [
    "fault.resource-exhaustion",
    "cleanup.resources",
  ])
})

test("resource exhaustion probes require both bounded failures and a live terminal lane", () => {
  const parsed = parseResourceExhaustionProbes([
    JSON.stringify({ schema: "chariox.resource_exhaustion_probe.v1", mode: "file-descriptor", exhausted: true, errorCode: "EMFILE", terminalLaneLive: true, cleanupComplete: true }),
    JSON.stringify({ schema: "chariox.resource_exhaustion_probe.v1", mode: "process", exhausted: true, errorCode: "EAGAIN", terminalLaneLive: true, cleanupComplete: true }),
  ])
  assert.equal(parsed.fileDescriptorLimitEnforced, true)
  assert.equal(parsed.processLimitEnforced, true)
  assert.equal(parsed.terminalLaneLive, true)

  assert.throws(
    () => parseResourceExhaustionProbes([
      JSON.stringify({ schema: "chariox.resource_exhaustion_probe.v1", mode: "file-descriptor", exhausted: true, errorCode: "EMFILE", terminalLaneLive: false, cleanupComplete: true }),
      JSON.stringify({ schema: "chariox.resource_exhaustion_probe.v1", mode: "process", exhausted: true, errorCode: "EAGAIN", terminalLaneLive: true, cleanupComplete: true }),
    ]),
    /terminal lane/i,
  )
})
