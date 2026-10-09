import assert from "node:assert/strict"
import { EventEmitter } from "node:events"
import { closeSync, mkdtempSync, openSync, readFileSync, rmSync } from "node:fs"
import os from "node:os"
import path from "node:path"
import test from "node:test"
import { createProviderLoginLinkPresenter, providerLoginLinkText, providerLoginUrl, providerLoginUrls } from "./provider-login-link.js"
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

test("MP-08/MP-11 Codex login start and Claude terminal-status links reach the same presenter", async () => {
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

test("MP-08/MP-11 login-status hands off only a waiting login and never auto-opens scraped PTY links", async () => {
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

test("MP-08/MP-11 the link view shows each authorization link once", async () => {
  const directory = mkdtempSync(path.join(process.env.TMPDIR ?? os.tmpdir(), "login-link-"))
  const outputPath = path.join(directory, "terminal")
  const fd = openSync(outputPath, "w")
  const input = Object.assign(new EventEmitter(), { isTTY: true, setRawMode: () => {}, resume: () => {} })
  const calls: string[] = []
  const renderer = { suspend: () => { calls.push("suspend") }, resume: () => { calls.push("resume") }, idle: async () => {}, copyToClipboardOSC52: () => false }
  try {
    const present = createProviderLoginLinkPresenter(renderer, { input, output: { isTTY: true, fd } })
    const first = present(url, { autoOpen: false })
    await new Promise(resolve => setImmediate(resolve))
    input.emit("data", Buffer.from("\r"))
    assert.equal(await first, true)
    assert.equal(await present(url, { autoOpen: false }), false)
    assert.deepEqual(calls, ["suspend", "resume"])
    assert.ok(readFileSync(outputPath, "utf8").includes(providerLoginLinkText(url)))
  } finally {
    closeSync(fd)
    rmSync(directory, { recursive: true, force: true })
  }
})

test("MP-08/MP-11 the transcript keeps the complete link after the link view closes", async () => {
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

test("MP-08/MP-11 the link view copies through the renderer's OSC 52 gate", async () => {
  const directory = mkdtempSync(path.join(process.env.TMPDIR ?? os.tmpdir(), "login-link-"))
  const outputPath = path.join(directory, "terminal")
  const fd = openSync(outputPath, "w")
  const input = Object.assign(new EventEmitter(), { isTTY: true, setRawMode: () => {}, resume: () => {} })
  const ssh = process.env.SSH_CONNECTION
  process.env.SSH_CONNECTION = "fixture 1 fixture 2"
  try {
    for (const supported of [true, false]) {
      const copies: string[] = []
      const renderer = { suspend: () => {}, resume: () => {}, idle: async () => {}, copyToClipboardOSC52: (text: string) => { copies.push(text); return supported } }
      const link = `${url}${supported}`
      const present = createProviderLoginLinkPresenter(renderer, { input, output: { isTTY: true, fd } })
      const shown = present(link, { autoOpen: false })
      await new Promise(resolve => setImmediate(resolve))
      input.emit("data", Buffer.from("c"))
      await new Promise(resolve => setTimeout(resolve, 50))
      input.emit("data", Buffer.from("\r"))
      assert.equal(await shown, true)
      assert.deepEqual(copies, [link])
      const written = readFileSync(outputPath, "utf8")
      assert.ok(written.includes(supported ? "OSC 52, unconfirmed" : "clipboard unavailable"))
      assert.ok(!written.includes("\x1b]52;"))
    }
  } finally {
    if (ssh === undefined) delete process.env.SSH_CONNECTION
    else process.env.SSH_CONNECTION = ssh
    closeSync(fd)
    rmSync(directory, { recursive: true, force: true })
  }
})

test("MP-08/MP-11 a clicked link shows again with numbered steps, and a bracketed paste returns with the code", async () => {
  const directory = mkdtempSync(path.join(process.env.TMPDIR ?? os.tmpdir(), "login-link-"))
  const outputPath = path.join(directory, "terminal")
  const fd = openSync(outputPath, "w")
  const input = Object.assign(new EventEmitter(), { isTTY: true, setRawMode: () => {}, resume: () => {} })
  const renderer = { suspend: () => {}, resume: () => {}, idle: async () => {}, copyToClipboardOSC52: () => false }
  try {
    const present = createProviderLoginLinkPresenter(renderer, { input, output: { isTTY: true, fd } })
    const first = present(url, { autoOpen: false })
    await new Promise(resolve => setImmediate(resolve))
    input.emit("data", Buffer.from("\r"))
    assert.equal(await first, true)
    const pasted: string[] = []
    const again = present(url, { force: true, autoOpen: false, title: "Sign in to Claude · work", steps: ["Authorize Chariox in your browser.", "Paste the code here (Cmd-V); Chariox returns with it."], onPaste: text => pasted.push(text) })
    await new Promise(resolve => setImmediate(resolve))
    // Split across reads, as a terminal may deliver a long paste.
    input.emit("data", Buffer.from("\x1b[200~code#"))
    assert.deepEqual(pasted, [])
    input.emit("data", Buffer.from("state\x1b[201~"))
    assert.equal(await again, true)
    assert.deepEqual(pasted, ["code#state"])
    const written = readFileSync(outputPath, "utf8")
    assert.ok(written.includes(`Sign in to Claude · work\r\n1. Open this link (Cmd-click it, or select and copy it):\r\n${providerLoginLinkText(url)}2. Authorize Chariox in your browser.\r\n3. Paste the code here (Cmd-V); Chariox returns with it.\r\n\x1b[?2004h`))
  } finally {
    closeSync(fd)
    rmSync(directory, { recursive: true, force: true })
  }
})

test("MP-08/MP-11 bracketed paste survives every delimiter split and UTF-8 byte chunks", async () => {
  const directory = mkdtempSync(path.join(process.env.TMPDIR ?? os.tmpdir(), "login-link-"))
  const fd = openSync(path.join(directory, "terminal"), "w")
  const opener = "\x1b[200~", closer = "\x1b[201~"
  const cases: Buffer[][] = []
  for (let start = 1; start < opener.length; start++) {
    for (let end = 1; end < closer.length; end++) {
      cases.push([opener.slice(0, start), opener.slice(start) + "code#state" + closer.slice(0, end), closer.slice(end)].map(value => Buffer.from(value)))
    }
  }
  cases.push([...Buffer.from(opener + "code#státé" + closer)].map(byte => Buffer.from([byte])))
  try {
    for (const [caseIndex, chunks] of cases.entries()) {
      const input = Object.assign(new EventEmitter(), { isTTY: true, setRawMode: () => {}, resume: () => {} })
      let resumes = 0
      const renderer = { suspend: () => {}, resume: () => { resumes++ }, idle: async () => {}, copyToClipboardOSC52: () => false }
      const pasted: string[] = []
      const present = createProviderLoginLinkPresenter(renderer, { input, output: { isTTY: true, fd } })
      const shown = present(url, { onPaste: value => pasted.push(value) })
      await new Promise(resolve => setImmediate(resolve))
      for (const [chunkIndex, chunk] of chunks.entries()) {
        input.emit("data", chunk)
        await new Promise(resolve => setImmediate(resolve))
        // A slow SSH packet must not turn the opener's ESC into a close.
        if (caseIndex === 0 && chunkIndex === 0) await new Promise(resolve => setTimeout(resolve, 300))
      }
      assert.deepEqual(pasted, [chunks.length === 3 ? "code#state" : "code#státé"])
      assert.equal(await shown, true)
      assert.equal(resumes, 1)
      assert.equal(input.listenerCount("data"), 0)
    }
  } finally {
    closeSync(fd)
    rmSync(directory, { recursive: true, force: true })
  }
})
