import assert from "node:assert/strict"
import test from "node:test"
import { createECDH } from "node:crypto"

import { LocalIpcClient, RelayClientIdentity } from "./ipc.js"
import { createCliCommandActionComposition } from "./cli-command-action-composition.js"
import {
  beginMutableLocalIpcClientPivot,
  createMutableLocalIpcClient,
} from "./mutable-local-ipc-client.js"

test("command composition starts with the mutable local client used by the app", () => {
  const client = createMutableLocalIpcClient(new LocalIpcClient("ws://127.0.0.1:1"))
  const handlers = createCliCommandActionComposition({
    client,
    options: {},
    preferencesState: () => ({}),
  } as Parameters<typeof createCliCommandActionComposition>[0])

  assert.equal(typeof handlers.handleRelayCommand, "function")
  assert.equal(client.getRelayClientIdentity(), null)
})

test("relay identity and transport follow a client pivot and rollback", async () => {
  const keys = createECDH("prime256v1")
  keys.generateKeys()
  const identity = new RelayClientIdentity(keys.getPrivateKey())
  const source = new LocalIpcClient("ws://127.0.0.1:1")
  const target = new LocalIpcClient("wss://relay.example.test", {
    relayAuthToken: "synthetic-test-token",
    relayIdentity: identity,
  })
  const client = createMutableLocalIpcClient(source)

  assert.equal(client.getRelayClientIdentity(), null)
  assert.equal(client.isRelayTransport(), false)
  const pivot = beginMutableLocalIpcClientPivot(client, target)
  assert.equal(client.getRelayClientIdentity(), identity)
  assert.equal(client.isRelayTransport(), true)
  await pivot.rollback()
  assert.equal(client.getRelayClientIdentity(), null)
  assert.equal(client.isRelayTransport(), false)
})

test("mutable IPC client rollback restores the source before the next request", async () => {
  const source = fakeClient("source")
  const target = fakeClient("target")
  const client = createMutableLocalIpcClient(source.client)
  const pivot = beginMutableLocalIpcClientPivot(client, target.client)

  assert.equal(await client.send<string>({ type: "during-pivot" }), "target")
  await pivot.rollback()

  assert.equal(await client.send<string>({ type: "next-local-launch" }), "source")
  assert.equal(source.closeCount(), 0)
  assert.equal(target.closeCount(), 1)
})

test("mutable IPC client commit keeps the target and closes the source", async () => {
  const source = fakeClient("source")
  const target = fakeClient("target")
  const client = createMutableLocalIpcClient(source.client)
  const pivot = beginMutableLocalIpcClientPivot(client, target.client)

  await pivot.commit()

  assert.equal(await client.send<string>({ type: "managed-launch" }), "target")
  assert.equal(source.closeCount(), 1)
  assert.equal(target.closeCount(), 0)
})

function fakeClient(name: string) {
  let closes = 0
  const client = {
    socketPath: name,
    supportsKernelEvents: () => true,
    send: async () => name,
    subscribeToKernelEvents: async () => {},
    subscribeToWaitingRoomInventory: async () => {},
    unsubscribeFromKernelEvents: async () => {},
    restartKernelEventStream: async () => {},
    onKernelEvent: () => () => {},
    close: async () => {
      closes += 1
    },
    destroy: () => {},
  } as unknown as LocalIpcClient
  return {
    client,
    closeCount: () => closes,
  }
}
