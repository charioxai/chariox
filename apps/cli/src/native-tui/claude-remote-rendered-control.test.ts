// MP-08/MP-10/MP-11: interrupt an admitted prompt without inventing settlement.
import assert from "node:assert/strict"
import test from "node:test"
import type { LocalIpcClient } from "../ipc.js"
import { sendClaudeRemoteRenderedInput } from "./claude-remote-rendered-control.js"
test("bare Escape cancels the exact rendered Claude agent through normal kernel ownership", async () => {
  const requests: any[] = []
  const client = { send: async (r: any) => {requests.push(r);return r.GetSessionState
    ? {SessionState: {agent_activity: {agent: {active_prompt_count: 1}}, session: {prompt_states: {agent: {active_prompt: {id: "home-prompt"}}}}}}
    : {PromptCancelled: {cancellation: {prompt: {id: "home-prompt"}}}}} } as unknown as LocalIpcClient
  await sendClaudeRemoteRenderedInput(client, "session", "attachment", "run", "\u001b", "agent")
  assert.deepEqual(requests.at(-1), {CancelActivePrompt: {session_id: "session", attachment_id: "attachment", target_agent_id: "agent", expected_prompt_id: "home-prompt"}})
  assert(!requests.some(r => r.SendTerminalInput), "kernel abort owns provider interruption")
})

for (const [label, data, activities] of [
  ["idle Escape", "\u001b", {agent: {active_prompt_count: 0}}],
  ["another agent working", "\u001b", {other: {active_prompt_count: 1}}],
  ["navigation escape sequence", "\u001b[A", {agent: {active_prompt_count: 1}}],
] as const) test(label + " remains provider terminal input", async () => {
  const requests: any[] = []
  const client = { send: async (r: any) => {requests.push(r); return {SessionState: {agent_activity: activities}}} } as unknown as LocalIpcClient
  await sendClaudeRemoteRenderedInput(client, "session", "attachment", "run", data, "agent")
  assert(requests.at(-1).SendTerminalInput)
  assert(!requests.some(r => r.CancelActivePrompt))
})
test("a rejected home interruption cannot become untracked provider cancellation", async () => {
  const requests: any[] = []
  const client = {send: async (r: any) => {requests.push(r);return r.GetSessionState
    ? {SessionState: {agent_activity: {agent: {active_prompt_count: 1}}, session: {prompt_states: {agent: {active_prompt: {id: "home-prompt"}}}}}} : {Error: {message: "not admitted"}}}} as unknown as LocalIpcClient
  await assert.rejects(sendClaudeRemoteRenderedInput(client, "session", "attachment", "run", "\u001b", "agent"), /not admitted/)
  assert(!requests.some(r => r.SendTerminalInput))
})

test("a delayed rendered Escape carries the observed prompt identity and leaves its successor alone", async () => {
  const requests: any[] = []
  let active = "prompt-A"
  const client = { send: async (request: any) => {
    requests.push(request)
    if (request.GetSessionState) {
      active = "prompt-B"
      return { SessionState: { agent_activity: { agent: { active_prompt_count: 1 } }, session: { prompt_states: { agent: { active_prompt: { id: "prompt-A" } } } } } }
    }
    if (request.CancelActivePrompt?.expected_prompt_id === active) active = "cancelled"
    return { Error: { message: "stale prompt" } }
  } } as unknown as LocalIpcClient
  await assert.rejects(sendClaudeRemoteRenderedInput(client, "session", "attachment", "run", "\u001b", "agent"), /not admitted/)
  assert.equal(requests.at(-1).CancelActivePrompt.expected_prompt_id, "prompt-A")
  assert.equal(active, "prompt-B")
  assert(!requests.some(request => request.SendTerminalInput))
})
