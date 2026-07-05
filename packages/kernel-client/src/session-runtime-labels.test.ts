import assert from "node:assert/strict"
import test from "node:test"

import { formatSessionHomeKernelLabel } from "./session-runtime-labels.js"

test("session home kernel label keeps kernel and machine identity visible", () => {
  assert.equal(formatSessionHomeKernelLabel({
    host_daemon_id: "home-kernel",
    host_machine_id: "home-machine",
  }), "home-kernel@home-machine")
  assert.equal(formatSessionHomeKernelLabel({
    kernel_id: "legacy-kernel",
    host_machine_id: "home-machine",
  }), "legacy-kernel@home-machine")
  assert.equal(formatSessionHomeKernelLabel({
    homeKernelId: "context-kernel",
    homeMachineId: "context-machine",
  }), "context-kernel@context-machine")
  assert.equal(formatSessionHomeKernelLabel({
    host_machine_id: "home-machine",
  }), "home-machine")
  assert.equal(formatSessionHomeKernelLabel(null), "-")
  assert.equal(formatSessionHomeKernelLabel({}, "unknown"), "unknown")
})
