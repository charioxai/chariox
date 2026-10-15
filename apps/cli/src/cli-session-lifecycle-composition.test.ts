import assert from "node:assert/strict"
import test from "node:test"

import {
  createCliSessionLifecycleComposition,
  type CliSessionLifecycleCompositionDeps,
} from "./cli-session-lifecycle-composition.js"

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

// MP-08 / MP-11: a failed inventory binding must not replace the waiting room.
test("waiting-room subscription failure retains rows and marks them reconnecting", async () => {
  const calls: string[] = []
  const values = {
    client: { subscribeToWaitingRoomInventory: async () => { throw new Error("unable to authenticate data") } },
    options: {}, renderer: {}, appLogger: { error: () => {}, debug: () => {} },
    closingStateController: { isClosing: () => false, setClosing: () => {} },
    supportsKernelEventStream: true,
    // MP-08 / MP-10 / MP-11: #888 subscribes only once a kernel is connected.
    kernelConnected: () => true,
    attachmentState: () => null,
    isAttached: () => false,
    formatError: (error: Error) => error.message,
    setDaemonDisconnected: (value: boolean) => calls.push(`disconnected:${value}`),
    setStatusLine: (value: string) => calls.push(`status:${value}`),
    appendNotice: () => calls.push("notice-replaces-list"),
    applyWaitingRoomTransportClosed: () => calls.push("retain-offline-rows"),
    updateSessionChrome: () => calls.push("chrome"),
  }
  // The other lifecycle operations are outside this subscription scenario.
  const deps = new Proxy(values, { get: (target, key) => key in target ? target[key as keyof typeof target] : () => {} }) as unknown as CliSessionLifecycleCompositionDeps
  await createCliSessionLifecycleComposition(deps).syncKernelEventSubscription()
  assert(calls.includes("disconnected:true"))
  assert(calls.includes("retain-offline-rows"))
  assert(!calls.includes("notice-replaces-list"))
})

// MP-08 / MP-10 (panes R12, PANE-15; panetest-d T-10 RED): a remote TUI whose
// attachment the kernel no longer has re-attaches and re-subscribes instead of
// retrying the rejected session/attachment pair forever.
test("a stale attachment subscription failure runs session-unavailable recovery", async () => {
  const recoveries: string[] = []
  const deps = {
    client: {
      subscribeToKernelEvents: async () => {
        throw new Error("kernel local transport `subscribe` failed: attachment `att-old` does not belong to session `s-1`")
      },
    },
    options: {}, renderer: {}, appLogger: { error: () => {}, debug: () => {} },
    closingStateController: { isClosing: () => false, setClosing: () => {} },
    supportsKernelEventStream: true,
    kernelConnected: () => true,
    attachmentState: () => ({ id: "att-old", session_id: "s-1" }),
    sessionState: () => ({ id: "s-1" }),
    isAttached: () => true,
    formatError: (error: Error) => error.message,
    setDaemonDisconnected: () => {},
    setStatusLine: () => {},
    appendNotice: () => {},
    updateSessionChrome: () => {},
    handleSessionUnavailable: (message: string) => { recoveries.push(message) },
  } as unknown as CliSessionLifecycleCompositionDeps

  await createCliSessionLifecycleComposition(deps).syncKernelEventSubscription()

  assert.equal(recoveries.length, 1)
  assert.match(recoveries[0] ?? "", /does not belong to session/)
})
