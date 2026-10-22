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
    onKernelTransportClosed: () => calls.push("disconnected:true", "retain-offline-rows", "chrome"),
    updateSessionChrome: () => calls.push("chrome"),
  }
  // The other lifecycle operations are outside this subscription scenario.
  const deps = new Proxy(values, { get: (target, key) => key in target ? target[key as keyof typeof target] : () => {} }) as unknown as CliSessionLifecycleCompositionDeps
  await createCliSessionLifecycleComposition(deps).syncKernelEventSubscription()
  assert(calls.includes("disconnected:true"))
  assert(calls.includes("retain-offline-rows"))
  assert(!calls.includes("notice-replaces-list"))
})

// MP-08/MP-10: restart recovery must settle the same health timer as socket recovery.
test("attached restart recovery uses shared connection chrome without agent notices", async () => {
  const notices: string[] = []
  let recovered = 0
  let disconnected = true
  const values = {
    client: { send: async (request: Record<string, unknown>) => "GetSessionState" in request
      ? { SessionState: { session: { id: "session", agents: [] } } }
      : { SessionAttached: { attachment: { id: "attachment", session_id: "session" } } }, subscribeToKernelEvents: async () => {} },
    options: { clientId: "client" }, renderer: {}, appLogger: { debug: () => {}, error: () => {} }, supportsKernelEventStream: true,
    sleep: async () => { throw Error("unexpected recovery retry") },
    closingStateController: { isClosing: () => false, setClosing: () => {} },
    kernelConnected: () => true, isAttached: () => true,
    daemonDisconnected: () => disconnected,
    sessionState: () => ({ id: "session" }), attachmentState: () => ({ id: "attachment", session_id: "session" }),
    providerRunState: () => null,
    setDaemonDisconnected: (value: boolean) => { disconnected = value },
    onKernelTransportRecovered: () => { recovered++; disconnected = false },
    appendNotice: (text: string) => notices.push(text),
  }
  const deps = new Proxy(values, { get: (target, key) => key in target ? target[key as keyof typeof target] : () => {} }) as unknown as CliSessionLifecycleCompositionDeps
  await createCliSessionLifecycleComposition(deps).recoverAttachedSessionAfterKernelRestart()
  assert.equal(recovered, 1)
  assert.deepEqual(notices, [])
})

test("MP-08/MP-10 attached subscription failures stay in shared chrome", async () => {
  const notices: string[] = []
  let closed = 0
  const values = {
    client: { subscribeToKernelEvents: async () => { throw Error("transport closed") } },
    options: {}, renderer: {}, appLogger: { debug: () => {}, error: () => {} }, supportsKernelEventStream: true,
    closingStateController: { isClosing: () => false, setClosing: () => {} },
    kernelConnected: () => true, isAttached: () => true,
    sessionState: () => ({ id: "session" }), attachmentState: () => ({ id: "attachment", session_id: "session" }),
    formatError: (error: Error) => error.message,
    onKernelTransportClosed: () => { closed++ },
    appendNotice: (text: string) => notices.push(text),
  }
  const deps = new Proxy(values, { get: (target, key) => key in target ? target[key as keyof typeof target] : () => {} }) as unknown as CliSessionLifecycleCompositionDeps
  await createCliSessionLifecycleComposition(deps).syncKernelEventSubscription()
  assert.equal(closed, 1)
  assert.deepEqual(notices, [])
})

test("MP-08/MP-10 recovery retries a failed session subscription before declaring recovery", async () => {
  let disconnected = true
  let attachment = { id: "old", session_id: "session" }
  let subscriptions = 0
  let repairs = 0
  let recovered = 0
  let retries = 0
  const values = {
    client: {
      send: async (request: Record<string, unknown>) => "GetSessionState" in request
        ? { SessionState: { session: { id: "session", agents: [] } } }
        : { SessionAttached: { attachment: { id: `repair-${++repairs}`, session_id: "session" } } },
      subscribeToKernelEvents: async () => {
        if (++subscriptions === 1) throw Error("event lane has not recovered")
      },
    },
    options: { clientId: "client" }, renderer: {},
    appLogger: { debug: () => {}, error: () => {} }, supportsKernelEventStream: true,
    closingStateController: { isClosing: () => false, setClosing: () => {} },
    kernelConnected: () => true, isAttached: () => true,
    daemonDisconnected: () => disconnected, sessionState: () => ({ id: "session" }),
    attachmentState: () => attachment,
    setAttachmentState: (next: typeof attachment) => { attachment = next },
    providerRunState: () => null, formatError: (error: Error) => error.message,
    setDaemonDisconnected: (value: boolean) => { disconnected = value },
    onKernelTransportClosed: () => { disconnected = true },
    onKernelTransportRecovered: () => { recovered++; disconnected = false },
    sleep: async () => { retries++ },
  }
  const deps = new Proxy(values, { get: (target, key) => key in target ? target[key as keyof typeof target] : () => {} }) as unknown as CliSessionLifecycleCompositionDeps
  await createCliSessionLifecycleComposition(deps).recoverAttachedSessionAfterKernelRestart()
  assert.deepEqual({ subscriptions, repairs, recovered, retries }, { subscriptions: 2, repairs: 2, recovered: 1, retries: 1 })
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
    onKernelTransportClosed: () => {},
    handleSessionUnavailable: (message: string) => { recoveries.push(message) },
  } as unknown as CliSessionLifecycleCompositionDeps

  await createCliSessionLifecycleComposition(deps).syncKernelEventSubscription()

  assert.equal(recoveries.length, 1)
  assert.match(recoveries[0] ?? "", /does not belong to session/)
})
