import assert from "node:assert/strict"
import { test } from "node:test"
import { allocationWorkerBindingDigest, allocationWorkerReleaseMatches } from "../deploy/managed-kernel/allocation-worker-receipt.mjs"

const receipt = () => ({
  schemaVersion: 1, status: "confirmed", allocationId: "worker-1",
  machineId: "worker-machine", kernelId: "worker-kernel", relayPublicKey: "worker-public-key",
  runtimeReleaseDigest: `sha256:${"b".repeat(64)}`,
  homeCaller: { accountId: "account-1", userId: "owner-1", realmId: "realm-1", machineId: "home-machine", kernelId: "home-kernel", relayPublicKey: "home-public-key" },
  confirmedAt: "2026-09-13T11:00:00Z",
})

test("allocation worker binding matches the Rust bootstrap fixture", () => {
  assert.equal(allocationWorkerBindingDigest(receipt()), "sha256:3341e3d7f98f947c0c37ff9923923cffce466d4a632ca6f238439df6e23109c7")
})

test("allocation worker override is bound to the original identity and release", () => {
  const original = receipt()
  const upgraded = `sha256:${"e".repeat(64)}`
  const override = { schemaVersion: 1, kind: "disposable_worker_release", bindingDigest: allocationWorkerBindingDigest(original), runtimeReleaseDigest: upgraded }
  assert.equal(allocationWorkerReleaseMatches(original, null, original.runtimeReleaseDigest), true)
  assert.equal(allocationWorkerReleaseMatches(original, override, upgraded), true)
  assert.equal(allocationWorkerReleaseMatches(original, override, original.runtimeReleaseDigest), false)
  for (const key of ["allocationId", "machineId", "kernelId", "relayPublicKey", "runtimeReleaseDigest"]) {
    const changed = receipt()
    changed[key] = key === "runtimeReleaseDigest" ? upgraded : "changed"
    assert.throws(() => allocationWorkerReleaseMatches(changed, override, upgraded))
  }
  for (const key of Object.keys(original.homeCaller)) {
    const changed = receipt()
    changed.homeCaller[key] = "changed"
    assert.throws(() => allocationWorkerReleaseMatches(changed, override, upgraded))
  }
})

test("allocation worker rejects unconfirmed and malformed receipts", () => {
  for (const patch of [{ status: "exchanged" }, { unexpected: true }, { confirmedAt: "yesterday" }, { schemaVersion: 2 }, { homeCaller: {} }]) {
    assert.throws(() => allocationWorkerBindingDigest({ ...receipt(), ...patch }))
  }
})
