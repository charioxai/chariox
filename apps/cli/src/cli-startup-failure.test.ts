import assert from "node:assert/strict"
import test from "node:test"
import { CloudClient } from "./cloud-client.js"
import { issueCloudPairingBootstrapToken } from "./cloud-client.js"
import { CloudClientAuthError } from "./cloud-client-http.js"
import { cliStartupFailureText } from "./cli-startup-failure.js"

test("a missing hosted receiving-terminal login is a typed outcome", async () => {
  const previous = CloudClient.prototype.profile
  CloudClient.prototype.profile = async () => null
  try {
    await assert.rejects(issueCloudPairingBootstrapToken("wss://relay.example.test", "kernel", {} as never),
      (error: unknown) => error instanceof CloudClientAuthError && error.code === "login_required")
  } finally { CloudClient.prototype.profile = previous }
})

test("the startup view explains how to recover a missing or ended terminal login", () => {
  for (const code of ["login_required", "signed_out", "client_revoked", "refresh_reuse_detected"]) {
    assert.match(cliStartupFailureText(new CloudClientAuthError(code)), /chariox cloud login.*--api-url.*retry/)
  }
  assert.match(cliStartupFailureText(new CloudClientAuthError("profile_conflict")), /separate CHARIOX_HOME/)
  assert.equal(cliStartupFailureText(new Error("private provider diagnostic")), "Unable to attach to this session.")
})
