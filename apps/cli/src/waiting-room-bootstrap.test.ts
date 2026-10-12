// MP-08 / MP-11: independent startup reads cannot starve one another.
import assert from "node:assert/strict"
import test from "node:test"
import { bootstrapWaitingRoom } from "./waiting-room-bootstrap.js"
test("MP-08 / MP-11 catalog loads when configured defaults are refused", async () => {
  let reads = 0
  const state = bootstrapWaitingRoom({} as any, {model:"default", effort:""} as any, {} as any, {
    getConfiguredProviderLaunchDefaults: async () => { throw new Error("defaults denied") },
    getProviderCatalog: async () => { reads++; return {providers:[]} },
    getProviderCommandCatalogs: async () => [], getTerminalCommandCatalog: async () => null,
  } as any)
  await assert.rejects(state.deferred!.waitingRoomDefaults!, /defaults denied/)
  await state.deferred!.providerCatalog
  assert.equal(reads, 1)
})
