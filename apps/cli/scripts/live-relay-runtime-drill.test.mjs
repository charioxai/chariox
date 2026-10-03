import assert from "node:assert/strict"
import test from "node:test"
import { makeChildrenEnv } from "./live-relay-runtime-drill.mjs"

test("scoped relay kernel registration binds the explicit daemon machine identity", () => {
  const { daemonEnv } = makeChildrenEnv({ relayPort: 12345, kernelPort: 12346 }, "/tmp/chariox-owned-relay")
  const claims = JSON.parse(Buffer.from(daemonEnv.CHARIOX_RELAY_TOKEN.split(".")[1], "base64url"))
  assert.equal(typeof daemonEnv.CHARIOX_MACHINE_ID, "string")
  assert.notEqual(daemonEnv.CHARIOX_MACHINE_ID, daemonEnv.CHARIOX_DAEMON_ID)
  assert.equal(claims.machine_id, daemonEnv.CHARIOX_MACHINE_ID)
  assert.equal(claims.sub, daemonEnv.CHARIOX_DAEMON_ID)
})
