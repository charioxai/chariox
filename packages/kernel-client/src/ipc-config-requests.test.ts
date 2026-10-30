import assert from "node:assert/strict"
import test from "node:test"

import { unlockCredentialVaultRequest } from "./ipc-config-requests.js"

test("the Vault unlock request carries the passphrase and nothing else", () => {
  assert.deepEqual(unlockCredentialVaultRequest("typed by the owner"), { UnlockCredentialVault: { passphrase: "typed by the owner" } })
})
