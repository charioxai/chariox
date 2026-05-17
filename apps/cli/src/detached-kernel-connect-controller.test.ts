import assert from "node:assert/strict"
import test from "node:test"

import { createDetachedKernelConnectController } from "./detached-kernel-connect-controller.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"

test("detached kernel connect hydrates catalogs, marks connected, and refreshes waiting room", async () => {
  const harness = createHarness()

  await harness.controller.connect()

  assert.deepEqual(harness.calls, [
    "info:connecting detached cli to configured kernel endpoint",
    "flash:info:connecting to kernel...",
    "getProviderCatalog",
    "getProviderCommandCatalogs",
    "invalidateWaitingRoomInventory",
    "setProviderCatalog",
    "setProviderCommandCatalogs",
    "setKernelConnected:true",
    "setDaemonDisconnected:false",
    "refreshWaitingRoomData",
    "flash:info:connected to kernel",
  ])
})

test("detached kernel connect propagates catalog failures before mutating connection state", async () => {
  const harness = createHarness({
    getProviderCatalog: async () => {
      harness.calls.push("getProviderCatalog")
      throw new Error("catalog down")
    },
  })

  await assert.rejects(() => harness.controller.connect(), /catalog down/)

  assert.deepEqual(harness.calls, [
    "info:connecting detached cli to configured kernel endpoint",
    "flash:info:connecting to kernel...",
    "getProviderCatalog",
    "getProviderCommandCatalogs",
  ])
})

function createHarness(options: {
  getProviderCatalog?: () => Promise<ReturnType<typeof fallbackProviderCatalog>>
} = {}) {
  const calls: string[] = []
  const controller = createDetachedKernelConnectController({
    logInfo: (message) => {
      calls.push(`info:${message}`)
    },
    flashFooter: (message, tone) => {
      calls.push(`flash:${tone}:${message}`)
    },
    getProviderCatalog: options.getProviderCatalog ?? (async () => {
      calls.push("getProviderCatalog")
      return fallbackProviderCatalog()
    }),
    getProviderCommandCatalogs: async () => {
      calls.push("getProviderCommandCatalogs")
      return fallbackProviderCommandCatalogs()
    },
    invalidateWaitingRoomInventory: () => {
      calls.push("invalidateWaitingRoomInventory")
    },
    setProviderCatalog: () => {
      calls.push("setProviderCatalog")
    },
    setProviderCommandCatalogs: () => {
      calls.push("setProviderCommandCatalogs")
    },
    setKernelConnected: (next) => {
      calls.push(`setKernelConnected:${next}`)
    },
    setDaemonDisconnected: (next) => {
      calls.push(`setDaemonDisconnected:${next}`)
    },
    refreshWaitingRoomData: async () => {
      calls.push("refreshWaitingRoomData")
    },
  })

  return { calls, controller }
}
