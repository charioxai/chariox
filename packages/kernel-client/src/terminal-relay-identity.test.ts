import assert from "node:assert/strict"
import test from "node:test"
import { TerminalRelayIdentity } from "./terminal-relay-identity.js"

function token(overrides: Record<string, unknown> = {}): string {
  const claims = { iss: "cloud", sub: "terminal", client_id: "terminal", subject_kind: "client", realm_id: "realm", account_id: "account", user_id: "user", public_key_thumbprint: "key", allowed_targets: ["kernel"], allowed_actions: ["connect"], exp: Math.floor(Date.now() / 1000) + 300, ...overrides }
  return "header." + Buffer.from(JSON.stringify(claims)).toString("base64url") + ".signature"
}
const tick = async () => { for (let i = 0; i < 20; i++) await Promise.resolve() }

test("MP-08/MP-11 terminal identity refresh precedes the pairing token expiry without a carrier change", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 1_000_000 })
  let requests = 0; let updates = 0
  const keeper = new TerminalRelayIdentity({ token: token(), relayUrl: "wss://relay", kernelId: "kernel", thumbprint: "key", eligible: () => true,
    request: async (key, body) => { requests++; assert.equal(key, "pinned"); assert.deepEqual(body, { IssueCloudRelayClientToken: { target_daemon_alias: "kernel", client_id: "terminal", session_id: null, public_key_thumbprint: "key" } }); return { CloudRelayClientTokenIssued: { token: { relay_url: "wss://relay", relay_token: token({ sub: "refreshed", client_id: "refreshed" }) } } } },
    onToken: () => { updates++ },
  })
  keeper.start("pinned"); t.mock.timers.tick(240_000); await tick()
  assert.equal(requests, 1, "the five-minute terminal identity was never refreshed")
  assert.equal(updates, 1)
  t.mock.timers.tick(240_000); await tick(); assert.equal(requests, 2)
  keeper.close(); t.mock.timers.tick(600_000); await tick(); assert.equal(requests, 2)
})

for (const [name, overrides] of Object.entries({ key: { public_key_thumbprint: "other" }, user: { user_id: "other" }, realm: { realm_id: "other" }, account: { account_id: "other" }, role: { subject_kind: "kernel" }, targets: { allowed_targets: ["other"] }, actions: { allowed_actions: ["admin"] }, expiry: { exp: 1 }, session: { session_id: "other" }, machine: { machine_id: "other" } })) {
  test(`MP-11 terminal refresh rejects changed ${name}`, async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 1_000_000 })
    let updates = 0
    const keeper = new TerminalRelayIdentity({ token: token(), relayUrl: "wss://relay", kernelId: "kernel", thumbprint: "key", eligible: () => true,
      request: async () => ({ CloudRelayClientTokenIssued: { token: { relay_url: "wss://relay", relay_token: token(overrides) } } }), onToken: () => { updates++ } })
    keeper.start("pinned"); t.mock.timers.tick(240_000); await tick(); assert.equal(updates, 0); keeper.close()
  })
}

test("MP-11 retiring the terminal fences a late identity issuer response", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 1_000_000 })
  let finish!: (body: Record<string, unknown>) => void; let updates = 0
  const keeper = new TerminalRelayIdentity({ token: token(), relayUrl: "wss://relay", kernelId: "kernel", thumbprint: "key", eligible: () => true,
    request: () => new Promise((resolve) => { finish = resolve }), onToken: () => { updates++ } })
  keeper.start("pinned"); t.mock.timers.tick(240_000); await tick(); keeper.close()
  assert.equal(typeof finish, "function")
  finish({ CloudRelayClientTokenIssued: { token: { relay_url: "wss://relay", relay_token: token() } } }); await tick(); assert.equal(updates, 0)
})


test("MP-11 an expired terminal bearer cannot authorize identity refresh", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 1_000_000 })
  let requests = 0
  const keeper = new TerminalRelayIdentity({ token: token({ exp: 1 }), relayUrl: "wss://relay", kernelId: "kernel", thumbprint: "key", eligible: () => true,
    request: async () => { requests++; return {} }, onToken: () => assert.fail("expired identity refreshed") })
  keeper.start("pinned"); t.mock.timers.tick(600_000); await tick(); assert.equal(requests, 0); keeper.close()
})
