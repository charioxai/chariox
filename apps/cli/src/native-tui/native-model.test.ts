import assert from "node:assert/strict"
import test from "node:test"

import type { LocalIpcClient } from "../ipc.js"
import { resolveNativeModel } from "./native-model.js"

function kernelServing(models: string[], defaultModel?: string): LocalIpcClient {
  return {
    send: async () => ({
      ProviderCatalog: {
        catalog: {
          all: [{ id: "codex", name: "Codex", models: Object.fromEntries(models.map((id) => [id, { id, name: id }])) }],
          default: defaultModel ? { codex: defaultModel } : {},
          connected: ["codex"],
        },
      },
    }),
  } as unknown as LocalIpcClient
}

test("a native Codex launch with no --model runs the model Codex names as its default", async () => {
  const client = kernelServing(["gpt-6-luna", "gpt-6.1-sol"], "gpt-6.1-sol")
  assert.equal(await resolveNativeModel(client, "codex", "Codex", undefined), "gpt-6.1-sol")
})

test("a model Codex no longer serves is refused with the models it serves", async () => {
  const client = kernelServing(["gpt-6-luna", "gpt-6.1-sol"], "gpt-6.1-sol")
  await assert.rejects(
    resolveNativeModel(client, "codex", "Codex", "gpt-5.4-mini"),
    /Codex does not serve model `gpt-5\.4-mini`.*--model.*gpt-6-luna, gpt-6\.1-sol/,
  )
})

test("a model Codex serves is kept", async () => {
  const client = kernelServing(["gpt-6-luna"], "gpt-6-luna")
  assert.equal(await resolveNativeModel(client, "codex", "Codex", "gpt-6-luna"), "gpt-6-luna")
})

test("a provider that reports no models says how to sign in", async () => {
  await assert.rejects(resolveNativeModel(kernelServing([]), "codex", "Codex", undefined), /Codex reports no models.*codex login/)
})

test("an OpenCode model that needs an account is refused with the account to add", async () => {
  const client = {
    send: async () => ({ ProviderCatalog: { catalog: { all: [{ id: "opencode", name: "OpenCode", models: { "big-pickle": { id: "big-pickle", name: "Big Pickle" } } }], default: { opencode: "big-pickle" }, connected: ["opencode"] } } }),
  } as unknown as LocalIpcClient
  await assert.rejects(
    resolveNativeModel(client, "opencode", "OpenCode", "gpt-5.6-terra-pro", "Add the model's provider in Provider Accounts."),
    /OpenCode model `gpt-5\.6-terra-pro` needs an account that is not connected\. Add the model's provider in Provider Accounts\. Models that run without one: big-pickle/,
  )
  assert.equal(await resolveNativeModel(client, "opencode", "OpenCode", "big-pickle", "x"), "big-pickle")
})
