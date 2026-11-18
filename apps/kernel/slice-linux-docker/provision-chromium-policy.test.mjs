import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"
import { managedPolicy } from "./docker/kernel-browser-policy.mjs"

const provisioner = new URL("./provision-linux-docker-slice.sh", import.meta.url)

// Chromium's password manager would copy Vault-filled passwords into the slice
// profile and raise its Save password bubble over the page after a login.
test("slice Chromium policy turns off the password manager and autofill like the kernel browser", async () => {
  const source = await readFile(provisioner, "utf8")
  const body = source.match(/configure_chromium_browser_policy\(\) \{[\s\S]*?<<'JSON'\n([^\n]*)\nJSON/)
  assert.ok(body, "slice Chromium policy heredoc is present")
  const policy = JSON.parse(body[1].replaceAll('\\"', '"'))
  for (const key of ["PasswordManagerEnabled", "AutofillAddressEnabled", "AutofillCreditCardEnabled", "BrowserSignin"]) {
    assert.equal(policy[key], managedPolicy[key], key)
  }
})
