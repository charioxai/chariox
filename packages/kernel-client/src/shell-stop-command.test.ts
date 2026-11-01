// MP-08 / MP-10 / MP-11: shared Stop covers active prompts and parked tasks.
import assert from "node:assert/strict"
import test from "node:test"
import { createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"

for (const [response, message] of [
  [{ PromptCancelled: { cancellation: { prompt: { id: "active" } } } }, "cancellation requested for prompt active"],
  [{ AgentTaskUpdated: { session_id: "room", task: { task_id: "parked", state: "cancelled" } } }, "cancelled task parked"],
] as const) {
  test(`shared stop reports ${message}`, async () => {
    const requests: Record<string, unknown>[] = []
    const context = { ...createDefaultShellContext({ workspace: "/repo", worktree: "/repo" }), sessionId: "room", attachmentId: "terminal", agentId: "agent" }
    const result = await executeShellCommand(parseShellCommand("stop"), context, {
      client: { send: async (request) => { requests.push(request); return response } },
    })
    assert.equal(result.ok, true)
    assert.equal(result.message, message)
    assert.deepEqual(requests, [{ CancelActivePrompt: { session_id: "room", attachment_id: "terminal", target_agent_id: "agent" } }])
    assert.deepEqual(result.data, Object.values(response)[0])
  })
}
