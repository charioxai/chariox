import assert from "node:assert/strict"
import test from "node:test"

import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { issueCloudRelayClientTokenRequest, resolveKernelClientConnectionRequest } from "./ipc-relay-control-requests.js"

test("key-bound CLI client-token request carries only the public thumbprint", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 473)
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


test("MP-08 protocol473 resolved terminal token binds the same CLI public key", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 473)
  assert.deepEqual(resolveKernelClientConnectionRequest({ kernelRef: "home", clientId: "cli", publicKeyThumbprint: "thumbprint" }), {
    ResolveKernelClientConnection: { kernel_ref: "home", machine_ref: null, client_id: "cli", session_id: null, public_key_thumbprint: "thumbprint" },
  })
})
