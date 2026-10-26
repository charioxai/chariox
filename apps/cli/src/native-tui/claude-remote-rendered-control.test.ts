// MP-08/MP-10/MP-11: interrupt an admitted prompt without inventing settlement.
import assert from "node:assert/strict"
import test from "node:test"
import type { LocalIpcClient } from "../ipc.js"
import { sendClaudeRemoteRenderedInput } from "./claude-remote-rendered-control.js"
test("bare Escape cancels the exact rendered Claude agent through normal kernel ownership", async () => {
  const requests: any[] = []
  const client = { send: async (r: any) => {requests.push(r);return r.GetSessionState
    ? {SessionState: {agent_activity: {agent: {active_prompt_count: 1}}}}
    : {PromptCancelled: {cancellation: {prompt: {id: "home-prompt"}}}}} } as unknown as LocalIpcClient
  await sendClaudeRemoteRenderedInput(client, "session", "attachment", "run", "\u001b", "agent")
  assert.deepEqual(requests.at(-1), {CancelActivePrompt: {session_id: "session", attachment_id: "attachment", target_agent_id: "agent"}})
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
    ? {SessionState: {agent_activity: {agent: {active_prompt_count: 1}}}} : {Error: {message: "not admitted"}}}} as unknown as LocalIpcClient
  await assert.rejects(sendClaudeRemoteRenderedInput(client, "session", "attachment", "run", "\u001b", "agent"), /not admitted/)
  assert(!requests.some(r => r.SendTerminalInput))
})
