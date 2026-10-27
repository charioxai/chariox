import assert from "node:assert/strict"
import test from "node:test"

import { resolveTerminalRecordAgentId } from "./terminal-record-agent-resolver.js"

test("resolveTerminalRecordAgentId prefers explicit record agent ids", () => {
  assert.equal(resolveTerminalRecordAgentId({
    record: { agent_id: "record-agent" },
  }), "record-agent")
})

test("resolveTerminalRecordAgentId does not infer unscoped record ownership", () => {
  assert.equal(resolveTerminalRecordAgentId({
    record: { agent_id: null },
  }), null)
})
