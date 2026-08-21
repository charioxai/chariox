import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "./ipc.js"
import {
  beginMutableLocalIpcClientPivot,
  createMutableLocalIpcClient,
} from "./mutable-local-ipc-client.js"

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
