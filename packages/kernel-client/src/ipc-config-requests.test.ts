import assert from "node:assert/strict"
import test from "node:test"

import { unlockCredentialVaultRequest } from "./ipc-config-requests.js"
import { parseVaultUnlockDurationArgument, vaultUnlockDefaultFromStatus } from "./vault-unlock-duration.js"

test("the Vault unlock request carries the passphrase and the chosen duration", () => {
  assert.deepEqual(unlockCredentialVaultRequest("typed by the owner", { kind: "until_kernel_stops" }, "terminal"), { UnlockCredentialVault: { passphrase: "typed by the owner", duration: { kind: "until_kernel_stops" }, surface: "terminal" } })
})

test("an unlock length is once, kernel, or minutes and hours", () => {
  assert.deepEqual(["once", "kernel", "45", "30m", "2h", "0", "forever", "-5m", ""].map(token => parseVaultUnlockDurationArgument(token)), [
    { kind: "once" }, { kind: "until_kernel_stops" }, { kind: "minutes", minutes: 45 }, { kind: "minutes", minutes: 30 }, { kind: "minutes", minutes: 120 }, null, null, null, null,
  ])
})

test("the preselected length comes from the kernel's status and is never guessed", () => {
  assert.deepEqual(vaultUnlockDefaultFromStatus({ CredentialVaultStatus: { status: { unlocked: false }, unlock: { durations: [], default: { kind: "minutes", minutes: 60 } } } }), { kind: "minutes", minutes: 60 })
  assert.throws(() => vaultUnlockDefaultFromStatus({ CredentialVaultStatus: { status: { unlocked: false } } }), /did not say/)
})
