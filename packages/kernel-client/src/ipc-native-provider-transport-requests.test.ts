import assert from "node:assert/strict"
import test from "node:test"
import { nativeProviderTransportRequest, nativeProviderTransportMinimumProtocolVersion } from "./ipc-native-provider-transport-requests.js"
import { LOCAL_DAEMON_PROTOCOL_VERSION } from "./kernel-types.js"
test("MP-08 native provider transport carries owning scope and bounded operations, never a URL", () => {
  assert.equal(LOCAL_DAEMON_PROTOCOL_VERSION, 526)
  assert.equal(nativeProviderTransportMinimumProtocolVersion, 526)
  const scope = { sessionId: "session", attachmentId: "attachment", agentId: "agent", providerRunId: "run" }
  assert.deepEqual(nativeProviderTransportRequest({ ...scope, operation: { kind: "open" } }), {
    NativeProviderTransport: { session_id: "session", attachment_id: "attachment", agent_id: "agent", provider_run_id: "run", operation: {kind:"open"} },
  })
  const exchange = nativeProviderTransportRequest({ ...scope, operation: {kind: "exchange", connection_id: "connection", frames: ['{"id":1,"method":"initialize"}']} })
  assert.equal(exchange.NativeProviderTransport.provider_run_id, "run")
  assert.deepEqual(Object.keys(exchange.NativeProviderTransport).sort(), ["agent_id", "attachment_id", "operation", "provider_run_id", "session_id"])
})
