import assert from "node:assert/strict"
import test from "node:test"

import { createCodexKernelOutputProjection } from "./native-tui/codex-kernel-output-projection.js"
import {
  EXTERNAL_PROVIDER_OBSERVED_SOURCE,
} from "@chariox/kernel-client/external-provider-observation"

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
      timestamp_ms: 1_000,
      kind: "provider_output",
      bytes: [...Buffer.from("unscoped", "utf8")],
    },
    {
      agent_id: "agent-2",
      timestamp_ms: 1_001,
      kind: "provider_output",
      bytes: [...Buffer.from("wrong", "utf8")],
    },
  ])

  assert.deepEqual(broadcasts, [])
  assert.deepEqual(debugLabels(debug, "projection_record_skipped").map((entry) => entry.payload.reason), [
    "agent_mismatch",
    "agent_mismatch",
  ])
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
    timestamp_ms: 1_700_000_000_000,
    kind: "provider_output",
    bytes: [...Buffer.from("hello", "utf8")],
  }])

  assert.equal(broadcasts.some((message) => JSON.stringify(message).includes("item/agentMessage/delta")), true)
})

test("MP-08 native output completes on kernel settlement rather than a gap between deltas", (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] })
  const broadcasts: unknown[] = []
  const projection = createCodexKernelOutputProjection({ agentId: "agent-1", broadcast: (message) => broadcasts.push(message), debug: () => {} })
  projection.setThreadId("thread-1")
  projection.project([{ agent_id: "agent-1", timestamp_ms: 1_000, kind: "provider_output", bytes: [...Buffer.from("AC")] }])
  context.mock.timers.tick(1_000)
  assert.equal(broadcasts.filter((message) => messageMethod(message) === "item/completed").length, 0)
  projection.project([{ agent_id: "agent-1", timestamp_ms: 2_000, kind: "provider_output", bytes: [...Buffer.from("CLD-NATIVE-OK")] }])
  projection.projectSessionState({ agent_activity: { "agent-1": { busy: false, last_completed_turn: { prompt_id: "prompt-1", completed_at_ms: 3_000 } } } } as unknown as import("./cli-types.js").RuntimeSession)
  const completed = broadcasts.filter((message) => messageMethod(message) === "item/completed") as Array<{ params: { item: { text: string } } }>
  assert.equal(completed.length, 1)
  assert.equal(completed[0]?.params.item.text, "ACCLD-NATIVE-OK")
})

test("MP-08 kernel settlement cannot complete output from a newer prompt", () => {
  const broadcasts: unknown[] = []
  const projection = createCodexKernelOutputProjection({ agentId: "agent-1", broadcast: (message) => broadcasts.push(message), debug: () => {} })
  projection.setThreadId("thread-1")
  projection.project([{ agent_id: "agent-1", timestamp_ms: 4_000, kind: "provider_output", bytes: [...Buffer.from("new turn")] }])
  const session = { agent_activity: { "agent-1": { busy: false, last_completed_turn: { prompt_id: "old", completed_at_ms: 3_000 } } } } as unknown as import("./cli-types.js").RuntimeSession
  projection.projectSessionState(session)
  assert.equal(broadcasts.some((message) => messageMethod(message) === "turn/completed"), false)
  projection.project([{ agent_id: "agent-1", prompt_id: "new", timestamp_ms: 2_000, kind: "provider_output", bytes: [...Buffer.from("scoped new turn")] }])
  projection.projectSessionState(session)
  assert.equal(broadcasts.some((message) => messageMethod(message) === "turn/completed"), false)
})

test("codex kernel output projection suppresses passive external telemetry", () => {
  const broadcasts: unknown[] = []
  const projection = createCodexKernelOutputProjection({
    agentId: "agent-1",
    broadcast: (message) => broadcasts.push(message),
    debug: () => {},
  })
  projection.setThreadId("thread-1")

  projection.project([{
    agent_id: "agent-1",
    timestamp_ms: 1_700_000_000_001,
    kind: "provider_status",
    source: EXTERNAL_PROVIDER_OBSERVED_SOURCE,
    external_provider: "codex",
    external_provider_session_id: "thread-1",
    external_provider_turn_id: "token-count",
    external_observation: {
      settles_active_prompt: false,
      passive_telemetry: true,
    },
    bytes: [...Buffer.from("codex token_count", "utf8")],
  }])

  assert.deepEqual(broadcasts, [])
})

test("codex kernel output projection follows shared live append suppression", () => {
  const broadcasts: unknown[] = []
  const projection = createCodexKernelOutputProjection({
    agentId: "agent-1",
    broadcast: (message) => broadcasts.push(message),
    debug: () => {},
  })
  projection.setThreadId("thread-1")

  projection.project([{
    agent_id: "agent-1",
    timestamp_ms: 1_700_000_000_002,
    kind: "provider_status",
    bytes: [...Buffer.from("OpenCode is idle.", "utf8")],
  }])

  assert.deepEqual(broadcasts, [])
})

test("codex kernel output projection normalizes provider errors through shared terminal projection", () => {
  const broadcasts: unknown[] = []
  const projection = createCodexKernelOutputProjection({
    agentId: "agent-1",
    broadcast: (message) => broadcasts.push(message),
    debug: () => {},
  })
  projection.setThreadId("thread-1")

  projection.project([{
    agent_id: "agent-1",
    timestamp_ms: 1_700_000_000_003,
    kind: "provider_error",
    bytes: [...Buffer.from("failed\r\n", "utf8")],
  }])

  assert.equal(broadcasts.some((message) => JSON.stringify(message).includes("item/agentMessage/delta")), true)
  assert.equal(broadcasts.some((message) => JSON.stringify(message).includes("failed\\r\\n")), false)
  assert.equal(broadcasts.some((message) => JSON.stringify(message).includes("failed")), true)
})

test("codex kernel output projection uses kernel record timestamps for user item lifecycle", () => {
  const broadcasts: unknown[] = []
  const projection = createCodexKernelOutputProjection({
    agentId: "agent-1",
    broadcast: (message) => broadcasts.push(message),
    debug: () => {},
    nowMs: () => 9_999_999,
  })
  projection.setThreadId("thread-1")

  projection.project([{
    agent_id: "agent-1",
    timestamp_ms: 1_700_000_123_456,
    kind: "prompt_echo",
    bytes: [...Buffer.from("hello", "utf8")],
  }])

  assert.equal(findMethodParam(broadcasts, "item/started", "startedAtMs"), 1_700_000_123_456)
  assert.equal(findMethodParam(broadcasts, "item/completed", "completedAtMs"), 1_700_000_123_456)
  const turnStarted = broadcasts.find((message) => messageMethod(message) === "turn/started") as { params?: { turn?: { startedAt?: number } } } | undefined
  assert.equal(turnStarted?.params?.turn?.startedAt, 1_700_000_123)
})

function messageMethod(message: unknown): string | undefined {
  return typeof message === "object" && message !== null && "method" in message
    ? String((message as { method?: unknown }).method)
    : undefined
}

function findMethodParam(messages: readonly unknown[], method: string, param: string): unknown {
  const message = messages.find((candidate) => messageMethod(candidate) === method)
  return (message as { params?: Record<string, unknown> } | undefined)?.params?.[param]
}

function debugLabels(entries: readonly unknown[], label: string) {
  return entries.filter((entry): entry is { label: string, payload: { reason?: string } } =>
    typeof entry === "object"
    && entry !== null
    && "label" in entry
    && (entry as { label?: unknown }).label === label
    && "payload" in entry
    && typeof (entry as { payload?: unknown }).payload === "object"
    && (entry as { payload?: unknown }).payload !== null
  )
}
