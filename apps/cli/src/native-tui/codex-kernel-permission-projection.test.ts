import assert from "node:assert/strict"
import test from "node:test"
import { createCodexKernelOutputProjection } from "./codex-kernel-output-projection.js"
import { createCodexKernelPermissionProjection } from "./codex-kernel-permission-projection.js"
import type { LocalIpcClient } from "../ipc.js"
import type { RuntimeInteraction, RuntimeProviderRun } from "../cli-types.js"

function fixture(refuse?: "GetSessionState" | "RespondToInteraction") {
  const interaction = { id: "codex-command-approval-1", agent_id: "agent", kind: "permission", level: "warning", title: "Command approval required", message: "Read the requested public documentation with curl", choices: [{ id: "allow_once" }, { id: "allow_session" }, { id: "deny" }], requested_at_ms: 1000, native_origin: { scope: "prompt", provider_run_id: "run", prompt_id: "home" } } as RuntimeInteraction
  let refused = false
  const requests: any[] = []; const messages: any[] = []; let live = [interaction]
  const projection = createCodexKernelPermissionProjection({ sessionId: "session", agentId: "agent", bindState: { run: { id: "run" } as RuntimeProviderRun, promise: null, displayTurns: new Map([["display-turn", { threadId: "display-thread", promptId: "home" }]]) }, broadcast: m => messages.push(m), client: { send: async (request: any) => { requests.push(request); if (refuse && request[refuse] && !refused) { refused = true; throw new Error("retryable transport refusal") }; return request.GetSessionState ? { SessionState: { session: { active_interactions: live } } } : { InteractionResponded: {} } } } as unknown as LocalIpcClient })
  return { interaction, projection, requests, messages, setLive: (value: RuntimeInteraction[]) => { live = value } }
}

test("MP-08 MP-10 MP-11 native Codex projects the exact kernel approval once", async () => {
  const h = fixture(); h.projection.project([h.interaction]); h.projection.project([h.interaction])
  assert.equal(h.messages.length, 2)
  assert.equal(h.messages[1].method, "item/commandExecution/requestApproval")
  assert.equal(h.messages[1].params.threadId, "display-thread")
  assert.equal(h.messages[1].params.turnId, "display-turn")
  assert.match(h.messages[1].params.reason, /public documentation/)
  assert.equal(await h.projection.respond({ id: h.messages[1].id, result: { decision: "accept" } }), true)
  assert.deepEqual(h.requests.at(-1), { RespondToInteraction: { session_id: "session", interaction_id: h.interaction.id, choice_id: "allow_once", custom_reply: null } })
})

test("MP-11 native Codex approval refuses foreign or unbound origins", () => {
  for (const change of [{ agent_id: "foreign" }, { native_origin: { scope: "prompt", provider_run_id: "foreign", prompt_id: "home" } }, { native_origin: { scope: "prompt", provider_run_id: "run", prompt_id: "foreign" } }, { native_origin: { scope: "native_turn", provider_run_id: "run", native_turn_id: "provider-turn" } }, { native_origin: undefined }]) {
    const h = fixture(); h.projection.project([{ ...h.interaction, ...change } as RuntimeInteraction]); assert.deepEqual(h.messages, [])
  }
})

test("MP-11 native Codex rechecks an exact live approval before a response", async () => {
  for (const live of [[], [{ native_origin: { scope: "prompt", provider_run_id: "run", prompt_id: "changed" } }], [{ agent_id: "changed" }]]) {
    const h = fixture(); h.projection.project([h.interaction]); h.setLive(live.map(change => ({ ...h.interaction, ...change } as RuntimeInteraction)))
    assert.equal(await h.projection.respond({ id: h.messages[1].id, result: { decision: "acceptForSession" } }), true)
    assert.equal(h.requests.filter(r => r.RespondToInteraction).length, 0)
  }
})

test("MP-11 native Codex declines through the kernel and consumes stale projection IDs", async () => {
  const h = fixture(); h.projection.project([h.interaction]); await h.projection.respond({ id: h.messages[1].id, result: { decision: "decline" } })
  assert.equal(h.requests.at(-1).RespondToInteraction.choice_id, "deny")
  h.projection.project([]); assert.equal(await h.projection.respond({ id: h.messages[1].id, result: { decision: "accept" } }), true)
  assert.equal(await h.projection.respond({ id: "upstream-request", result: {} }), false)
})


test("MP-08 MP-10 pending approval closes its preamble stream without settling the home turn", () => {
  const messages: any[] = []
  const output = createCodexKernelOutputProjection({ agentId: "agent", broadcast: m => messages.push(m), debug: () => {}, nowMs: () => 1000 })
  output.setThreadId("thread")
  output.startPrompt("home")
  output.project([{ agent_id: "agent", prompt_id: "home", kind: "provider_output", bytes: [...Buffer.from("I will read the requested public documentation.")], timestamp_ms: 1000 } as any])
  const started = messages.find(m => m.method === "item/started" && m.params.item.type === "agentMessage")
  assert(started)
  output.finishPromptItemsForInteraction("foreign")
  assert.equal(messages.filter(m => m.method === "item/completed").length, 0)
  output.finishPromptItemsForInteraction("home")
  const completed = messages.find(m => m.method === "item/completed")
  assert.equal(completed.params.item.id, started.params.item.id)
  assert.equal(completed.params.item.phase, "commentary")
  assert.equal(messages.filter(m => m.method === "turn/completed").length, 0)
  output.project([{ agent_id: "agent", prompt_id: "home", kind: "provider_output", bytes: [...Buffer.from("The documentation was read after approval.")], timestamp_ms: 2000 } as any])
  const assistantStarts = messages.filter(m => m.method === "item/started" && m.params.item.type === "agentMessage")
  assert.equal(assistantStarts.length, 2)
  assert.notEqual(assistantStarts[1].params.item.id, started.params.item.id)
  assert.equal(messages.filter(m => m.method === "turn/started").length, 1)
})


test("MP-11 native Codex cancels an approval through kernel denial", async () => {
  const h = fixture(); h.projection.project([h.interaction]); await h.projection.respond({ id: h.messages[1].id, result: { decision: "cancel" } })
  assert.equal(h.requests.at(-1)?.RespondToInteraction?.choice_id, "deny")
  assert.equal(h.messages.at(-2)?.method, "serverRequest/resolved")
  assert.equal(h.messages.at(-1)?.params.item.status, "declined")
})

test("MP-11 native Codex dismisses a permission resolved by another client", () => {
  const h = fixture(); h.projection.project([h.interaction]); h.projection.project([]); h.projection.project([])
  assert.equal(h.messages.filter(m => m.method === "serverRequest/resolved").length, 1)
  assert.equal(h.messages.filter(m => m.method === "item/completed").length, 1)
})

test("MP-08 / MP-11 native permission replies retry after either kernel call fails", async () => {
  for (const refuse of ["GetSessionState", "RespondToInteraction"] as const) {
    const h = fixture(refuse); h.projection.project([h.interaction])
    const reply = {id:h.messages[1].id, result:{decision:"accept"}}
    await assert.rejects(h.projection.respond(reply), /retryable/)
    assert.equal(h.messages.some(m => m.method === "serverRequest/resolved"), false)
    await h.projection.respond(reply)
    assert.equal(h.requests.at(-1).RespondToInteraction.choice_id, "allow_once")
    assert.equal(h.messages.filter(m => m.method === "serverRequest/resolved").length, 1)
  }
})
