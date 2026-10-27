import assert from "node:assert/strict"
import test from "node:test"
import { localKernelConnectionLabel } from "./local-connection-indicator.js"

const local = { kernelId: "home-1", machineId: "machine-1", host: "127.0.0.1", port: 43119, heartbeatAtMs: 50_000 }
const relay = { socketPath: "wss://relay.example", isRelayTransport: () => true }

test("MP-08 Local/Relay display requires fresh same-machine discovery for the current kernel", () => {
  assert.equal(localKernelConnectionLabel(relay, "home-1", [local], 50_000), "Relay connection")
  assert.equal(localKernelConnectionLabel(relay, "remote-1", [local], 50_000), null)
  assert.equal(localKernelConnectionLabel(relay, "home-1", [local], 80_001), null)
  assert.equal(localKernelConnectionLabel(relay, "home-1", [], 50_000), null)
  assert.equal(localKernelConnectionLabel(relay, "slice:child", [{ ...local, kernelId: "slice:child" }], 50_000), null)
  assert.equal(localKernelConnectionLabel({ socketPath: "ws://127.0.0.1:43119/kernel", isRelayTransport: () => false }, "home-1", [local], 50_000), "Local connection")
  assert.equal(localKernelConnectionLabel({ socketPath: "wss://remote.example/kernel", isRelayTransport: () => false }, "home-1", [local], 50_000), null)
})
