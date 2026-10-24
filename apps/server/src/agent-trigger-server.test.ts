import test from "node:test"
import assert from "node:assert/strict"
import type { AgentTrigger } from "@chariox/kernel-client/kernel-types"
import { buildAgentTriggerServer, loopbackHost } from "./agent-trigger-server.js"
import type { Conversation, Invocation } from "./agent-trigger-client.js"

async function fixture() {
  const trigger: AgentTrigger = { id: "http-test", agent_id: "agent-test", source: { kind: "http", ingress_id: "local-test", port: 12345 },
    created_by_user_id: "local", busy_policy: "queue", prompt: "Act on this input", enabled: true, created_at_ms: 1, fired: 0, coalesced: 0 }
  let online = true
  const invocations: unknown[] = []
  const resends: string[] = []
  let unconfirmed: Conversation["unconfirmed"] = []
  const client = {
    async metadata() { return { trigger, agent_alias: "Test agent", online, busy: false, history: { SessionHistoryOutline: { agents: [] } }, deliveries: [] } },
    async subscribe() {}, async close() {}, onChange() { return () => {} },
    async resend(promptId: string) { if (promptId === "gone") throw Error("already sent again"); resends.push(promptId) },
    async invoke(payload: unknown): Promise<Invocation> { invocations.push(payload); return { status: "accepted", prompt_id: `prompt-${invocations.length}`, outcome: "started" } },
    async conversation(): Promise<Conversation> { return { alias: "Test agent", online, busy: false, enabled: trigger.enabled, entries: [
      { key: "1", role: "user", agentId: "agent-test", text: "conversation before trigger creation </script><script>bad()</script>" },
    ], turns: [], nextCursor: null, unconfirmed } },
  }
  const app = await buildAgentTriggerServer(client, "127.0.0.1")
  return { app, trigger, invocations, resends, offline: () => { online = false }, unconfirm: (items: Conversation["unconfirmed"]) => { unconfirmed = items } }
}

test("MP-08/MP-10/MP-11: non-loopback ingress is refused", () => {
  for (const host of ["0.0.0.0", "::", "example.org"]) assert.throws(() => loopbackHost(host), /loopback/)
  assert.equal(loopbackHost("127.0.0.1"), "127.0.0.1")
})
test("MP-08/MP-10/MP-11: URL GET and POST invoke once; HEAD and prefetch do not", async () => {
  const f = await fixture()
  try {
    for (const request of [{ method: "HEAD" as const, url: "/local-test/?prompt=unfurl" },
      { method: "GET" as const, url: "/local-test/?prompt=prefetch", headers: { "sec-purpose": "prefetch" } }]) {
      assert.equal((await f.app.inject(request)).statusCode, 200)
    }
    assert.equal(f.invocations.length, 0)
    assert.equal((await f.app.inject({ method: "GET", url: "/local-test/?prompt=hello&detail=one", headers: { accept: "application/json" } })).statusCode, 202)
    assert.equal((f.invocations[0] as { prompt: string }).prompt, "hello")
    const response = await f.app.inject({ method: "POST", url: "/local-test/", payload: { prompt: "second" }, headers: { accept: "text/html" } })
    assert.equal(response.statusCode, 303)
    assert.match(response.headers.location!, /^\/local-test\/#firing-/)
    assert.equal(f.invocations.length, 2)
  } finally { await f.app.close() }
})
test("MP-08/MP-10/MP-11: history predates the trigger and cannot escape its JSON script", async () => {
  const f = await fixture()
  try {
    const response = await f.app.inject("/local-test/")
    assert.match(response.body, /conversation before trigger creation/)
    assert.doesNotMatch(response.body, /<script>bad\(\)<\/script>/)
    const state = JSON.parse(response.body.match(/id="initial-conversation">([^]*?)<\/script>/)![1]!) as Conversation
    assert.equal(state.entries[0]?.text, "conversation before trigger creation </script><script>bad()</script>")
    assert.equal(f.invocations.length, 0)
  } finally { await f.app.close() }
})
test("AT-10: a malformed JSON body is a 400 while the agent is online, and never enqueues", async () => {
  const f = await fixture()
  try {
    const response = await f.app.inject({ method: "POST", url: "/local-test/", headers: { "content-type": "application/json" }, payload: "{nope" })
    assert.equal(response.statusCode, 400)
    assert.deepEqual(response.json(), { error: "invalid request body" })
    assert.equal(f.invocations.length, 0)
  } finally { await f.app.close() }
})
test("MP-08/MP-10/MP-11: offline, disabled, payload and rate limits never enqueue", async () => {
  const f = await fixture()
  try {
    f.offline()
    assert.equal((await f.app.inject({ method: "POST", url: "/local-test/", payload: { prompt: "later" } })).statusCode, 503)
    assert.equal(f.invocations.length, 0)
    f.trigger.enabled = false
    assert.equal((await f.app.inject({ method: "POST", url: "/local-test/", payload: {} })).statusCode, 403)
    assert.equal(f.invocations.length, 0)
    assert.equal((await f.app.inject({ method: "POST", url: "/local-test/", payload: { prompt: "a".repeat(70_000) } })).statusCode, 413)
  } finally { await f.app.close() }
  const active = await fixture()
  try {
    for (let n = 0; n < 3; n++) assert.equal((await active.app.inject({ method: "POST", url: "/local-test/", payload: {} })).statusCode, 202)
    const limited = await active.app.inject({ method: "POST", url: "/local-test/", payload: {} })
    assert.equal(limited.statusCode, 429)
    assert.equal(limited.headers["retry-after"], "10")
    assert.equal(active.invocations.length, 3)
  } finally { await active.app.close() }
})
test("MP-08/MP-10/MP-11: other websites cannot read or invoke the local page, the page itself still works", async () => {
  const f = await fixture()
  const local = { host: "127.0.0.1:41000" }
  const statuses = async (requests: { method: "GET" | "POST"; url: string; payload?: { prompt: string }; headers: Record<string, string> }[]) =>
    Promise.all(requests.map(async request => (await f.app.inject(request)).statusCode))
  try {
    assert.deepEqual(await statuses([
      { method: "POST", url: "/local-test/", payload: { prompt: "rebound" }, headers: { host: "attacker.example:41000" } },
      { method: "GET", url: "/local-test/history", headers: { host: "attacker.example" } },
      { method: "POST", url: "/local-test/", payload: { prompt: "form" }, headers: { ...local, origin: "https://attacker.example" } },
      { method: "POST", url: "/local-test/", payload: { prompt: "sandboxed" }, headers: { ...local, origin: "null" } },
      { method: "GET", url: "/local-test/history", headers: { ...local, origin: "https://attacker.example", "sec-fetch-site": "cross-site", "sec-fetch-mode": "cors" } },
      { method: "GET", url: "/local-test/?prompt=link", headers: { ...local, "sec-fetch-site": "cross-site", "sec-fetch-mode": "navigate" } },
      { method: "POST", url: "/local-test/", payload: { prompt: "image" }, headers: { ...local, "sec-fetch-site": "same-site" } },
    ]), [403, 403, 403, 403, 403, 403, 403])
    assert.equal(f.invocations.length, 0)
    const refused = await f.app.inject({ method: "GET", url: "/local-test/", headers: { host: "attacker.example", accept: "text/html" } })
    assert.match(refused.body, /Another website sent this request/)
    assert.doesNotMatch(refused.body, /id="initial-conversation"/)
    assert.deepEqual(await statuses([
      { method: "GET", url: "/local-test/", headers: { ...local, "sec-fetch-site": "cross-site", "sec-fetch-mode": "navigate", accept: "text/html" } },
      { method: "GET", url: "/local-test/history", headers: { ...local, origin: "http://127.0.0.1:41000", "sec-fetch-site": "same-origin", "sec-fetch-mode": "cors" } },
      { method: "POST", url: "/local-test/", payload: { prompt: "page" }, headers: { ...local, origin: "http://127.0.0.1:41000", "sec-fetch-site": "same-origin" } },
      { method: "GET", url: "/local-test/?prompt=typed", headers: { ...local, "sec-fetch-site": "none", "sec-fetch-mode": "navigate", accept: "application/json" } },
      { method: "POST", url: "/local-test/", payload: { prompt: "curl" }, headers: { host: "[::1]:41000" } },
    ]), [200, 200, 202, 202, 202])
    assert.equal(f.invocations.length, 3)
  } finally { await f.app.close() }
})

test("MP-08/MP-10: the trigger page lists an unconfirmed firing and resends it once, refusing other websites", async () => {
  const f = await fixture()
  try {
    f.unconfirm([{ prompt_id: "prompt-7", origin: "http", prompt: "Check <the> build.\nThen report." }])
    const page = await f.app.inject("/local-test/")
    assert.match(page.body, /<strong>Not confirmed<\/strong> · HTTP · Check &lt;the&gt; build\./)
    assert.match(page.body, /data-resend="prompt-7"/)
    const foreign = await f.app.inject({ method: "POST", url: "/local-test/resend", payload: { prompt_id: "prompt-7" }, headers: { origin: "https://evil.example" } })
    assert.equal(foreign.statusCode, 403)
    assert.deepEqual(f.resends, [])
    const own = await f.app.inject({ method: "POST", url: "/local-test/resend", payload: { prompt_id: "prompt-7" }, headers: { accept: "application/json" } })
    assert.equal(own.statusCode, 202)
    assert.deepEqual(f.resends, ["prompt-7"])
    assert.equal((await f.app.inject({ method: "POST", url: "/local-test/resend", payload: {} })).statusCode, 400)
    const gone = await f.app.inject({ method: "POST", url: "/local-test/resend", payload: { prompt_id: "gone" }, headers: { accept: "application/json" } })
    assert.equal(gone.statusCode, 409)
    assert.match(gone.body, /no longer waiting/)
  } finally { await f.app.close() }
})
