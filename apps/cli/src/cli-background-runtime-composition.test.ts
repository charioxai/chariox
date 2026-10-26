import assert from "node:assert/strict"
import test from "node:test"
import { createCliBackgroundRuntimeComposition } from "./cli-background-runtime-composition.js"
import { makeSession } from "./command-actions-test-support.js"
import { createKernelEventSubscriptionController } from "./kernel-event-subscription-controller.js"

// MP-08/MP-10: exercise production recovery callbacks, not just the event guard.
for (const recovery of ["resync", "session-unavailable"] as const) {
  test(`${recovery} cannot clear a pending binding outage or restart its display deadline`, async t => {
    t.mock.timers.enable({ apis: ["setTimeout"] })
    let disconnected = false
    let repairInFlight = false
    let finishRead!: () => void
    let receiveEvent!: (event: unknown) => void
    const state = makeSession()
    const statuses: string[] = []
    const subscription = createKernelEventSubscriptionController({
      kernelConnected: () => true, supportsKernelEventStream: () => true,
      getAttachment: () => ({ id: "attachment-1" }), getSessionId: () => state.id,
      subscribeToWaitingRoomInventory: async () => {},
      subscribeToKernelEvents: async () => { repairInFlight = true; throw new Error("subscription rejected") },
      onEvaluate: () => {}, onWaitingRoomSubscribed: () => {}, onSessionSubscribed: () => {},
      onWaitingRoomSubscriptionFailed: () => {}, onSessionSubscriptionFailed: () => {},
    })
    const bindingReady = () => subscription.state().scope === "session"
    const ensureBinding = async () => {
      await subscription.sync()
      if (!bindingReady()) throw new Error("Session event binding has not recovered")
    }
    const noOp = new Proxy(() => undefined, { get: () => noOp })
    const deps = new Proxy({
      supportsKernelEventStream: true,
      client: { onKernelEvent: (callback: typeof receiveEvent) => { receiveEvent = callback; return () => {} } },
      closingStateController: { isClosing: () => false, markClosing: () => {} },
      isAttached: () => true, sessionState: () => state,
      attachmentState: () => ({ id: "attachment-1" }), providerRunState: () => null,
      daemonDisconnected: () => disconnected, setDaemonDisconnected: (value: boolean) => { disconnected = value },
      setStatusLine: (value: string) => statuses.push(value),
      sessionBindingRepairInFlight: () => repairInFlight,
      currentSessionEventBindingReady: bindingReady,
      syncValidatedKernelEventSubscription: ensureBinding,
      syncKernelEventSubscription: subscription.sync,
      kernelEventSubscriptionController: subscription,
      getSessionState: async () => {
        if (recovery === "resync") await new Promise<void>(resolve => { finishRead = resolve })
        return state
      },
      attachToSession: async () => ({ id: "attachment-1" }),
      catchUpAttachedSession: async () => {}, refreshAgentPanes: async () => {},
      shouldRefreshAgentPanesForSessionChange: () => false,
      scheduleInterval: () => 1,
    }, { get: (target, key) => Reflect.get(target, key) ?? noOp })
    const runtime = createCliBackgroundRuntimeComposition(deps as never)
    if (recovery === "resync") {
      receiveEvent({ event: "replay_gap" })
      await new Promise<void>(resolve => setImmediate(resolve))
    }
    runtime.applyKernelTransportClosed("event lane closed")
    repairInFlight = true
    const statusCount = statuses.length
    t.mock.timers.tick(5000)
    if (recovery === "resync") finishRead()
    else receiveEvent({ event: "session_unavailable", message: "Session unavailable." })
    await new Promise<void>(resolve => setImmediate(resolve))
    assert.equal(disconnected, true)
    assert.equal(statuses.slice(statusCount).some(status => status.includes("Connected")), false)
    runtime.applyKernelTransportClosed("retry failed")
    t.mock.timers.tick(5000)
    assert.match(statuses.at(-1) ?? "", /Disconnected.*last known/)
  })
}
