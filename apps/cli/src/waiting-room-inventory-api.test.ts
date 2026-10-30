import assert from "node:assert/strict"
import test from "node:test"
import type { LocalIpcClient } from "./ipc.js"
import { getWaitingRoomInventory } from "./waiting-room-inventory-api.js"

for (const reason of ["Cloud enrollment is required", "Cloud peer directory is unavailable"]) {
  test(`optional login discovery preserves inventory when ${reason}`, async () => {
    const accounts = [{ id: "work", provider: "claude", label: "Work" }]
    const sessions = [{ id: "session", created_at_ms: 1 }]
    const targets = [{ kernel_id: "other" }]
    let discoveries = 0
    const client = { send: async (request: Record<string, unknown>) => {
      if ("GetWaitingRoomPublicSnapshot" in request) return { WaitingRoomPublicSnapshot: { snapshot: {
        schema_version: 1, inventory_version: "v1", structural_version: "s1", activity_revision: "a1",
        relay_status: { connected: true, daemon_id: "local", machine_id: "machine", capabilities: ["provider_account_copy_pull_v1"] },
        provider_accounts: accounts, sessions, remote_kernels: targets,
      } } }
      if ("ListSlices" in request) return { SlicesListed: { slices: [] } }
      if ("ListProviderAccountCopyOffers" in request) { discoveries++; throw new Error(reason) }
      throw new Error(`Unexpected request: ${Object.keys(request).join(",")}`)
    } } as unknown as LocalIpcClient
    const inventory = await getWaitingRoomInventory(client)
    assert.equal(discoveries, 1)
    assert.deepEqual(inventory.providerAccounts, accounts)
    assert.equal(inventory.sessions[0]?.id, "session")
    assert.deepEqual(inventory.remoteKernels, targets)
    assert.deepEqual(inventory.providerAccountCopyOffers, [])
    assert.equal(inventory.providerAccountCopyUnavailable, true)
  })
}
