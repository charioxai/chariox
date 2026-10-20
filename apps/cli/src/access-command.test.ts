// MP-08 / MP-10 / MP-11: one process-bound grant for the whole local kernel.
import assert from "node:assert/strict"
import { test } from "node:test"
import { parseAccessRequest } from "./access-command.js"

test("access request has no session scope and defaults to the caller's OS ancestor", () => {
  const result = parseAccessRequest(["--holder-pid", "42", "--minutes", "480", "--socket", "/tmp/k.sock"], () => { throw new Error("unused") })
  assert.deepEqual(result, { request: { RequestKernelAccess: { holder_pid: 42, term: { minutes: 480 } } }, socket: "/tmp/k.sock" })
  assert.equal(parseAccessRequest([], () => 43).request.RequestKernelAccess.holder_pid, 43)
  for (const argv of [["--session", "s"], ["--holder-pid", "0"], ["--holder-pid", "1"], ["--minutes", "1.5"], ["--holder-pid", "5", "--holder-pid", "6"], ["--passkey", "never"], ["--until-exit", "--minutes", "30"], ["--until-exit", "--until-exit"]]) {
    assert.throws(() => parseAccessRequest(argv, () => 43))
  }
})

test("access request can ask to last until the agent exits, and says nothing when no term is given", () => {
  assert.deepEqual(parseAccessRequest(["--until-exit"], () => 43).request, { RequestKernelAccess: { holder_pid: 43, term: "until_holder_exits" } })
  assert.deepEqual(parseAccessRequest(["--socket", "/tmp/k.sock", "--until-exit", "--holder-pid", "7"], () => 43), { request: { RequestKernelAccess: { holder_pid: 7, term: "until_holder_exits" } }, socket: "/tmp/k.sock" })
  assert.deepEqual(parseAccessRequest([], () => 43).request, { RequestKernelAccess: { holder_pid: 43 } })
})
