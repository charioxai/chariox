import assert from "node:assert/strict"
import test from "node:test"
import { LocalIpcError } from "./local-ipc-error.js"
import { userDomainRefusalReasons, userDomainRefusalReason } from "./user-domain-refusal.js"
for (const reason of userDomainRefusalReasons) test("typed kernel refusal survives CLI transport: " + reason, () => {
  const error = new LocalIpcError("handle kernel response", "User-domain request refused", "user_domain_" + reason, false)
  assert.equal(error.refusalReason, reason)
  assert.equal(error.retryable, false)
  assert.ok(error.message.includes("reason=" + reason))
})
test("generic transport and invented codes do not become CLI refusals", () => {
  for (const code of [null, "local_transport_error", "kernel_browser_failed", "user_domain_internal", "not_granted"]) {
    assert.equal(userDomainRefusalReason(code), null)
    assert.equal(new LocalIpcError("read", "user_domain_not_granted", code).refusalReason, null)
  }
})
