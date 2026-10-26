import assert from "node:assert/strict"
import test from "node:test"
import { LocalIpcClient } from "./ipc.js"
import type { KernelEvent } from "./kernel-events.js"

// MP-08 / MP-10 (panes R12, PANE-15; panetest-d T-10 RED): when the kernel
// rejects the stored session/attachment pair on resume, the client stops
// retrying it and emits session_unavailable so the TUI re-attaches.
test("a rejected subscription pair stops reconnecting and reports session_unavailable", async () => {
  const client = new LocalIpcClient("ws://127.0.0.1:9/kernel")
  const events: KernelEvent[] = []
  client.onKernelEvent((event) => { events.push(event) })
  const internals = client as unknown as {
    activeKernelSubscription: unknown
    reconnectTimeout: unknown
    sendWebSocket: () => Promise<never>
    resumeKernelSubscription: () => Promise<void>
  }
  internals.activeKernelSubscription = {
    sessionId: "s-1", attachmentId: "att-old", scope: "session",
    relaySubscriptionId: null, relayPublicKey: null, relayDecryptEvent: null,
  }
  internals.sendWebSocket = async () => {
    throw new Error("kernel local transport `subscribe` failed: attachment `att-old` does not belong to session `s-1`")
  }

  await internals.resumeKernelSubscription()

  assert.equal(internals.activeKernelSubscription, null)
  assert.equal(internals.reconnectTimeout, null)
  assert.deepEqual(events.map((event) => event.event), ["session_unavailable"])
  await client.close?.()
})
