import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "./ipc.js"
import { getProviderCatalog } from "./provider-api.js"
import type { ProviderCatalog } from "./provider-catalog.js"

test("getProviderCatalog marks daemon catalog source", async () => {
  const catalog = await getProviderCatalog(clientReturning({
    ProviderCatalog: {
      catalog: {
        all: [],
        default: {},
        connected: [],
      } satisfies ProviderCatalog,
    },
  }))

  assert.equal(catalog.source, "daemon")
})

test("getProviderCatalog marks local fallback when daemon lookup fails", async () => {
  const warnings: Array<Record<string, unknown>> = []
  const catalog = await getProviderCatalog(clientRejecting(new Error("catalog down")), {
    warn: (_message: string, fields?: Record<string, unknown>) => {
      warnings.push(fields ?? {})
    },
  } as never)

  assert.equal(catalog.source, "local_fallback")
  assert.match(catalog.unavailable_reason ?? "", /catalog down/)
  assert.match(String(warnings[0]?.error ?? ""), /catalog down/)
})

function clientReturning(response: Record<string, unknown>): LocalIpcClient {
  return {
    send: async () => response,
  } as unknown as LocalIpcClient
}

function clientRejecting(error: Error): LocalIpcClient {
  return {
    send: async () => {
      throw error
    },
  } as unknown as LocalIpcClient
}
