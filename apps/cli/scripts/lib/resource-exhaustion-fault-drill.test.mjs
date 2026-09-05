import assert from "node:assert/strict"
import test from "node:test"

import {
  RESOURCE_EXHAUSTION_CASE_IDS,
  boundedEvidenceText,
  parseResourceExhaustionProbes,
} from "./resource-exhaustion-fault-drill.mjs"

test("resource exhaustion drill declares the bounded failure and cleanup cases", () => {
  assert.deepEqual(RESOURCE_EXHAUSTION_CASE_IDS, [
    "fault.resource-exhaustion",
    "cleanup.resources",
  ])
})

test("resource evidence removes controls without corrupting diagnostics", () => {
  assert.equal(
    boundedEvidenceText("resource exhaustion failed: spawn EAGAIN\u0000\u0007\nfree memory 68%"),
    "resource exhaustion failed: spawn EAGAIN\nfree memory 68%",
  )
  assert.equal(boundedEvidenceText("prefix-useful-tail", 11), "useful-tail")
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
