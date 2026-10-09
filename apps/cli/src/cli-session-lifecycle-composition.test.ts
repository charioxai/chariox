import assert from "node:assert/strict"
import test from "node:test"

import {
  createCliSessionLifecycleComposition,
  type CliSessionLifecycleCompositionDeps,
} from "./cli-session-lifecycle-composition.js"

// MP-08 / MP-11: a failed inventory binding must not replace the waiting room.
test("waiting-room subscription failure retains rows and marks them reconnecting", async () => {
  const calls: string[] = []
  const values = {
    client: { subscribeToWaitingRoomInventory: async () => { throw new Error("unable to authenticate data") } },
    options: {}, renderer: {}, appLogger: { error: () => {}, debug: () => {} },
    closingStateController: { isClosing: () => false, setClosing: () => {} },
    supportsKernelEventStream: true,
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
