import assert from "node:assert/strict"
import test from "node:test"
import { providerLoginLines, providerLoginProblem, providerLoginView, safeProviderLoginUrl } from "./provider-login-projection.js"

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
