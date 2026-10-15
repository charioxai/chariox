import assert from "node:assert/strict"
import test from "node:test"
import { createProviderLoginLinkOpener, providerLoginUrl, providerLoginUrls } from "./provider-login-link.js"
import { handleProviderSlashCommand } from "./provider-command-handlers.js"
const url = `https://claude.ai/oauth/authorize?client_id=fixture&state=${"abc123".repeat(70)}`

test("MP-08/MP-10/MP-11 URLs exclude controls, credentials and nonweb schemes", () => {
  assert.deepEqual(providerLoginUrls(`Authorize:\r\n\x1b[32m${url}\x1b[0m\r\n`), [url])
  for (const value of ["file:///etc/passwd", "javascript:alert(1)", "https://a/\x1b]52;bad", "https://u:p@host/", "https://a/\nnext"]) assert.equal(providerLoginUrl(value), null)
})

test("MP-08/MP-10/MP-11 SSH never opens a remote browser; local explicit starts open once", async () => {
  const calls: string[] = []
  let desktop = false
  const open = createProviderLoginLinkOpener({localDesktop: () => desktop, openUrl: async value => {calls.push(value); return true}})
  assert.equal(await open(url, {autoOpen:true}), false)
  desktop = true
  assert.equal(await open(url), false, "scraped status links never open automatically")
  assert.equal(await open(url, {autoOpen:true}), true)
  assert.equal(await open(url, {autoOpen:true}), false)
  assert.equal(await open("file:///bad", {autoOpen:true}), false)
  assert.deepEqual(calls, [url])
})

test("MP-08/MP-11 Codex login start and Claude terminal-status links keep links in the transcript", async () => {
  const links: string[] = []
  const notices: string[] = []
  const deps = {
    currentProviderId: () => "codex", flashFooter: () => {}, appendNotice: (value: string) => notices.push(value),
    showProviderLoginLink: async (value: string) => { links.push(value); return true },
    startProviderLogin: async () => ({ provider: "codex", account_profile: "default", login_kind: "device", login_id: null, auth_url: null, verification_url: "https://auth.openai.com/codex/device", user_code: "FIXTURE" }),
    getProviderLoginStatus: async () => ({ provider: "claude", account_profile: "default", login_id: "fixture", state: "running" as const, terminal_output_base64: Buffer.from(`Authorize:\n${url}\n`).toString("base64"), started_at_ms: 0, updated_at_ms: 0 }),
  }
  await handleProviderSlashCommand(deps, { kind: "provider", value: "login codex", raw: "/provider login codex" })
  await handleProviderSlashCommand(deps, { kind: "provider", value: "login-status fixture", raw: "/provider login-status fixture" })
  assert.deepEqual(links, ["https://auth.openai.com/codex/device", url])
  assert.ok(notices.some(value => value.includes("FIXTURE")))
})

test("MP-08/MP-11 login-status requests browser opening only for a waiting login and never auto-opens scraped PTY links", async () => {
  const links: Array<{ url: string; autoOpen: boolean | undefined }> = []
  let state: "running" | "succeeded" = "running"
  const deps = {
    currentProviderId: () => "claude", flashFooter: () => {}, appendNotice: () => {},
    showProviderLoginLink: async (value: string, options?: { autoOpen?: boolean }) => { links.push({ url: value, autoOpen: options?.autoOpen }); return true },
    startProviderLogin: async () => ({ provider: "codex", account_profile: "default", login_kind: "device", login_id: null, auth_url: null, verification_url: "https://auth.openai.com/codex/device", user_code: "FIXTURE" }),
    getProviderLoginStatus: async () => ({ provider: "claude", account_profile: "default", login_id: "fixture", state, terminal_output_base64: Buffer.from(`Authorize:\n${url}\nDone\n`).toString("base64"), started_at_ms: 0, updated_at_ms: 0 }),
  }
  await handleProviderSlashCommand(deps, { kind: "provider", value: "login codex", raw: "/provider login codex" })
  await handleProviderSlashCommand(deps, { kind: "provider", value: "login-status fixture", raw: "/provider login-status fixture" })
  state = "succeeded"
  await handleProviderSlashCommand(deps, { kind: "provider", value: "login-status fixture", raw: "/provider login-status fixture" })
  assert.deepEqual(links, [{ url: "https://auth.openai.com/codex/device", autoOpen: true }, { url, autoOpen: false }])
})

test("MP-08/MP-11 the transcript keeps complete links and omits URLs with credentials", async () => {
  const notices: string[] = []
  const flashes: string[] = []
  let verificationUrl = url
  const deps = {
    currentProviderId: () => "claude", flashFooter: (value: string) => { flashes.push(value) }, appendNotice: (value: string) => { notices.push(value) },
    showProviderLoginLink: async () => true,
    startProviderLogin: async () => ({ provider: "claude", account_profile: "default", login_kind: "oauth", login_id: null, auth_url: null, verification_url: verificationUrl, user_code: null }),
    getProviderLoginStatus: async () => ({ provider: "claude", account_profile: "default", login_id: "fixture", state: "running" as const, terminal_output_base64: Buffer.from(`Authorize:\n${url}2\n`).toString("base64"), started_at_ms: 0, updated_at_ms: 0 }),
  }
  await handleProviderSlashCommand(deps, { kind: "provider", value: "login claude", raw: "/provider login claude" })
  await handleProviderSlashCommand(deps, { kind: "provider", value: "login-status fixture", raw: "/provider login-status fixture" })
  assert.match(flashes[0] ?? "", /authorization link below/)
  assert.ok(notices.includes(`\n${url}\n`))
  assert.ok(notices.includes(`\n${url}2\n`))
  verificationUrl = "https://user:secret@example.org/authorize"
  await handleProviderSlashCommand(deps, { kind: "provider", value: "login claude", raw: "/provider login claude" })
  assert.doesNotMatch(flashes.at(-1) ?? "", /authorization link below/)
})
