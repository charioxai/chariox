import assert from "node:assert/strict"
import test from "node:test"
import type { ProviderLoginStatus, RuntimeInteraction } from "./cli-types.js"
import { createProviderLoginInteractionController, type ProviderLoginInteractionControllerDeps } from "./provider-login-interaction-controller.js"

const url = "https://claude.ai/oauth/authorize?code=true&state=fixture"
const interaction: RuntimeInteraction = {
  id: "provider-auth-recovery:login-1", agent_id: "agent-1", kind: "choice", level: "warning", message: "Sign in",
  choices: [{ id: "cancel", label: "Cancel", reply: "cancel" }],
  custom_choice: { id: "provider-response", label: "Send response", input_kind: "secret", min_length: 1 },
  timeout_sec: 600, requested_at_ms: 1_000,
  provider_login: { kernel_id: "fleet", login: { provider: "claude", account_profile: "disposable-claude-ggpinwmx", login_kind: "terminal_setup_token", login_id: "login-1", auth_url: url }, terminal_output_base64: "" },
}
const output = (text: string) => Buffer.from(text).toString("base64")
const status = (state: ProviderLoginStatus["state"], text = ""): ProviderLoginStatus => ({
  provider: "claude", account_profile: "disposable-claude-ggpinwmx", login_id: "login-1", state,
  terminal_output_base64: output(text), started_at_ms: 50_000, updated_at_ms: 50_000,
})

function harness(statuses: Array<ProviderLoginStatus | Error>, overrides: Partial<ProviderLoginInteractionControllerDeps> = {}) {
  const timers: Array<() => void> = []
  const notices: string[] = []
  let verificationRequests = 0
  const flashes: Array<{ message: string; tone: string }> = []
  const controller = createProviderLoginInteractionController({
    getKernelId: async () => "fleet",
    getLoginStatus: async () => {
      const next = statuses.length > 1 ? statuses.shift()! : statuses[0]!
      if (next instanceof Error) throw next
      return next
    },
    // A legacy verification spy makes an accidental completion call fail.
    ...{getAuthStatus: async () => { verificationRequests++; return {identity_summary: null} }},
    getAccountProfile: async () => ({identity_summary: "miguel@example.org"}),
    accountLabel: () => "disposable-claude",
    localDesktop: () => false,
    openUrl: async () => true,
    copyUrl: async () => "clipboard request sent (OSC 52, unconfirmed); if empty, use native selection and Copy",
    appendNotice: (message) => { notices.push(message) },
    flashFooter: (message, tone) => { flashes.push({ message, tone }) },
    render: () => {},
    now: () => 60_000,
    setTimer: (callback) => { timers.push(callback); return timers.length },
    clearTimer: () => {},
    ...overrides,
  })
  const tick = async () => { const next = timers.shift(); next?.(); for (let i = 0; i < 5; i++) await Promise.resolve() }
  return { controller, tick, timers, notices, flashes, verificationRequests: () => verificationRequests }
}

test("MP-08/MP-11 a sent code waits for the kernel and ends with who signed in and where it was saved", async () => {
  const h = harness([status("running", "Paste code here if prompted >"), status("running", "Paste code here if prompted > ****"), status("succeeded")])
  const state = h.controller.stripState(interaction)!
  assert.equal(state.view.url, url)
  assert.equal(state.deadlineMs, 1_000 + 600_000)
  await h.tick()
  assert.equal(h.controller.stripState(interaction)!.deadlineMs, 50_000 + 600_000, "the kernel's login start sets the deadline")
  assert.equal(h.controller.codeSent(interaction), true)
  assert.equal(h.controller.stripState(interaction)!.checking, true)
  assert.match(h.flashes.at(-1)!.message, /Claude · disposable-claude: checking the code/)
  await h.tick()
  assert.equal(h.controller.stripState(interaction)!.checking, true)
  await h.tick()
  for (let i = 0; i < 5; i++) await Promise.resolve()
  assert.deepEqual(h.notices, ["Signed in to Claude as miguel@example.org · saved to disposable-claude"])
  assert.equal(h.timers.length, 0, "polling stops with the result")
})

test("MP-08/MP-11 a failed login names the kernel's reason and how to retry", async () => {
  const h = harness([status("failed", "Paste code here if prompted > ****\nClaude did not accept the setup token; nothing was stored: 401 Unauthorized")])
  h.controller.track({ login_id: "login-1", provider: "claude", account_profile: "disposable-claude-ggpinwmx" })
  await h.tick()
  assert.deepEqual(h.notices, ["Claude · disposable-claude: sign-in failed — Claude did not accept the setup token; nothing was stored: 401 Unauthorized\nRetry: /provider login claude disposable-claude"])
  assert.equal(h.flashes.at(-1)!.tone, "error")
  // A finished login is never tracked or reported again.
  h.controller.track({ login_id: "login-1", provider: "claude", account_profile: "disposable-claude-ggpinwmx" })
  assert.equal(h.timers.length, 0)
})

test("MP-08/MP-11 a code the provider rejects is shown while the login keeps waiting", async () => {
  const h = harness([status("running", "Paste code here if prompted >"), status("running", "OAuth error: Request failed with status code 400\nPress Enter to retry.")])
  h.controller.stripState(interaction)
  await h.tick()
  h.controller.codeSent(interaction)
  await h.tick()
  const state = h.controller.stripState(interaction)!
  assert.equal(state.checking, false)
  assert.equal(state.problem, "OAuth error: Request failed with status code 400")
  assert.deepEqual(h.notices, [])
})

test("MP-08/MP-11 three failed status reads preserve the watcher and reconcile success after re-render", async () => {
  const h = harness([
    status("running"),
    new Error("relay disconnected"), new Error("request timed out"), new Error("kernel unreachable"),
    status("succeeded"),
  ])
  h.controller.stripState(interaction)
  await h.tick()
  h.controller.codeSent(interaction)
  await h.tick(); await h.tick(); await h.tick()
  assert.deepEqual(h.notices, [], "transport failure cannot determine the login outcome")
  const state = h.controller.stripState({ ...interaction })!
  assert.equal(state.checking, true)
  assert.equal(state.deadlineMs, 50_000 + 600_000)
  assert.equal(h.timers.length, 1, "the same projection keeps exactly one watcher")
  await h.tick()
  assert.deepEqual(h.notices, ["Signed in to Claude as miguel@example.org · saved to disposable-claude"])
  h.controller.stripState({ ...interaction })
  assert.equal(h.timers.length, 0, "a confirmed outcome stops tracking even after re-render")
})

test("MP-08/MP-11 kernel identity read failures retry until the kernel confirms cancellation", async () => {
  let reads = 0
  const h = harness([status("cancelled")], {
    getKernelId: async () => {
      if (++reads <= 3) throw new Error("relay disconnected")
      return "fleet"
    },
  })
  h.controller.stripState(interaction)
  await h.tick(); await h.tick(); await h.tick()
  assert.deepEqual(h.notices, [])
  h.controller.stripState({ ...interaction })
  assert.equal(h.timers.length, 1)
  await h.tick()
  assert.match(h.notices[0]!, /sign-in cancelled/)
  assert.equal(h.timers.length, 0)
})

test("MP-08/MP-11 disposing during a failed status read does not restart polling", async () => {
  let rejectRead!: (error: Error) => void
  const h = harness([], {
    getLoginStatus: () => new Promise((_resolve, reject) => { rejectRead = reject }),
  })
  h.controller.stripState(interaction)
  await h.tick()
  h.controller.dispose()
  rejectRead(new Error("relay disconnected"))
  await h.tick()
  assert.deepEqual(h.notices, [])
  assert.equal(h.timers.length, 0)
})

test("MP-08/MP-11 retry and Vault phases keep the kernel's message and still follow the final login result", async () => {
  for (const custom of [null, { id: "passphrase", label: "Vault passphrase", input_kind: "secret" as const, min_length: 1 }]) {
    const h = harness([status("succeeded")])
    const phase: RuntimeInteraction = { ...interaction, title: custom ? "Unlock Chariox Vault" : "Retry Claude authorization", custom_choice: custom }
    assert.equal(h.controller.stripState(phase), null, "the authorization strip must not replace another kernel-owned phase, even with a retained URL")
    await h.tick()
    assert.match(h.notices[0]!, /Signed in to Claude as miguel@example.org · saved to disposable-claude/)
    h.controller.dispose()
  }
})

test("MP-08/MP-11 MP-10 over SSH a link click stays in the TUI and explains native opening", async () => {
  const h = harness([status("running")])
  await h.controller.openLink(interaction)
  assert.match(h.flashes.at(-1)!.message, /Cmd-click.*Cmd-double-click.*Cmd-C/)
  await h.controller.copyLink(interaction)
  assert.match(h.flashes.at(-1)!.message, /^Link: clipboard request sent \(OSC 52, unconfirmed\)/)
  const local = harness([status("running")], { localDesktop: () => true })
  await local.controller.openLink(interaction)
  assert.equal(local.flashes.at(-1)!.message, "Opened the link in your browser")
})

test("MP-08/MP-11 ordinary terminal logins preserve the official CLI prompts before and after a URL", () => {
  for (const auth_url of [null, "https://auth.openai.com/authorize"]) {
    const h = harness([status("running")])
    const terminal: RuntimeInteraction = {
      ...interaction,
      message: "Complete the provider's official login.",
      provider_login: {
        kernel_id: "fleet",
        login: { ...interaction.provider_login!.login, provider: "opencode", login_kind: "terminal", auth_url },
        terminal_output_base64: output("Select provider\nOpenAI\nSelect login method\nChatGPT"),
      },
    }
    assert.equal(h.controller.stripState(terminal), null, "the generic renderer must retain the message and terminal output")
    h.controller.dispose()
  }
})

test("MP-08/MP-11 a home-attached TUI leaves worker login status and completion to the worker projection", async () => {
  const reads: unknown[][] = []
  const h = harness([], {
    getLoginStatus: async (...args) => { reads.push(args); throw Error("provider login was not found") },
    getAccountProfile: async () => { throw Error("home must not query worker auth") },
  })
  const forwarded: RuntimeInteraction = {
    ...interaction,
    provider_login: { ...interaction.provider_login!, kernel_id: "worker" },
  }
  h.controller.stripState(forwarded)
  await h.tick(); await h.tick(); await h.tick()
  assert.deepEqual(reads, [], "worker login IDs must never be queried against the home store")
  assert.deepEqual(h.notices, [], "home lookup failure must never replace the execution kernel's result notice")
  assert.equal(h.controller.codeSent(forwarded), true)
  assert.equal(h.controller.stripState(forwarded)!.checking, true)
  const updated: RuntimeInteraction = {
    ...forwarded,
    provider_login: { ...forwarded.provider_login!, terminal_output_base64: output("OAuth error: code expired") },
  }
  assert.equal(h.controller.stripState(updated)!.problem, "OAuth error: code expired")
  assert.equal(h.controller.stripState(updated)!.checking, false)
  assert.equal(h.timers.length, 0, "forwarded output comes from session projections")
  h.controller.dispose()
})

test("MP-08/MP-11 the local slash-command watcher adopts its kernel projection once", async () => {
  const h = harness([status("succeeded")])
  h.controller.track({ login_id: "login-1", provider: "claude", account_profile: "disposable-claude-ggpinwmx" })
  h.controller.stripState(interaction)
  assert.equal(h.timers.length, 1)
  await h.tick()
  h.controller.stripState(interaction)
  assert.equal(h.notices.length, 1)
  assert.equal(h.timers.length, 0)
  h.controller.dispose()
})

for (const kernelId of ["fleet", "worker"]) {
  test(`MP-08/MP-10/MP-11 ${kernelId} login exposes a shared rejection without a local code submission`, async () => {
    const error = "OAuth error: Request failed with status code 400"
    const h = harness([status("running", error)])
    const shared = { ...interaction, provider_login: { ...interaction.provider_login!, kernel_id: kernelId, terminal_output_base64: output(error) } }
    try {
      h.controller.stripState(shared)
      await h.tick()
      assert.equal(h.controller.stripState(shared)!.problem, error)
      assert.equal(h.controller.stripState(shared)!.checking, false)
    } finally { h.controller.dispose() }
  })
}

// MP-08 / MP-11: login completion is an observation, never a paid token probe.
test("MP-08 / MP-11 successful login completion sends no explicit auth verification", async () => {
  const h = harness([status("succeeded")])
  h.controller.track({ login_id: "login-1", provider: "claude", account_profile: "disposable-claude-ggpinwmx" })
  await h.tick()
  assert.equal(h.verificationRequests(), 0)
  assert.match(h.notices[0]!, /Signed in to Claude/)
})

test("MP-08 / MP-11 completion identity uses only the saved profile RPC", async () => {
  const {getProviderAccountProfile} = await import("./provider-api.js")
  const requests: unknown[] = []
  const client = {send: async (request: unknown) => {requests.push(request); return {ProviderAccountProfile: {profile: {identity_summary: "saved identity"}}}}} as unknown as import("./ipc.js").LocalIpcClient
  const h = harness([status("succeeded")], {getAccountProfile: (provider, profile) => getProviderAccountProfile(client, provider, profile)})
  h.controller.track({login_id: "login-1", provider: "claude", account_profile: "disposable-claude-ggpinwmx"})
  await h.tick()
  for (let i = 0; i < 10; i++) await Promise.resolve()
  assert.deepEqual(requests, [{GetProviderAccountProfile: {provider: "claude", account_profile: "disposable-claude-ggpinwmx"}}])
  assert.match(h.notices[0]!, /as saved identity/)
})

for (const message of [
  "The code looks incomplete. Copy the whole code, including # and the part after it.",
  "That code was not accepted. Open the new sign-in link and copy a new code.",
]) {
  test(`MP-08/MP-11 structured retry restores the code field: ${message}`, async () => {
    const next = { ...status("running"), message }
    const h = harness([status("running"), next])
    h.controller.stripState(interaction)
    await h.tick()
    h.controller.codeSent(interaction)
    await h.tick()
    const state = h.controller.stripState(interaction)!
    assert.equal(state.checking, false)
    assert.equal(state.problem, message)
    // Resubmitting while the previous retry is still projected waits again.
    h.controller.codeSent({ ...interaction, message })
    assert.equal(h.controller.stripState({ ...interaction, message })!.checking, true)
    await h.tick()
    assert.equal(h.controller.stripState({ ...interaction, message })!.checking, false, "another identical rejection answers the next submission")
    h.controller.dispose()
  })

  test(`MP-08/MP-11 a forwarded interaction answers the sent code: ${message}`, async () => {
    const h = harness([], { getKernelId: async () => "home" })
    h.controller.stripState(interaction)
    await h.tick()
    h.controller.codeSent(interaction)
    const state = h.controller.stripState({ ...interaction, message })!
    assert.equal(state.checking, false)
    assert.equal(state.problem, message)
    h.controller.dispose()
  })

  test(`MP-08/MP-10/MP-11 first attachment at a forwarded retry preserves repeated errors: ${message}`, async () => {
    let reads = 0
    const h = harness([], {
      getKernelId: async () => "home",
      getLoginStatus: async () => { reads++; throw Error("forwarded login must use projections") },
    })
    const retry = { ...interaction, message }
    try {
      assert.equal(h.controller.stripState(retry)!.problem, message, "the first observed message can already be a retry")
      await h.tick()
      assert.equal(h.timers.length, 0, "forwarded logins stop polling")
      assert.equal(h.controller.stripState(retry)!.problem, message)
      for (let attempt = 0; attempt < 2; attempt++) {
        assert.equal(h.controller.codeSent(retry), true)
        assert.equal(h.controller.stripState(retry)!.checking, true, "the old retry cannot answer a new submission")
        assert.equal(h.controller.stripState(retry)!.problem, null)
        retry.requested_at_ms++
        const state = h.controller.stripState(retry)!
        assert.equal(state.checking, false, "a new request with the same retry answers the submission")
        assert.equal(state.problem, message)
      }
      assert.equal(reads, 0)
    } finally { h.controller.dispose() }
  })
}

test("MP-08/MP-10/MP-11 first attachment to forwarded sign-in instructions has no problem", async () => {
  const h = harness([], { getKernelId: async () => "home" })
  const initial = {
    ...interaction,
    message: "Open the Claude authorization link to sign in. If Claude gives you a code, paste it below.",
  }
  try {
    assert.equal(h.controller.stripState(initial)!.problem, null)
    await h.tick()
    assert.equal(h.timers.length, 0)
    assert.equal(h.controller.stripState(initial)!.problem, null)
    h.controller.codeSent(initial)
    assert.equal(h.controller.stripState(initial)!.checking, true)
    assert.equal(h.controller.stripState(initial)!.problem, null)
  } finally { h.controller.dispose() }
})


test("MP-08/MP-11 a status read started before submission cannot answer the new code", async () => {
  const retry = { ...status("running"), message: "That code was not accepted. Open the new sign-in link and copy a new code." }
  let complete!: (value: ProviderLoginStatus) => void
  let reads = 0
  const h = harness([], { getLoginStatus: () => ++reads === 1 ? new Promise(resolve => { complete = resolve }) : Promise.resolve(retry) })
  h.controller.stripState(interaction)
  await h.tick()
  h.controller.codeSent(interaction)
  complete(retry)
  for (let index = 0; index < 5; index++) await Promise.resolve()
  assert.equal(h.controller.stripState(interaction)!.checking, true)
  await h.tick()
  assert.equal(h.controller.stripState(interaction)!.checking, false)
  h.controller.dispose()
})
