import assert from "node:assert/strict"
import test from "node:test"
import { getClientCatalogRevisions } from "./client-catalog-api.js"
import type { LocalIpcClient } from "./ipc.js"
test("MP-08 / MP-10 / MP-11 catalog revision compatibility catches only unknown request variants", async () => {
  const client = { send: async () => { throw new Error("unknown variant `GetClientCatalogRevisions`") } } as unknown as LocalIpcClient
  assert.equal(await getClientCatalogRevisions(client), null)
  client.send = async () => { throw new Error("401 Unauthorized") }
  await assert.rejects(getClientCatalogRevisions(client), /401/)
})
test("MP-08 / MP-10 / MP-11 scoped catalog request accepts the existing prefixed terminal revision", async () => {
  let request: unknown
  const revisions = { provider_accounts: "a".repeat(64), provider_catalog: "b".repeat(64), provider_commands: "c".repeat(64), terminal_commands: "sha256:" + "d".repeat(64) }
  const client = { send: async (r: unknown) => { request = r; return { ClientCatalogRevisions: { revisions } } } } as unknown as LocalIpcClient
  assert.deepEqual(await getClientCatalogRevisions(client, {provider:"codex",accountProfile:"work"}), revisions)
  assert.deepEqual(request, {GetClientCatalogRevisions:{provider:"codex",account_profiles:{codex:"work"},execution_location:{kind:"local"}}})
})

test("MP-08 / MP-10 / MP-11 failed full catalog reads cannot be cached as matching server revisions", async () => {
  const {getProviderCatalog,getProviderCommandCatalogs} = await import("./provider-api.js")
  const client = {send:async()=>{throw new Error("connection closed")}} as unknown as LocalIpcClient
  await assert.rejects(getProviderCatalog(client,null,{},false), /connection closed/)
  await assert.rejects(getProviderCommandCatalogs(client,null,false), /connection closed/)
})
