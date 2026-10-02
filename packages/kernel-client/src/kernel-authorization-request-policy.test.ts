import assert from "node:assert/strict"
import test from "node:test"
import { waitsForKernelAuthorization } from "./kernel-authorization-request-policy.js"

test("only access and exact sudo commands disable response timers and replay", () => {
  for (const prompt of ["/sudo", "/sudo task", "  /sudo\ttask", "\n/sudo task"]) {
    assert.equal(waitsForKernelAuthorization({ SubmitPrompt: { prompt } }), true)
  }
  assert.equal(waitsForKernelAuthorization({ RequestKernelAccess: {} }), true)
  for (const request of [null, undefined, "x", {}, { ListSessions: null },
    { SubmitPrompt: null }, { SubmitPrompt: { prompt: 1 } },
    ...["ordinary prompt", "/sudoku", "/sudo-task", "/SUDO task"].map(prompt => ({ SubmitPrompt: { prompt } }))]) {
    assert.equal(waitsForKernelAuthorization(request), false)
  }
})
