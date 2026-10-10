import assert from "node:assert/strict"
import test from "node:test"
import { createCliSessionLifecycleComposition, type CliSessionLifecycleCompositionDeps } from "./cli-session-lifecycle-composition.js"

// MP-08 / MP-10 / MP-11: a detached terminal has no local runtime to subscribe to.
test("detached event subscription waits for kernel selection then follows the real connection", async () => {
  let connected = false
  let subscriptions = 0
  const failures: string[] = []
  const composition = createCliSessionLifecycleComposition({
    client: { subscribeToWaitingRoomInventory: async () => {
      subscriptions++
      if (!connected) throw new Error("local kernel unavailable")
    } },
    options: {},
    renderer: {},
    supportsKernelEventStream: true,
    kernelConnected: () => connected,
    closingStateController: { isClosing: () => false, setClosing: () => {} },
    attachmentState: () => null,
    sessionState: () => ({ id: "no-session" }),
    formatError: (error: Error) => error.message,
    setDaemonDisconnected: () => {},
    setStatusLine: () => {},
    updateSessionChrome: () => {},
    appendNotice: (message: string) => failures.push(message),
  } as unknown as CliSessionLifecycleCompositionDeps)
  await composition.syncKernelEventSubscription()
  assert.equal(subscriptions, 0, "no local endpoint should be contacted before selecting a kernel")
  assert.deepEqual(failures, [])
  connected = true
  await composition.syncKernelEventSubscription()
  await composition.syncKernelEventSubscription()
  assert.equal(subscriptions, 1)
  assert.equal(composition.kernelEventSubscriptionController.state().scope, "waiting-room")
})
