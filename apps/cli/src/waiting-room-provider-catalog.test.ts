import assert from "node:assert/strict"
import test from "node:test"

import { buildModelItems } from "./command-center-dynamic-items.js"
import type { LocalIpcClient } from "./ipc.js"
import { catalogModelOptions, selectConfiguredModel, type ProviderCatalog } from "./provider-catalog.js"
import { createWaitingRoomState } from "./waiting-room-state.js"
import { waitingRoomRows } from "./waiting-room-rows.js"
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

test("recorded OpenCode native discovery reaches selectors without inventing Go entitlement", async () => {
  const catalog = await loadProviderCatalogForKernel(
    clientReturning({
      ProviderCatalog: { catalog: recordedOpenCodeNativeCatalog() },
    }, []),
    undefined,
    { providerId: "opencode", accountProfileId: "default" },
  )

  assert.equal(catalog.source, "daemon")
  assert.deepEqual(catalog.connected, ["opencode"])

  const entitledZenOptions = catalogModelOptions(catalog, "opencode")
  assert.deepEqual(entitledZenOptions.map((option) => option.id), ["opencode/big-pickle"])

  const state = createWaitingRoomState([], catalog, "opencode", "opencode/big-pickle", "")
  assert.equal(
    waitingRoomRows(state, [], catalog).find((row) => row.id === "model")?.value,
    "OpenCode Zen Big Pickle",
  )
  assert.deepEqual(
    buildModelItems("/model big", {
      providerCatalog: catalog,
      currentProvider: "opencode",
      currentModel: "opencode/big-pickle",
      currentVariant: "",
    }).map((item) => ({ label: item.label, value: item.value })),
    [{ label: "OpenCode Zen Big Pickle", value: "opencode/big-pickle" }],
  )

  // The installed server reports Go in `all` but not `connected` without a
  // Go entitlement. A separate connected fixture only checks rendering and
  // routing for that entitlement; it does not make the live account claim it.
  const entitledBoth = { ...catalog, connected: ["opencode", "opencode-go"] }
  const goOption = catalogModelOptions(entitledBoth, "opencode")
    .find((option) => option.id === "opencode-go/gpt-5.6-luna")
  assert.deepEqual(
    goOption && { id: goOption.id, providerName: goOption.providerName, variants: goOption.variants },
    {
      id: "opencode-go/gpt-5.6-luna",
      providerName: "OpenCode Go",
      variants: ["none", "low", "medium", "high", "xhigh", "max"],
    },
  )
  assert.deepEqual(
    buildModelItems("/model luna", {
      providerCatalog: entitledBoth,
      currentProvider: "opencode",
      currentModel: "opencode-go/gpt-5.6-luna",
      currentVariant: "max",
    }).map((item) => ({ label: item.label, value: item.value })),
    [{ label: "OpenCode Go GPT-5.6 Luna", value: "opencode-go/gpt-5.6-luna" }],
  )
})

function recordedOpenCodeNativeCatalog(): ProviderCatalog {
  // These native ids, names, defaults, and variants were recorded from the
  // installed OpenCode 1.18.23 `/provider` response. Only the two native
  // plan entries needed by this selector regression are retained here.
  return {
    all: [
      {
        id: "opencode",
        name: "OpenCode Zen",
        models: {
          "big-pickle": {
            id: "big-pickle",
            name: "Big Pickle",
            status: "active",
            variants: {},
          },
        },
      },
      {
        id: "opencode-go",
        name: "OpenCode Go",
        models: {
          "gpt-5.6-luna": {
            id: "gpt-5.6-luna",
            name: "GPT-5.6 Luna",
            status: "active",
            variants: {
              none: {},
              low: {},
              medium: {},
              high: {},
              xhigh: {},
              max: {},
            },
          },
        },
      },
    ],
    default: {
      opencode: "big-pickle",
      "opencode-go": "gpt-5.6-luna",
    },
    connected: ["opencode"],
  }
}

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
