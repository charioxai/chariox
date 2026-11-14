import test from "node:test"
import assert from "node:assert/strict"
import type { AgentTrigger } from "@chariox/kernel-client/kernel-types"
import { buildAgentTriggerServer, loopbackHost } from "./agent-trigger-server.js"
import type { Conversation, Invocation } from "./agent-trigger-client.js"
import type { AgentHttpOptions, TriggerReply } from "@chariox/agent-trigger-gateway/http-trigger"

async function fixture(httpOptions: AgentHttpOptions = { route: "/", methods: ["GET", "POST"], parser: { kind: "json" }, mode: "async" }) {
  const trigger: AgentTrigger = { id: "http-test", agent_id: "agent-test", source: { kind: "http", ingress_id: "local-test", port: 12345, ...httpOptions },
    created_by_user_id: "local", busy_policy: "queue", prompt: "Act on this input", enabled: true, created_at_ms: 1, fired: 0, coalesced: 0 }
  let online = true
  const invocations: unknown[] = []
  const bodies: Array<{ body_base64: string; content_type: string } | null> = []
  const replies: TriggerReply[] = []
  let refusal: Invocation | null = null
  const resends: string[] = []
  let unconfirmed: Conversation["unconfirmed"] = []
  const client = {
    async metadata() { return { trigger, agent_alias: "Test agent", online, busy: false, history: { SessionHistoryOutline: { agents: [] } }, deliveries: [] } },
    async subscribe() {}, async close() {}, onChange() { return () => {} },
    async resend(promptId: string) { if (promptId === "gone") throw Error("already sent again"); resends.push(promptId) },
    async options() { return { enabled: trigger.enabled, options: httpOptions } },
    async reply(): Promise<TriggerReply> { return replies.shift() ?? { status: "pending" } },
    async invoke(input: unknown, http: { body_base64: string; content_type: string } | null): Promise<Invocation> {
      if (!online) return { status: "offline" }
      if (refusal) return refusal
      invocations.push(input); bodies.push(http)
      return { status: "accepted", prompt_id: `prompt-${invocations.length}`, outcome: "started" }
    },
    async workspaceImage(path: string) { return path === "charts/cost.png" ? { mime: "image/png", bytes: Buffer.from("png-bytes") } : null },
    async conversation(): Promise<Conversation> { return { alias: "Test agent", online, busy: false, enabled: trigger.enabled, entries: [
      { key: "1", role: "user", agentId: "agent-test", text: "conversation before trigger creation </script><script>bad()</script>" },
    ], turns: [], nextCursor: null, unconfirmed } },
  }
  const app = await buildAgentTriggerServer(client, "127.0.0.1")
  return { app, trigger, invocations, bodies, replies, resends, refuse: (value: Invocation) => { refusal = value }, offline: () => { online = false }, unconfirm: (items: Conversation["unconfirmed"]) => { unconfirmed = items } }
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

test("#1263 P2 the trigger page serves a workspace picture a reply references, and nothing it cannot read", async () => {
  const f = await fixture()
  try {
    const image = await f.app.inject({ method: "GET", url: "/local-test/workspace-image?path=charts%2Fcost.png" })
    assert.equal(image.statusCode, 200)
    assert.equal(image.headers["content-type"], "image/png")
    assert.equal(image.headers["x-content-type-options"], "nosniff")
    assert.equal(image.body, "png-bytes")
    assert.equal((await f.app.inject({ method: "GET", url: "/local-test/workspace-image?path=..%2Fsecret.png" })).statusCode, 404)
    assert.equal((await f.app.inject({ method: "GET", url: "/local-test/workspace-image" })).statusCode, 400)
  } finally { await f.app.close() }
})

test("AT-25: a trigger takes requests at its route with its methods only; the page stays at the base", async () => {
  const f = await fixture({ route: "/hooks/build", methods: ["POST"], parser: { kind: "json" }, mode: "async" })
  try {
    const fired = await f.app.inject({ method: "POST", url: "/local-test/hooks/build", payload: { ticket: 7 } })
    assert.equal(fired.statusCode, 202)
    assert.deepEqual(f.invocations, [{ ticket: 7 }])
    assert.equal(Buffer.from(f.bodies[0]!.body_base64, "base64").toString(), JSON.stringify({ ticket: 7 }))
    const get = await f.app.inject({ method: "GET", url: "/local-test/hooks/build?prompt=x" })
    assert.equal(get.statusCode, 405)
    assert.equal(get.headers.allow, "POST")
    for (const method of ["PUT", "DELETE", "PATCH"] as const) assert.equal((await f.app.inject({ method, url: "/local-test/hooks/build" })).statusCode, 405)
    assert.equal((await f.app.inject({ method: "POST", url: "/local-test/", payload: { ticket: 1 } })).statusCode, 404)
    assert.equal((await f.app.inject({ method: "POST", url: "/local-test/hooks/other", payload: {} })).statusCode, 404)
    assert.match((await f.app.inject("/local-test/")).body, /initial-conversation/)
    assert.equal(f.invocations.length, 1)
  } finally { await f.app.close() }
})

test("AT-26: path prompt, JSON body, query params and webhook envelope build the input as a publication does", async () => {
  const path = await fixture({ route: "/run", methods: ["GET", "POST"], parser: { kind: "path_template", template: "/run/:prompt" }, mode: "async" })
  try {
    assert.equal((await path.app.inject({ method: "GET", url: "/local-test/run/check%20the%20build" })).statusCode, 202)
    assert.deepEqual(path.invocations, [{ prompt: "check the build" }])
    assert.equal((await path.app.inject({ method: "GET", url: "/local-test/run" })).statusCode, 404)
  } finally { await path.app.close() }
  const query = await fixture({ route: "/", methods: ["GET"], parser: { kind: "query_params" }, mode: "async" })
  try {
    assert.equal((await query.app.inject({ method: "GET", url: "/local-test/?ticket=42&team=core" })).statusCode, 202)
    assert.deepEqual(query.invocations, [{ ticket: "42", team: "core" }])
    assert.equal((await query.app.inject({ method: "POST", url: "/local-test/", payload: {} })).statusCode, 405)
  } finally { await query.app.close() }
  const webhook = await fixture({ route: "/github", methods: ["POST"], parser: { kind: "webhook" }, mode: "async" })
  try {
    const signed = { "content-type": "application/json", "x-hub-signature-256": "sha256=abc" }
    assert.equal((await webhook.app.inject({ method: "POST", url: "/local-test/github?delivery=1", headers: signed, payload: JSON.stringify({ action: "opened" }) })).statusCode, 202)
    const envelope = webhook.invocations[0] as { headers: Record<string, string>; body: unknown; query: unknown }
    assert.equal(envelope.headers["x-hub-signature-256"], "sha256=abc")
    assert.deepEqual(envelope.body, { action: "opened" })
    assert.deepEqual(envelope.query, { delivery: "1" })
    const malformed = await webhook.app.inject({ method: "POST", url: "/local-test/github", headers: { "content-type": "application/json" }, payload: "{nope" })
    assert.equal(malformed.statusCode, 400)
    assert.equal(webhook.invocations.length, 1)
  } finally { await webhook.app.close() }
})

test("AT-27: sync waits for the reply, polling, and answers the reply, the failure or the timeout", async () => {
  const f = await fixture({ route: "/", methods: ["POST"], parser: { kind: "json" }, mode: "sync", sync_timeout_ms: 2_000, poll_ms: 50 })
  try {
    f.replies.push({ status: "pending" }, { status: "pending" }, { status: "completed", text: "The build is green." })
    const started = Date.now()
    const done = await f.app.inject({ method: "POST", url: "/local-test/", payload: { prompt: "status?" } })
    assert.equal(done.statusCode, 200)
    assert.equal(done.json().reply, "The build is green.")
    assert.equal(done.json().status, "completed")
    assert(Date.now() - started >= 100, "it polled twice before the reply")
    f.replies.push({ status: "failed", message: "The turn was stopped before the agent answered." })
    const failed = await f.app.inject({ method: "POST", url: "/local-test/", payload: {} })
    assert.equal(failed.statusCode, 502)
    assert.match(failed.json().error, /stopped/)
  } finally { await f.app.close() }
  const slow = await fixture({ route: "/", methods: ["POST"], parser: { kind: "json" }, mode: "sync", sync_timeout_ms: 150, poll_ms: 50 })
  try {
    const timeout = await slow.app.inject({ method: "POST", url: "/local-test/", payload: {} })
    assert.equal(timeout.statusCode, 202)
    assert.equal(timeout.json().status, "timeout")
    assert.equal(timeout.json().prompt_id, "prompt-1")
  } finally { await slow.app.close() }
  const async = await fixture({ route: "/", methods: ["POST"], parser: { kind: "json" }, mode: "async" })
  try {
    const accepted = await async.app.inject({ method: "POST", url: "/local-test/", payload: {} })
    assert.equal(accepted.statusCode, 202)
    assert.equal(accepted.json().reply, undefined)
  } finally { await async.app.close() }
})

test("AT-28: a request the input schema refuses is a client error with the kernel's reason", async () => {
  const f = await fixture()
  try {
    f.refuse({ status: "invalid_input", message: "input does not match the input schema: \"ticket\" is a required property" })
    const refused = await f.app.inject({ method: "POST", url: "/local-test/", payload: {}, headers: { accept: "application/json" } })
    assert.equal(refused.statusCode, 400)
    assert.match(refused.json().error, /required property/)
    assert.equal(f.invocations.length, 0)
  } finally { await f.app.close() }
})

test("the trigger page's message box sends its text whatever the route and parser are", async () => {
  const f = await fixture({ route: "/hooks", methods: ["GET"], parser: { kind: "query_params" }, mode: "sync", sync_timeout_ms: 1_000, poll_ms: 50 })
  try {
    const sent = await f.app.inject({ method: "POST", url: "/local-test/send", payload: { prompt: "hello from the page" } })
    assert.equal(sent.statusCode, 202)
    assert.deepEqual(f.invocations, [{ prompt: "hello from the page" }])
    assert.deepEqual(f.bodies, [null])
    assert.equal((await f.app.inject({ method: "POST", url: "/local-test/send", payload: {} })).statusCode, 400)
  } finally { await f.app.close() }
})

test("an offline agent does not use up the gateway's rate limit: every retry is a 503, never a 429", async () => {
  const f = await fixture()
  try {
    f.offline()
    for (let n = 0; n < 6; n++) {
      const response = await f.app.inject({ method: "POST", url: "/local-test/", payload: { prompt: "later" } })
      assert.equal(response.statusCode, 503, `retry ${n}`)
      assert.equal(response.headers["retry-after"], "5")
    }
    for (let n = 0; n < 4; n++) assert.equal((await f.app.inject({ method: "POST", url: "/local-test/send", payload: { prompt: "page" } })).statusCode, 503)
    assert.equal(f.invocations.length, 0)
  } finally { await f.app.close() }
})
