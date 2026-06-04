import assert from "node:assert/strict"
import test from "node:test"

import { createCodexKernelOutputProjection } from "./native-tui/codex-kernel-output-projection.js"

test("codex kernel output projection ignores unscoped and wrong-agent records", () => {
  const broadcasts: unknown[] = []
  const debug: unknown[] = []
  const projection = createCodexKernelOutputProjection({
    agentId: "agent-1",
    broadcast: (message) => broadcasts.push(message),
    debug: (label, payload) => debug.push({ label, payload }),
  })
  projection.setThreadId("thread-1")

  projection.project([
    {
      agent_id: null,
      kind: "provider_output",
      bytes: [...Buffer.from("unscoped", "utf8")],
    },
    {
      agent_id: "agent-2",
      kind: "provider_output",
      bytes: [...Buffer.from("wrong", "utf8")],
    },
  ])

  assert.deepEqual(broadcasts, [])
  assert.deepEqual(debug, [])
})

test("codex kernel output projection broadcasts matching agent records", () => {
  const broadcasts: unknown[] = []
  const projection = createCodexKernelOutputProjection({
    agentId: "agent-1",
    broadcast: (message) => broadcasts.push(message),
    debug: () => {},
  })
  projection.setThreadId("thread-1")

  projection.project([{
    agent_id: "agent-1",
    kind: "provider_output",
    bytes: [...Buffer.from("hello", "utf8")],
  }])

  assert.equal(broadcasts.some((message) => JSON.stringify(message).includes("item/agentMessage/delta")), true)
})
