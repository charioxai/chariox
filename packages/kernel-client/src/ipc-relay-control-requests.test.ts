import assert from "node:assert/strict"
import test from "node:test"

import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { issueCloudRelayClientTokenRequest, pollCloudRelayLoginRequest } from "./ipc-relay-control-requests.js"

test("key-bound CLI client-token request carries only the public thumbprint", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 439)
  assert.deepEqual(
    issueCloudRelayClientTokenRequest("home", "cli-1", "session-1", "public-thumbprint"),
    {
      IssueCloudRelayClientToken: {
        target_daemon_alias: "home",
        client_id: "cli-1",
        session_id: "session-1",
        public_key_thumbprint: "public-thumbprint",
      },
    },
  )
})

test("legacy client-token request omits identity binding explicitly", () => {
  assert.deepEqual(issueCloudRelayClientTokenRequest("home", "cli-1"), {
    IssueCloudRelayClientToken: {
      target_daemon_alias: "home",
      client_id: "cli-1",
      session_id: null,
    },
  })
})

test("kernel pivot binds the receiving terminal key on protocol 438", async () => {
  const { resolveKernelClientConnectionRequest } = await import("./ipc-relay-control-requests.js")
  assert.deepEqual(resolveKernelClientConnectionRequest({kernelRef: "kernel-b", clientId: "terminal-a", publicKeyThumbprint: "a".repeat(64)}), {ResolveKernelClientConnection: {kernel_ref: "kernel-b", machine_ref: null, client_id: "terminal-a", session_id: null, public_key_thumbprint: "a".repeat(64)}})
})

test("denial support is an explicit poll capability; legacy requests retain their shape", () => {
  const request = pollCloudRelayLoginRequest
  const legacy = { PollCloudRelayLogin: { api_url: "https://cloud.example.test", device_code: "synthetic-device-code" } }
  assert.deepEqual(request("https://cloud.example.test", "synthetic-device-code"), legacy)
  assert.deepEqual(request("https://cloud.example.test", "synthetic-device-code", false), legacy)
  assert.deepEqual(request("https://cloud.example.test", "synthetic-device-code", true), {
    PollCloudRelayLogin: { ...legacy.PollCloudRelayLogin, supports_access_denied: true },
  })
})
