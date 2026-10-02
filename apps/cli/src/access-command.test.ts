import assert from "node:assert/strict"
import { test } from "node:test"
import { parseAccessRequest } from "./access-command.js"

test("access request uses the caller's selected OS ancestor without sending credentials", () => {
  const result = parseAccessRequest(["--session", "s", "--holder-pid", "42", "--minutes", "30", "--socket", "/tmp/k.sock"], () => { throw new Error("unused") })
  assert.deepEqual(result, { request: { RequestKernelAccess: { session_id: "s", holder_pid: 42, lifetime_minutes: 30 } }, socket: "/tmp/k.sock" })
  assert.equal(parseAccessRequest(["--session", "s"], () => 43).request.RequestKernelAccess.holder_pid, 43)
  for (const argv of [["--session", "s", "--holder-pid", "0"], ["--session", "s", "--minutes", "1.5"], ["--session", "s", "--holder-pid", "5", "--holder-pid", "6"], ["--passkey", "never"], []]) {
    assert.throws(() => parseAccessRequest(argv, () => 43))
  }
})
