import assert from "node:assert/strict"
import { createHash } from "node:crypto"
import test from "node:test"
import { pollCloudRelayLoginRequest } from "./ipc-relay-control-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION, type CloudRelayLoginPoll } from "./kernel-types.js"

test("device denial has the same versioned wire snapshot as Rust", () => {
  const result: CloudRelayLoginPoll = {
    expires_at: null, interval_seconds: null, profile: null, status: "access_denied",
  }
  const wire = { CloudRelayLoginPolled: { result } }
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 463)
  assert.equal(createHash("sha256").update(JSON.stringify(wire)).digest("hex"),
    "8524242d50a6a2307b56ec31b01b47d9ed25debc8d15b9b2f4fa48e3a389169d")
})

test("explicit denial support has a versioned request wire snapshot", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 463)
  const wire = pollCloudRelayLoginRequest("https://cloud.example.test", "synthetic-device-code", true)
  assert.equal(createHash("sha256").update(JSON.stringify(wire)).digest("hex"),
    "ecb2c3ec5f5860cb712393613a0c0b73fbbc4b5bb42eef051fc040de75859953")
})
