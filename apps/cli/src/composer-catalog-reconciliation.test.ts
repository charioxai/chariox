import assert from "node:assert/strict"
import test from "node:test"
import { reconcileComposerCatalogs } from "./composer-catalog-reconciliation.js"
import { fallbackProviderCatalog } from "./provider-catalog.js"
import { fallbackProviderCommandCatalogs } from "./provider-command-catalog.js"
const hashes = { provider_accounts: "a".repeat(64), provider_catalog: "b".repeat(64), provider_commands: "c".repeat(64), terminal_commands: "sha256:" + "d".repeat(64) }
function setup() {
  const calls: string[] = []
  const cached = { providerCatalog: fallbackProviderCatalog(), providerCommandCatalogs: fallbackProviderCommandCatalogs(), terminalCommandCatalog: { revision: hashes.terminal_commands, nodes: [] }, catalogRevisions: hashes }
  const reads = { revisions: async () => { calls.push("revision"); return hashes as typeof hashes | null }, provider: async () => { calls.push("provider"); return fallbackProviderCatalog() }, providerCommands: async () => { calls.push("providerCommands"); return fallbackProviderCommandCatalogs() }, terminalCommands: async () => { calls.push("terminalCommands"); return { revision: hashes.terminal_commands, nodes: [] } } }
  return { cached, reads, calls }
}
test("MP-08 / MP-10 / MP-11 unchanged catalog reconciliation transfers only the small revision header", async () => {
  const {cached, reads, calls} = setup(); const value = await reconcileComposerCatalogs(cached, reads)
  assert.deepEqual(calls, ["revision"]); assert.equal(value.providerCatalog, cached.providerCatalog); assert.equal(value.terminalCommandCatalog, cached.terminalCommandCatalog)
})
test("MP-08 / MP-10 / MP-11 a changed provider hash fetches only that scoped catalog", async () => {
  const {cached, reads, calls} = setup(); reads.revisions = async () => { calls.push("revision"); return {...hashes, provider_catalog: "e".repeat(64)} }
  const value = await reconcileComposerCatalogs(cached, reads)
  assert.deepEqual(calls, ["revision", "provider"]); assert.equal(value.providerCommandCatalogs, cached.providerCommandCatalogs); assert.equal(value.catalogRevisions?.provider_catalog, "e".repeat(64))
})
test("MP-08 / MP-10 / MP-11 an older kernel reconciles through ordinary full requests", async () => {
  const {cached, reads, calls} = setup(); reads.revisions = async () => null
  await reconcileComposerCatalogs(cached, reads); assert.deepEqual(calls.sort(), ["provider", "providerCommands", "terminalCommands"])
})
test("MP-08 / MP-10 / MP-11 revision auth failures retain the previous view without fetching other inventories", async () => {
  const {cached, reads, calls} = setup(); reads.revisions = async () => { throw new Error("denied") }
  await assert.rejects(reconcileComposerCatalogs(cached, reads), /denied/); assert.deepEqual(calls, [])
})
