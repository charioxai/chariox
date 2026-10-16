import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"
import { localProtocolVersion, relayPeerProtocolVersion } from "./lib/protocol-versions.mjs"

const read = path => readFileSync(new URL("../" + path, import.meta.url), "utf8")

test("MP-08 / MP-11 C5 retains local 520 and peer 128 floors under later allocations", () => {
  const local = read("apps/kernel/src/local/api/types.rs")
  const peer = read("apps/kernel/src/transport/relay_peer.rs")
  assert.ok(localProtocolVersion >= 520, "current local protocol must retain the C5 floor")
  assert.ok(relayPeerProtocolVersion >= 128, "current relay peer protocol must retain the C5 floor")
  const client = read("packages/kernel-client/src/kernel-types.ts")
  assert.equal(Number(client.match(/LOCAL_DAEMON_PROTOCOL_VERSION = (\d+)\b/)[1]), localProtocolVersion)
  assert.doesNotMatch(local + peer, /PROTOCOL-ALLOCATION-PENDING/)
  const helpers = read("packages/kernel-client/src/ipc-managed-context-requests.ts")
  assert.match(helpers, /OWNER_MANAGED_CONTEXT_MINIMUM_PROTOCOL_VERSION = 520\b/)
  assert.match(helpers, /OWNER_MANAGED_CONTEXT_MINIMUM_RELAY_PROTOCOL_VERSION = 128\b/)
  assert.match(read("docs/PROTOCOL.md"), /C5.*local 520.*peer 128/)
})
