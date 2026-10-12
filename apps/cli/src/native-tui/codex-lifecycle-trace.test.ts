// MP-08 / MP-10 / MP-11: diagnostics must never retain arbitrary payload fields or raw IDs.
import assert from "node:assert/strict"
import test from "node:test"
import { codexLifecycleTraceRecord } from "./codex-lifecycle-trace.js"
test("native lifecycle trace excludes bodies, credentials and raw identifiers", () => {
  const result = JSON.stringify(codexLifecycleTraceRecord("steer_receipt", {
    clientId: "private-client-id", promptId: "private-prompt-id",
    input: [{ text: "private message body" }], authorization: "private bearer value",
  } as any))
  for (const value of ["private-client-id", "private-prompt-id", "private message body", "private bearer value", "authorization", "input"])
    assert(!result.includes(value))
  assert.match(result, /"client":"[0-9a-f]{64}"/)
})
