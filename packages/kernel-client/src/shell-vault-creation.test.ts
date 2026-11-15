import assert from "node:assert/strict"
import test from "node:test"
import type { ParsedShellCommand } from "./shell-core.js"
import { executeCredentialCommand } from "./shell-config-command.js"

const parsed = (action: string): ParsedShellCommand => ({ kind: "shared", normalized: `credential vault ${action}`, command: "credential", args: ["vault", action], raw: `credential vault ${action}` })

test("a fresh terminal creates its Vault with hidden confirmed input and no session", async () => {
  const requests: Record<string, unknown>[] = [], prompts: string[] = []
  const result = await executeCredentialCommand(parsed("create"), {
    client: { send: async request => { requests.push(request); return { CredentialVaultUnlocked: { action: "created", status: { unlocked: true } } } } },
    readSecret: async prompt => { prompts.push(prompt); return "Chosen Passphrase 🔑" },
  })
  assert.equal(result.ok, true)
  assert.equal(result.message, "vault created")
  assert.deepEqual(prompts, ["New Vault passphrase: ", "Confirm Vault passphrase: "])
  assert.deepEqual(requests, [{ CreateCredentialVault: { origin: { kind: "passphrase", passphrase: "Chosen Passphrase 🔑" } } }])
  assert.ok(!JSON.stringify(result).includes("Chosen Passphrase"))
})

test("a missing Vault has actionable status without requesting a prompt", async () => {
  const requests: Record<string, unknown>[] = []
  const result = await executeCredentialCommand(parsed("status"), {
    client: { send: async request => { requests.push(request); return { CredentialVaultStatus: { status: { unlocked: false, awaiting_passphrase: true } } } } },
    readSecret: async () => { throw Error("status must not prompt") },
  })
  assert.equal(result.message, "vault needs a passphrase; run /credential vault create")
  assert.deepEqual(requests, [{ GetCredentialVaultStatus: null }])
})

for (const input of [[], [""], ["new", "different"]]) {
  test(`creation refuses absent hidden input, empty input or mismatch: ${input.length}`, async () => {
    const values = [...input]
    const result = await executeCredentialCommand(parsed("create"), {
      client: { send: async () => { throw Error("must not create") } },
      ...(input.length ? { readSecret: async () => values.shift()! } : {}),
    })
    assert.equal(result.ok, false)
    assert.match(result.message!, input.length === 0 ? /hidden input/ : input[0] === "" ? /must not be empty/ : /do not match/)
  })
}
