import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const providerAuthControl = await readFile(
  new URL("../apps/kernel/src/runtime/provider_auth_control.rs", import.meta.url),
  "utf8",
)

test("terminal provider login uses the shared managed account launch boundary", () => {
  const start = providerAuthControl.indexOf("async fn start_terminal_provider_auth(")
  const end = providerAuthControl.indexOf("\nfn terminal_provider_auth_args(", start)
  assert.notEqual(start, -1)
  assert.notEqual(end, -1)
  const implementation = providerAuthControl.slice(start, end)

  assert.match(
    implementation,
    /managed_isolated_utility_launch\(/,
    "interactive login must use the same account-path projection as post-login validation",
  )
  assert.match(
    implementation,
    /provider_auth_env_vars\(provider\)/,
    "the managed launch must continue removing ambient provider credentials",
  )
  assert.doesNotMatch(
    implementation,
    /program:\s*program\.to_string_lossy\(\)\.to_string\(\)/,
    "interactive login must not bypass the managed provider launch boundary",
  )
})

test("terminal provider login prepares isolation before recording a running login", () => {
  const start = providerAuthControl.indexOf("async fn start_terminal_provider_auth(")
  const end = providerAuthControl.indexOf("\nfn terminal_provider_auth_args(", start)
  assert.notEqual(start, -1)
  assert.notEqual(end, -1)
  const implementation = providerAuthControl.slice(start, end)

  const prepareLaunch = implementation.indexOf("managed_isolated_utility_launch(")
  const insertRunningLogin = implementation.indexOf(
    "provider_login_process_store().insert(",
  )
  assert.notEqual(prepareLaunch, -1)
  assert.notEqual(insertRunningLogin, -1)
  assert.ok(
    prepareLaunch < insertRunningLogin,
    "isolation preparation failures must not leave an orphaned running login record",
  )
})
