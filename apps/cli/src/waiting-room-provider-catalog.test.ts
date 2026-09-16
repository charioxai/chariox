import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "./ipc.js"
import { catalogModelOptions, selectConfiguredModel, type ProviderCatalog } from "./provider-catalog.js"
import { loadProviderCatalogForKernel } from "./waiting-room-provider-catalog.js"

test("target-kernel catalog hydration keeps discovered model efforts and qualified routing", async () => {
  const requests: unknown[] = []
  const catalog = await loadProviderCatalogForKernel(
    clientReturning({
      ProviderCatalog: {
        catalog: {
          all: [
            {
              id: "opencode",
              name: "OpenCode Zen",
              models: {
                "authoritative-model": {
                  id: "authoritative-model",
                  name: "Authoritative model",
                  status: "active",
                  variants: { medium: {}, high: {} },
                },
              },
            },
            {
              id: "opencode-go",
              name: "OpenCode Go",
              models: {
                "authoritative-model": {
                  id: "authoritative-model",
                  name: "Authoritative model",
                  status: "active",
                  variants: { low: {}, max: {} },
                },
              },
            },
          ],
          default: {
            opencode: "authoritative-model",
            "opencode-go": "authoritative-model",
          },
          connected: ["opencode", "opencode-go"],
        } satisfies ProviderCatalog,
      },
    }, requests),
    undefined,
    { providerId: "opencode", accountProfileId: "managed-account" },
  )

  assert.deepEqual(requests, [{
    GetProviderCatalog: {
      provider: "opencode",
      account_profiles: { opencode: "managed-account" },
      execution_location: { kind: "local" },
    },
  }])
  assert.equal(catalog.source, "daemon")

  const options = catalogModelOptions(catalog, "opencode")
  assert.deepEqual(options.map((option) => option.id).sort(), [
    "opencode-go/authoritative-model",
    "opencode/authoritative-model",
  ])
  assert.deepEqual(
    options.find((option) => option.id === "opencode/authoritative-model")?.variants,
    ["medium", "high"],
  )
  assert.deepEqual(
    options.find((option) => option.id === "opencode-go/authoritative-model")?.variants,
    ["low", "max"],
  )
  assert.equal(
    selectConfiguredModel(catalog, "opencode-go/authoritative-model", "opencode")?.providerId,
    "opencode-go",
  )
})

test("target-kernel catalog hydration surfaces discovery failure without fallback models", async () => {
  await assert.rejects(
    loadProviderCatalogForKernel(clientRejecting(new Error("target catalog unavailable")), undefined, {
      providerId: "codex",
      accountProfileId: "managed-account",
    }),
    /target catalog unavailable/,
  )
})

function clientReturning(response: Record<string, unknown>, requests: unknown[]): LocalIpcClient {
  return {
    send: async (request: unknown) => {
      requests.push(request)
      return response
    },
  } as unknown as LocalIpcClient
}

function clientRejecting(error: Error): LocalIpcClient {
  return {
    send: async () => {
      throw error
    },
  } as unknown as LocalIpcClient
}
