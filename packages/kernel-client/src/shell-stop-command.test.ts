// MP-08 / MP-10 / MP-11: shared Stop covers active prompts and parked tasks.
import assert from "node:assert/strict"
import test from "node:test"
import { createDefaultShellContext, parseShellCommand } from "./shell-core.js"
import { executeShellCommand } from "./shell-executor.js"

for (const [response, message] of [
  [{ PromptCancelled: { cancellation: { prompt: { id: "active" } } } }, "cancellation requested for prompt active"],
  [{ AgentTaskUpdated: { session_id: "room", task: { task_id: "parked", state: "cancelled" } } }, "cancelled task parked"],
  [{ NothingToStop: { session_id: "room", agent_id: "agent" } }, "nothing to stop"],
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

for (const [command, request, message] of [
  ["stop task parked 8", { CancelAgentTask: { session_id: "room", attachment_id: "terminal", task_id: "parked", revision: 8 } }, "cancelled task parked"],
  ["cancel task parked 8", { CancelAgentTask: { session_id: "room", attachment_id: "terminal", task_id: "parked", revision: 8 } }, "cancelled task parked"],
  ["stop timer review-timer", { CancelAgentWake: { session_id: "room", attachment_id: "terminal", wake_id: "review-timer" } }, "cancelled timer review-timer"],
] as const) {
  test(`shared shell ${command} cancels exact retained work`, async () => {
    const requests: Record<string, unknown>[] = []
    const task = { task_id: "parked", state: command.includes("timer") ? "waiting" : "cancelled" }
    const response = { AgentTaskUpdated: { session_id: "room", task } }
    const context = { ...createDefaultShellContext(), sessionId: "room", attachmentId: "terminal", agentId: "unrelated-active-agent" }
    const result = await executeShellCommand(parseShellCommand(command), context, {
      client: { send: async (sent) => { requests.push(sent); return response } },
    })
    assert.equal(result.ok, true)
    assert.equal(result.message, message)
    assert.deepEqual(requests, [request])
    assert.deepEqual(result.data, response.AgentTaskUpdated)
  })
}

test("shared stop validates exact arguments and revision before sending requests", async () => {
  for (const command of ["stop task", "stop task t", "stop task t -1", "stop task t 1.5", "stop task t NaN", "stop task t 9007199254740992", "stop task t 1 extra", "stop timer", "stop timer t extra", "stop unknown"]) {
    const context = { ...createDefaultShellContext(), sessionId: "room", attachmentId: "terminal" }
    const result = await executeShellCommand(parseShellCommand(command), context, {
      client: { send: async () => { assert.fail(`invalid command sent a request: ${command}`) } },
    })
    assert.equal(result.ok, false, command)
    assert.equal(result.message, "usage: stop [task <task-id> <revision> | timer <wake-id>]", command)
  }
})
