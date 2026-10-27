import assert from "node:assert/strict"
import test from "node:test"
import type { RuntimeProviderRun } from "../cli-types.js"
import type { LocalIpcClient } from "../ipc.js"
import { bindCodexDisplayTurn, handleCodexNativeTurnSteer, mapCodexNativeCompaction } from "./codex-conversation-controls.js"
import { createCodexKernelOutputProjection } from "./codex-kernel-output-projection.js"
import { extractCodexSteeringPrompt } from "./codex-prompt.js"

const boundRun = {
  id: "run", session_id: "session", agent_instance_id: "agent", provider: "codex",
  state: "Running", structured_endpoint: "ws://fixture", provider_session_id: "old-thread",
} as RuntimeProviderRun
const message = { id: "compact", method: "thread/compact/start", params: { threadId: "display-thread" } }

test("MP-08 MP-10 steering receipts preserve text parts and intentional trailing whitespace", () => {
  assert.equal(extractCodexSteeringPrompt({ input: [
    { type: "text", text: "compare " }, { type: "text", text: "both sources\n\n" },
    { type: "localImage", path: "/workspace/source.png" },
  ] }), "compare both sources\n\n")
})

test("MP-08 MP-10 projected output turn steers its exact home prompt", async () => {
  const requests: unknown[] = []
  const opts = { ...options(async (request) => {
    requests.push(request)
    return { ActivePromptSteered: { prompt: { id: "steer" } } }
  }), attachmentId: "attachment", inlineLocalAttachments: false }
  const turns: { id: string, method?: string, params?: { turn: { id: string } } }[] = []
  const projection = createCodexKernelOutputProjection({
    agentId: "agent", broadcast: (m) => turns.push(m as typeof turns[number]), debug: () => {},
    onTurnMapped: (thread, turn, prompt) => bindCodexDisplayTurn(opts.bindState, thread, turn, prompt),
  })
  projection.setThreadId("display-thread")
  projection.project([{ agent_id: "agent", prompt_id: "projected-home-prompt", kind: "provider_output", bytes: [...Buffer.from("progress")], timestamp_ms: 1000 }])
  const turnId = turns.find(m => m.method === "turn/started")!.params!.turn.id
  const responses: unknown[] = []
  await handleCodexNativeTurnSteer({ id: "steer", method: "turn/steer", params: {
    threadId: "display-thread", expectedTurnId: turnId, input: [{ type: "text", text: "new direction" }],
  } }, opts, (m) => responses.push(m))
  assert.deepEqual(requests, [{ SteerActivePrompt: {
    session_id: "session", attachment_id: "attachment", target_agent_id: "agent",
    expected_active_prompt_id: "projected-home-prompt", prompt: "new direction", attachments: [],
  } }])
  assert.deepEqual(responses.filter((value: any) => value.id !== undefined), [{ id: "steer", result: { turnId } }])
})

test("MP-08 MP-10 admitted steering returns its committed receipt to the requesting display", async () => {
  const opts = { ...options(async () => ({ ActivePromptSteered: { prompt: { id: "steer-prompt" } } })),
    attachmentId: "attachment", inlineLocalAttachments: false }
  bindCodexDisplayTurn(opts.bindState, "display-thread", "display-turn", "home-prompt")
  const input = [{ type: "text", text: "new direction" }]
  const responses: any[] = []
  await handleCodexNativeTurnSteer({ id: 1, method: "turn/steer", params: {
    threadId: "display-thread", expectedTurnId: "display-turn", clientUserMessageId: "native-input", input,
  } }, opts, value => responses.push(value))
  const receipt = responses.find(value => value.method === "item/completed")
  assert(receipt, "initiating native display needs a committed steering receipt")
  assert.equal(receipt.params.turnId, "display-turn")
  assert.equal(receipt.params.threadId, "display-thread")
  // MP-08/MP-10/MP-11: the official typed client drops either lifecycle
  // notification without its required timestamp, leaving the steer pending.
  const started = responses.find(value => value.method === "item/started")
  assert(Number.isFinite(started?.params.startedAtMs))
  assert(Number.isFinite(receipt.params.completedAtMs))
  assert(started.params.startedAtMs <= receipt.params.completedAtMs)
  assert.deepEqual(receipt.params.item, { type: "userMessage", id: "chariox-steering-steer-prompt", clientId: "native-input", content: input })
  assert.deepEqual(responses.filter(value => value.id !== undefined), [{ id: 1, result: { turnId: "display-turn" } }])
})

test("MP-08 MP-10 steering preserves kernel rejection without starting or queuing a fallback", async () => {
  let calls = 0
  const opts = { ...options(async () => { calls++; throw new Error("active prompt changed before steering") }),
    attachmentId: "attachment", inlineLocalAttachments: false }
  bindCodexDisplayTurn(opts.bindState, "display-thread", "display-turn", "old-home-prompt")
  const responses: unknown[] = []
  await handleCodexNativeTurnSteer({ id: 1, method: "turn/steer", params: {
    threadId: "display-thread", expectedTurnId: "display-turn", input: [{ type: "text", text: "late input" }],
  } }, opts, (m) => responses.push(m))
  assert.equal(calls, 1)
  assert.deepEqual(responses, [{ id: 1, error: { code: -32000, message: "active prompt changed before steering" } }])
})
function options(send: (request: unknown) => Promise<unknown>) {
  return {
    client: { send } as LocalIpcClient, sessionId: "session", agentId: "agent",
    bindState: { run: boundRun, promise: Promise.resolve(boundRun) },
  }
}

test("MP-08 MP-10 native compaction refreshes the managed conversation for every request", async () => {
  let calls = 0
  const opts = options(async (request) => {
    assert.deepEqual(request, { GetProviderRun: { provider_run_id: "run" } })
    return { ProviderRun: { provider_run: { ...boundRun, provider_session_id: `thread-${++calls}` } } }
  })
  for (const threadId of ["thread-1", "thread-2"]) {
    assert.deepEqual(await mapCodexNativeCompaction(message, opts), { ...message, params: { threadId } })
  }
  assert.equal(opts.bindState.run.provider_session_id, "old-thread", "launch metadata must not become an identity cache")
})

test("MP-08 MP-10 native compaction rejects ended or foreign run projections", async () => {
  for (const change of [
    { id: "replacement" }, { session_id: "other" }, { agent_instance_id: "other" },
    { provider: "other" }, { state: "Ended" }, { structured_endpoint: "ws://other" },
  ]) {
    await assert.rejects(mapCodexNativeCompaction(message, options(async () => ({ ProviderRun: { provider_run: { ...boundRun, ...change } } }))),
      /changed or ended/)
  }
})

test("MP-08 MP-10 native compaction waits for asynchronous first-turn identity publication", async () => {
  let calls = 0
  const mapped = await mapCodexNativeCompaction(message, options(async () => ({ ProviderRun: { provider_run: {
    ...boundRun, provider_session_id: ++calls === 1 ? null : "managed-thread",
  } } })), 500)
  assert.equal(mapped.params?.threadId, "managed-thread")
  assert.equal(calls, 2)
})

test("MP-08 MP-10 native compaction bounds missing identity and stalled kernel reads", async () => {
  for (const send of [
    async () => ({ ProviderRun: { provider_run: { ...boundRun, provider_session_id: null } } }),
    async () => new Promise(() => {}),
  ]) await assert.rejects(mapCodexNativeCompaction(message, options(send), 20), /timed out.*managed Codex conversation/)
})


test("MP-08 MP-10 native output without a prompt id keeps the admitted home turn", () => {
  const messages: any[] = []
  const mapped: string[] = []
  const projection = createCodexKernelOutputProjection({ agentId: "agent", broadcast: message => messages.push(message), debug: () => {}, onTurnMapped: (_thread, turn) => mapped.push(turn) })
  projection.setThreadId("display-thread")
  projection.startPrompt("home-prompt")
  const progress = { agent_id: "agent", prompt_origin: "chariox" as const, kind: "provider_output" as const, bytes: [...Buffer.from("real provider progress")], timestamp_ms: 1000 }
  projection.project([progress])
  assert.deepEqual(messages.filter(m => m.method === "turn/started").map(m => m.params.turn.id), ["chariox-native-home-prompt"])
  assert.deepEqual(mapped, ["chariox-native-home-prompt"])
  assert(messages.some(m => m.method === "item/agentMessage/delta" && m.params.turnId === "chariox-native-home-prompt"))
  projection.projectAgentActivity({ last_completed_turn: { agent_id: "agent", prompt_id: "home-prompt", settlement_status: "completed", completed_at_ms: 2000 } } as any)
  projection.project([progress])
  assert.equal(messages.filter(m => m.method === "turn/started").length, 1, "late unbound output cannot reopen a settled display turn")
})
