import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "./ipc.js"
import { getProviderCatalog, getProviderCommandCatalogs } from "./provider-api.js"
import type { ProviderCatalog } from "./provider-catalog.js"
import type { ProviderCommandCatalogs } from "./provider-command-catalog.js"

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

test("getProviderCommandCatalogs marks daemon catalog source", async () => {
  const catalogs = await getProviderCommandCatalogs(clientReturning({
    ProviderCommandCatalogs: {
      catalogs: {
        opencode: {
          provider: "opencode",
          source: "shipped",
          discovery: "none",
          commands: [],
        },
        codex: {
          provider: "codex",
          source: "shipped",
          discovery: "none",
          commands: [],
        },
        "claude-headless": {
          provider: "claude-headless",
          source: "shipped",
          discovery: "none",
          commands: [],
        },
        "claude-p": {
          provider: "claude-p",
          source: "shipped",
          discovery: "none",
          commands: [],
        },
      } satisfies ProviderCommandCatalogs,
    },
  }))

  assert.equal(catalogs.codex.catalog_source, "daemon")
})

test("getProviderCommandCatalogs marks local fallback when daemon lookup fails", async () => {
  const warnings: Array<Record<string, unknown>> = []
  const catalogs = await getProviderCommandCatalogs(clientRejecting(new Error("command catalog down")), {
    warn: (_message: string, fields?: Record<string, unknown>) => {
      warnings.push(fields ?? {})
    },
  } as never)

  assert.equal(catalogs.codex.catalog_source, "local_fallback")
  assert.match(catalogs.codex.unavailable_reason ?? "", /command catalog down/)
  assert.match(String(warnings[0]?.error ?? ""), /command catalog down/)
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
