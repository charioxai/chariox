import assert from "node:assert/strict"
import test from "node:test"

import {
  fatalProviderOutput,
  terminalProviderOutputSnapshot,
} from "./remote-machine-runtime-output.mjs"

const record = (kind, text, agentId = "agent-1") => ({
  kind,
  agent_id: agentId,
  bytes: [...Buffer.from(text)],
})

test("terminal provider output excludes prompt echoes and joins fragmented output", () => {
  const snapshot = terminalProviderOutputSnapshot([
    { event: "terminal_output", records: [record("prompt_echo", "REMOTE_MACHINE_OK")] },
    { event: "terminal_output", records: [record("provider_output", "REMOTE_")] },
    { event: "terminal_output", records: [record("ProviderOutput", "MACHINE_OK")] },
  ], "agent-1")

  assert.equal(snapshot.providerText, "REMOTE_MACHINE_OK")
  assert.equal(snapshot.recordCount, 3)
  assert.equal(fatalProviderOutput(snapshot), null)
})

test("terminal provider output ignores records for another agent", () => {
  const snapshot = terminalProviderOutputSnapshot([
    { event: "terminal_output", records: [
      record("provider_output", "WRONG", "agent-2"),
      record("provider_output", "RIGHT", "agent-1"),
    ] },
  ], "agent-1")

  assert.equal(snapshot.providerText, "RIGHT")
  assert.equal(snapshot.recordCount, 1)
})

test("fatal provider output reports error and failed status records", () => {
  const errorSnapshot = terminalProviderOutputSnapshot([
    { event: "terminal_output", records: [record("provider_error", "credential expired")] },
  ])
  const statusSnapshot = terminalProviderOutputSnapshot([
    { event: "terminal_output", records: [record("provider_status", "thread/status=systemError")] },
  ])

  assert.equal(fatalProviderOutput(errorSnapshot), "credential expired")
  assert.equal(fatalProviderOutput(statusSnapshot), "thread/status=systemError")
})
