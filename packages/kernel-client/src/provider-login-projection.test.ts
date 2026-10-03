import assert from "node:assert/strict"
import test from "node:test"
import { providerLoginLines, safeProviderLoginUrl } from "./provider-login-projection.js"

test("MP-08/MP-10/MP-11 login projection identifies receiving kernel and renders official challenge", () => {
  const lines = providerLoginLines({kernel_id: "worker", login: {provider: "codex", account_profile: "work",
    login_kind: "chatgptDeviceCode", verification_url: "http://127.0.0.1/device", user_code: "SYNTHETIC"},
    terminal_output_base64: Buffer.from("\u001b[31mOfficial CLI prompt\u001b[0m").toString("base64")})
  assert.deepEqual(lines, ["Login runs on worker.", "http://127.0.0.1/device", "Code: SYNTHETIC", "Official CLI prompt"])
  assert.equal(safeProviderLoginUrl("javascript:alert(1)"), null)
  assert.equal(safeProviderLoginUrl("https://user:password@provider.test/"), null)
})
