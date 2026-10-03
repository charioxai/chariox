// MP-10: use the existing kernel client auth consumer, never a second issuer.
import assert from "node:assert/strict"
import test from "node:test"
import { managedOrdinaryKernelConnection, prepareManagedOrdinaryProbeEnvironment } from "./managed-ordinary-kernel-endpoint.mjs"

test("MP-10 consumed one-shot auth survives only in owned probe context", async () => {
  const environment = {
    CHARIOX_KERNEL_URL: "ws://127.0.0.1:43118/kernel",
    CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE: "/private/one-shot",
    CHARIOX_PARITY_SIGNING_KEY: "manifest-key",
    PATH: "/usr/bin",
  }
  const prepared = await prepareManagedOrdinaryProbeEnvironment(environment, {
    consumeLocalAuthToken: (endpoint) => {
      assert.equal(endpoint, "ws://127.0.0.1:43118/")
      delete environment.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE
      return "admitted-loopback-bearer"
    },
  })
  assert.equal(prepared.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN, "admitted-loopback-bearer")
  assert.equal(prepared.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE, undefined)
  assert.equal(prepared.CHARIOX_PARITY_SIGNING_KEY, undefined)
  assert.equal(prepared.PATH, "/usr/bin")
  assert.equal(environment.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN, undefined)
})

test("MP-10 an already-consumed client context remains usable by probes", async () => {
  const prepared = await prepareManagedOrdinaryProbeEnvironment({ CHARIOX_KERNEL_URL: "ws://[::1]:43118/" }, {
    consumeLocalAuthToken: () => "previously-admitted-bearer",
  })
  assert.equal(prepared.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN, "previously-admitted-bearer")
})

test("MP-10 endpoint and mixed credential rejection precede auth consumption", async () => {
  const invalid = [
    { CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE: "/private/token" },
    { CHARIOX_KERNEL_URL: "ws://localhost:43118", CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE: "/private/token" },
    { CHARIOX_KERNEL_URL: "ws://127.0.0.1:43118/?token=secret" },
    { CHARIOX_KERNEL_URL: "wss://relay.example.test/", CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE: "/private/token",
      CHARIOX_PARITY_PROJECT_SETUP_RELAY_TOKEN: "relay-bearer",
      CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON: JSON.stringify({kernel_identity: { kernel_id: "kernel-target" }}) },
  ]
  for (const environment of invalid) {
    await assert.rejects(prepareManagedOrdinaryProbeEnvironment(environment, {
      consumeLocalAuthToken: () => { assert.fail("unadmitted endpoint consumed credential") },
    }))
  }
})

test("MP-10 approved relay context targets the exact kernel and never consumes local auth", async () => {
  const environment = { CHARIOX_KERNEL_URL: "wss://relay.example.test/runtime",
    CHARIOX_PARITY_PROJECT_SETUP_RELAY_TOKEN: "relay-bearer",
    CHARIOX_PARITY_CAPTURE_EVIDENCE_JSON: JSON.stringify({ kernel_identity: { kernel_id: "kernel-target" }}) }
  const prepared = await prepareManagedOrdinaryProbeEnvironment(environment, {
    consumeLocalAuthToken: () => assert.fail("relay consumed local auth"),
  })
  assert.deepEqual(managedOrdinaryKernelConnection(prepared), {
    endpoint: environment.CHARIOX_KERNEL_URL, transport: "relay",
    clientOptions: { relayAuthToken: "relay-bearer", targetDaemonId: "kernel-target" },
  })
})
