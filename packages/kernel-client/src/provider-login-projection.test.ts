import assert from "node:assert/strict"
import test from "node:test"
import { providerLoginLines, providerLoginProblem, providerLoginView, safeProviderLoginUrl, providerLoginStatusInteraction, providerLoginInteractionReplyRequest } from "./provider-login-projection.js"
import type { RuntimeInteraction } from "./kernel-types-runtime.js"

test("MP-08/MP-10/MP-11 login projection identifies receiving kernel and renders official challenge", () => {
  const lines = providerLoginLines({kernel_id: "worker", login: {provider: "codex", account_profile: "work",
    login_kind: "chatgptDeviceCode", verification_url: "http://127.0.0.1/device", user_code: "SYNTHETIC"},
    terminal_output_base64: Buffer.from("\u001b[31mOfficial CLI prompt\u001b[0m").toString("base64")})
  assert.deepEqual(lines, ["Login runs on worker.", "http://127.0.0.1/device", "Code: SYNTHETIC", "Official CLI prompt"])
  assert.equal(safeProviderLoginUrl("javascript:alert(1)"), null)
  assert.equal(safeProviderLoginUrl("https://user:password@provider.test/"), null)
})

test("MP-08/MP-11 login view numbers the steps and names the failure the provider or kernel reported", () => {
  const projection = {kernel_id: "fleet", login: {provider: "claude", account_profile: "claude-1-abc", login_kind: "terminal_setup_token",
    auth_url: "https://claude.ai/oauth/authorize?code=true"}, terminal_output_base64: ""}
  assert.deepEqual(providerLoginView(projection, "provider-response", "work"), {
    title: "Sign in to Claude · work", url: "https://claude.ai/oauth/authorize?code=true",
    steps: ["Authorize Chariox in your browser.", "Paste the code below and press Enter."], takesCode: true})
  assert.equal(providerLoginView({...projection, login: {...projection.login, auth_url: "javascript:alert(1)"}}, "provider-response").url, null)
  assert.equal(providerLoginView(projection, "passphrase").takesCode, false)
  const output = (text: string) => Buffer.from(text).toString("base64")
  assert.equal(providerLoginProblem(output("\u001b[31mOAuth error: Request failed with status code 400\u001b[0m\nPress Enter to retry.")), "OAuth error: Request failed with status code 400")
  assert.equal(providerLoginProblem(output("Paste code here if prompted >")), null)
  assert.equal(providerLoginProblem(output("Paste code here if prompted >"), true), "Paste code here if prompted >")
})

test("MP-08/MP-11 a provider-response terminal choice is not necessarily an OAuth code", () => {
  const projection = { kernel_id: "worker", login: { provider: "opencode", account_profile: "work", login_kind: "terminal" }, terminal_output_base64: "" }
  assert.equal(providerLoginView(projection, "provider-response").takesCode, false)
})

test("MP-08/MP-11 sessionless replies preserve the kernel's code and Vault choices on one login", () => {
  const projection = { kernel_id: "kernel", login: { provider: "claude", account_profile: "fresh", login_id: "login", login_kind: "terminal_setup_token" }, terminal_output_base64: "" }
  const template: RuntimeInteraction = { id: "login", agent_id: "provider-account:claude:fresh", kind: "choice", level: "critical",
    title: "Unlock Chariox Vault", message: "Enter the Vault passphrase", requested_at_ms: 42,
    choices: [{ id: "cancel", label: "Cancel", reply: "cancel" }], custom_choice: { id: "passphrase", label: "Vault passphrase", input_kind: "secret" } }
  const status = { provider: "claude", account_profile: "fresh", login_id: "login", state: "running" as const,
    interaction: template, started_at_ms: 42, updated_at_ms: 45, terminal_output_base64: "" }
  const interaction = providerLoginStatusInteraction(status, projection)!
  assert.deepEqual(interaction, { ...template, provider_login: projection })
  for (const choiceId of ["passphrase", "provider-response"]) {
    const current = { ...interaction, custom_choice: { ...interaction.custom_choice!, id: choiceId } }
    const request = providerLoginInteractionReplyRequest(current, choiceId, "CaseSensitive Ω")
    assert.deepEqual(request, { SendProviderLoginInput: { login_id: "login", data_base64: Buffer.from("CaseSensitive Ω\r").toString("base64") } })
  }
  assert.deepEqual(providerLoginInteractionReplyRequest(interaction, "cancel", null), { CancelProviderLogin: { login_id: "login" } })
  assert.throws(() => providerLoginInteractionReplyRequest(interaction, "invented", "value"), /unavailable/)
  assert.equal(providerLoginStatusInteraction({ ...status, state: "cancelled" }, projection), null)
})


test("MP-08/MP-11 detached setup-token Vault phases suppress the previous terminal screen like the attached workflow", () => {
  const interaction = { id: "login", agent_id: "provider-account:claude:fresh", kind: "choice", level: "critical", title: "Unlock Chariox Vault", message: "Enter the Vault passphrase", requested_at_ms: 42, choices: [], custom_choice: { id: "passphrase", label: "Vault passphrase", input_kind: "secret" } }
  const projection = { kernel_id: "kernel", login: { provider: "claude", account_profile: "fresh", login_id: "login", login_kind: "terminal_setup_token" }, terminal_output_base64: Buffer.from("Old authorization screen should stay out of the vault prompt").toString("base64") }
  const status = { provider: "claude", account_profile: "fresh", login_id: "login", state: "running" as const, interaction, started_at_ms: 42, updated_at_ms: 45, terminal_output_base64: projection.terminal_output_base64 }
  assert.equal(providerLoginStatusInteraction(status, projection)?.provider_login?.terminal_output_base64, "")
})
