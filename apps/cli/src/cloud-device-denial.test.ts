import assert from "node:assert/strict"
import test from "node:test"
import { startHostedCloudLink, type CloudCommandLifecycleDeps } from "./cloud-command-lifecycle.js"

test("a denied device sign-in stops polling and reports refusal without saving or connecting", async () => {
  const notices: string[] = []
  let polls = 0
  const deps: CloudCommandLifecycleDeps = {
    appendNotice: (message: string) => { notices.push(message) },
    flashFooter: () => {},
    formatError: String,
    getRelayStatus: async () => ({ configured: false, connected: false, relay_token_configured: false, machine_id: "fixture-machine", daemon_id: "fixture-kernel" }),
    startCloudDeviceLogin: async () => ({ apiUrl: "https://cloud.example.test", deviceCode: "synthetic-device-code", userCode: "ABCD-EFGH", verificationUrl: "https://cloud.example.test/activate?user_code=ABCD-EFGH", expiresAtMs: Date.now() + 100, intervalSeconds: 1 }),
    pollCloudDeviceLogin: async () => { polls += 1; return { status: "access_denied" as const } },
    saveCloudRelayProfile: async () => { assert.fail("denied login must not save a profile") },
    configureRelay: async () => { assert.fail("denied login must not connect") },
  }
  await startHostedCloudLink(deps)
  assert.equal(polls, 1)
  assert.equal(notices.at(-1), "cloud login denied by the account owner")
})
