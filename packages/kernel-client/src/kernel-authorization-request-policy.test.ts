import assert from "node:assert/strict"
import test from "node:test"
import { waitsForKernelAuthorization } from "./kernel-authorization-request-policy.js"

test("access, Vault management and exact sudo commands disable response timers and replay", () => {
  for (const prompt of ["/sudo", "/sudo task", "  /sudo\ttask", "\n/sudo task"]) {
    assert.equal(waitsForKernelAuthorization({ SubmitPrompt: { prompt } }), true)
  }
  assert.equal(waitsForKernelAuthorization({ RequestKernelAccess: {} }), true)
  assert.equal(waitsForKernelAuthorization({ ManageCredentialVault: { session_id: "s" } }), true)
  assert.equal(waitsForKernelAuthorization({ RequestKernelSudo: { agent_id: "a", prompt: "task" } }), true)
  for (const request of [null, undefined, "x", {}, { ListSessions: null },
    { SubmitPrompt: null }, { SubmitPrompt: { prompt: 1 } },
    ...["ordinary prompt", "/sudoku", "/sudo-task", "/SUDO task"].map(prompt => ({ SubmitPrompt: { prompt } }))]) {
    assert.equal(waitsForKernelAuthorization(request), false)
  }
})
