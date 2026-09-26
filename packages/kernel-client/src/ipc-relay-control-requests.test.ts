import assert from "node:assert/strict"
import test from "node:test"

import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
import { issueCloudRelayClientTokenRequest } from "./ipc-relay-control-requests.js"

test("key-bound CLI client-token request carries only the public thumbprint", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 349)
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
