// MP-08 / MP-10 / MP-11: delayed display controls cannot interrupt a successor.
import assert from "node:assert/strict"
import test from "node:test"
import type { LocalIpcClient } from "../ipc.js"
import { bindCodexDisplayTurn } from "./codex-conversation-controls.js"
import { handleCodexNativeTurnInterrupt, type CodexNativeBindingState } from "./codex-turn-submission.js"

async function interrupt(threadId: string, turnId: string, active: string) {
  const state: CodexNativeBindingState = { promise: null, run: null }
  bindCodexDisplayTurn(state, "display-thread", "turn-A", "prompt-A")
  bindCodexDisplayTurn(state, "display-thread", "turn-B", "prompt-B")
  const requests: any[] = [], responses: any[] = []
  let cancelled: string | null = null
  const client = { async send(request: any) {
    requests.push(request)
    const expected = request.CancelActivePrompt?.expected_prompt_id
    if (expected && expected !== active) throw new Error("active prompt changed before cancellation")
    cancelled = active
    return { PromptCancelled: { cancellation: { prompt: { id: active } } } }
  } } as LocalIpcClient
  await handleCodexNativeTurnInterrupt({ id: 1, method: "turn/interrupt", params: { threadId, turnId } },
    { client, sessionId: "session", attachmentId: "attachment", agentId: "agent", bindState: state },
    response => responses.push(response))
  return { requests, responses, cancelled }
}

test("MP-08 MP-10 delayed turn A interrupt leaves current prompt B running", async () => {
  const result = await interrupt("display-thread", "turn-A", "prompt-B")
  assert.equal(result.cancelled, null)
  assert.equal(result.requests[0].CancelActivePrompt.expected_prompt_id, "prompt-A")
  assert.match(result.responses[0].error.message, /active prompt changed/)
})

test("MP-08 MP-10 foreign and unbound interrupts never reach the kernel", async () => {
  for (const [thread,turn] of [["foreign-thread", "turn-B"], ["display-thread", "unbound-turn"]] as const) {
    const result = await interrupt(thread, turn, "prompt-B")
    assert.equal(result.cancelled, null)
    assert.equal(result.requests.length, 0)
    assert.match(result.responses[0].error.message, /not bound/)
  }
})

test("MP-08 MP-10 current bound turn B interruption carries its exact prompt", async () => {
  const result = await interrupt("display-thread", "turn-B", "prompt-B")
  assert.equal(result.cancelled, "prompt-B")
  assert.equal(result.requests[0].CancelActivePrompt.expected_prompt_id, "prompt-B")
  assert.deepEqual(result.responses, [{ id: 1, result: {} }])
})
