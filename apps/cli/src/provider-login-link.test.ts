import assert from "node:assert/strict"
import test from "node:test"
import { providerLoginLinkText, providerLoginUrl, providerLoginUrls } from "./provider-login-link.js"
import { handleProviderSlashCommand } from "./provider-command-handlers.js"

const url = `https://claude.ai/oauth/authorize?client_id=fixture&state=${"abc123".repeat(70)}`

test("MP-08/MP-11 authorization URL is one OSC 8 logical line with no layout chrome", () => {
  assert.equal(providerLoginLinkText(url), `\x1b]8;;${url}\x1b\\${url}\x1b]8;;\x1b\\\r\n`)
  assert.deepEqual(providerLoginUrls(`Authorize:\r\n\x1b[32m${url}\x1b[0m\r\n`), [url])
  for (const value of ["file:///etc/passwd", "javascript:alert(1)", "https://a/\x1b]52;bad", "https://u:p@host/", "https://a/\nnext"]) {
    assert.equal(providerLoginUrl(value), null)
    assert.throws(() => providerLoginLinkText(value))
  }
})

test("MP-08 / MP-10 login links stay inline; only a requested login opens the browser", async () => {
  const opened: string[] = []
  const notices: string[] = []
  const deps = {
    currentProviderId: () => "codex", flashFooter: () => {}, appendNotice: (value: string) => notices.push(value),
    openProviderLoginLink: async (value: string) => { opened.push(value); return true },
    startProviderLogin: async () => ({ provider: "codex", account_profile: "default", login_kind: "device", login_id: null, auth_url: null, verification_url: "https://auth.openai.com/codex/device", user_code: "FIXTURE" }),
    getProviderLoginStatus: async () => ({ provider: "claude", account_profile: "default", login_id: "fixture", state: "running" as const, terminal_output_base64: Buffer.from(`Authorize:\n${url}\n`).toString("base64"), started_at_ms: 0, updated_at_ms: 0 }),
  }
  await handleProviderSlashCommand(deps, { kind: "provider", value: "login codex", raw: "/provider login codex" })
  await handleProviderSlashCommand(deps, { kind: "provider", value: "login-status fixture", raw: "/provider login-status fixture" })
  assert.deepEqual(opened, ["https://auth.openai.com/codex/device"])
  assert.ok(notices.includes(`\n${url}\n`))
  assert.ok(notices.some(value => value.includes("FIXTURE")))
})
