import assert from "node:assert/strict"
import test from "node:test"

import {
  BROWSER_CONTROLLER_FAULT_CASE_IDS,
  BROWSER_CONTROLLER_FAULT_TEST_NAME,
  buildBrowserControllerFaultCargoArgs,
  parseBrowserControllerFaultProbe,
} from "./browser-controller-fault-drill.mjs"

test("controller fault drill runs only the exact kernel library test", () => {
  assert.deepEqual(buildBrowserControllerFaultCargoArgs(), [
    "test",
    "-p",
    "chariox-kernel",
    "--lib",
    BROWSER_CONTROLLER_FAULT_TEST_NAME,
    "--",
    "--exact",
    "--nocapture",
  ])
  assert.deepEqual(BROWSER_CONTROLLER_FAULT_CASE_IDS, [
    "fault.controller-crash",
    "fault.controller-crash-during-queued-mutations",
    "cleanup.resources",
  ])
})

test("controller fault drill requires every crash-recovery invariant", () => {
  const probe = {
    schema: "chariox.browser_controller_fault_probe.v2",
    faultTriggered: true,
    processLostAttributed: true,
    staleReferenceRejected: true,
    processReplaced: true,
    tabsPreserved: true,
    authorityPreserved: true,
    postRecoveryActionExactlyOnce: true,
    runningMutationNotRepeated: true,
    queuedMutationSettled: true,
    freshMutationExactlyOnce: true,
  }
  assert.deepEqual(parseBrowserControllerFaultProbe(`noise\n${JSON.stringify(probe)}\n`), probe)

  const invalid = { ...probe, tabsPreserved: false }
  assert.throws(
    () => parseBrowserControllerFaultProbe(JSON.stringify(invalid)),
    /tabsPreserved must be true/,
  )
})
