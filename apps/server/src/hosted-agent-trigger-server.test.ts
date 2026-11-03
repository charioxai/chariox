import test from "node:test"
import assert from "node:assert/strict"
import Fastify from "fastify"
import {buildHostedAgentTriggerServer} from "./hosted-agent-trigger-server.js"

// MP-08 / MP-09 / MP-10 / MP-11: runtime bytes never enter the control-plane request.
async function fixture() {
  const control = Fastify(), metadata: Array<{url: string; body: unknown; cookie: string | undefined}> = [], requests: unknown[] = []
  let denied = 0, closed = 0
  control.post("/browser/agent-trigger-deployments/:id/admission", async (request, reply) => {
    metadata.push({url: request.url, body: request.body, cookie: request.headers.cookie})
    if (denied) return reply.code(denied).send({error: "refused"})
    reply.header("content-security-policy", "default-src 'self'; frame-ancestors 'none'")
    reply.header("x-content-type-options", "nosniff")
    reply.header("x-internal-admission", "not-public")
    reply.header("set-cookie", "cloud=session; HttpOnly; Secure; Path=/")
    return {deployment: {id: (request.params as {id: string}).id, kernelId: "kernel", sessionId: "session", triggerId: "trigger"}, ...((request.body as {publicKeyThumbprint?: string}).publicKeyThumbprint ? {bootstrap: {relayUrl: "wss://relay.example", relayToken: "test-grant", relayTokenExpiresAt: new Date(Date.now() + 60_000).toISOString(), target: {daemonId: "kernel", kernelRef: "kernel"}}} : {})}
  })
  await control.listen({host: "127.0.0.1", port: 0})
  const address = control.server.address();assert(address && typeof address === "object")
  const app = buildHostedAgentTriggerServer({publicOrigin: "https://cloud.example", controlOrigin: `http://127.0.0.1:${address.port}`, connect: (async (kernel: string, bootstrap: (key: string) => Promise<unknown>, origin: string) => {
    assert.equal(kernel, "kernel");assert.equal(origin, "https://cloud.example");await bootstrap("a".repeat(64))
    return {send: async (value: unknown) => {requests.push(value);return {AgentTriggerInvoked: {invocation: {status: "accepted", prompt_id: `prompt-${requests.length}`, outcome: "queued"}}}}, close: async () => {closed++}}
  }) as never})
  return {app, metadata, requests, closed: () => closed, deny: (status: number) => {denied = status}, close: async () => {await app.close();await control.close()}}
}

test("hosted runtime ingress preserves JSON, form, text and large bodies without sending them to Cloud", async () => {
  const f = await fixture()
  try {
    for (const [index, type, raw] of [[0, "application/json", JSON.stringify({value: "x".repeat(30_000)})], [1, "application/x-www-form-urlencoded", "prompt=RUNTIME_ONLY_MARKER+%CE%BB&extra=%2B"], [2, "text/plain", "RUNTIME_ONLY_MARKER\r\nλ"]] as const) {
      const response = await f.app.inject({method: "POST", url: `/agent-triggers/deployment-${index}/?extra=RUNTIME_QUERY_MARKER`, headers: {"content-type": type, "x-chariox-browser-session": "owner", cookie: "other=value"}, payload: raw})
      assert.equal(response.statusCode, 202);assert.equal(response.json().prompt_id, `prompt-${index + 1}`)
      const payload = (f.requests.at(-1) as {InvokeAgentTrigger: {payload: {http: {body_base64: string; content_type: string}}}}).InvokeAgentTrigger.payload
      assert.deepEqual(Buffer.from(payload.http.body_base64, "base64"), Buffer.from(raw));assert.equal(payload.http.content_type, type)
    }
    assert.equal(f.closed(), 3)
    const admission = JSON.stringify(f.metadata)
    assert(!admission.includes("RUNTIME_ONLY_MARKER"));assert(!admission.includes("RUNTIME_QUERY_MARKER"));assert(!admission.includes("x".repeat(100)))
    assert(f.metadata.every(item => Object.keys(item.body as object).every(key => key === "publicKeyThumbprint")))
    assert(f.metadata.some(item => item.cookie?.includes("cloud=session")), "new owner cookies are carried into grant admission")
  } finally {await f.close()}
})

test("hosted viewer keeps history browser-to-kernel; GET, HEAD, prefetch and signed-out navigation use admission only", async () => {
  const f = await fixture()
  try {
    for (const headers of [{}, {"sec-purpose": "prefetch"}]) {
      const page = await f.app.inject({url: "/agent-triggers/deployment/?prompt=RUNTIME_ONLY_MARKER", method: headers["sec-purpose"] ? "GET" : "HEAD", headers})
      assert.equal(page.statusCode, 200)
      assert(f.metadata.at(-1)?.url.includes("access=read"))
    }
    const page = await f.app.inject({url: "/agent-triggers/deployment/"})
    assert.match(page.body, /data-cloud-deployment="deployment"/);assert.match(page.body, /\/assets\/agent-trigger-viewer.js/)
    assert.equal(page.headers["cache-control"], "private, no-cache")
    assert.equal(page.headers["content-security-policy"], "default-src 'self'; frame-ancestors 'none'")
    assert.equal(page.headers["x-content-type-options"], "nosniff")
    assert.equal(page.headers["x-internal-admission"], undefined)
    assert.deepEqual(f.requests, [])
    const invoked = await f.app.inject({url: "/agent-triggers/deployment/?prompt=RUNTIME_ONLY_MARKER", headers: {"x-chariox-browser-session": "owner"}})
    assert.equal(invoked.statusCode, 202);assert.equal(f.requests.length, 1)
    assert(!JSON.stringify(f.metadata).includes("RUNTIME_ONLY_MARKER"))
    f.deny(401)
    const login = await f.app.inject({url: "/agent-triggers/deployment/?prompt=PRIVATE", headers: {accept: "text/html"}})
    assert.equal(login.statusCode, 302);assert.equal(login.headers.location, "/auth/login?returnTo=%2Fagent-triggers%2Fdeployment%2F")
  } finally {await f.close()}
})

test("hosted HTML invocation redirects to its firing without replay on refresh", async () => {
  const f = await fixture()
  try {
    const fired = await f.app.inject({url: "/agent-triggers/deployment/?prompt=hello", headers: {accept: "text/html"}})
    assert.equal(fired.statusCode, 303)
    assert.equal(fired.headers.location, "/agent-triggers/deployment/#firing-prompt-1")
    assert.equal(fired.headers["cache-control"], "no-store")
    assert.equal(f.requests.length, 1)
    const conversation = await f.app.inject({url: fired.headers.location!.split("#")[0]!, headers: {accept: "text/html"}})
    assert.equal(conversation.statusCode, 200)
    assert.match(conversation.body, /data-cloud-deployment="deployment"/)
    assert.equal(f.requests.length, 1)
  } finally {await f.close()}
})

test("hosted ingress refuses unauthorized work, oversized bodies and excess requests before dispatch", async () => {
  const f = await fixture()
  try {
    f.deny(403)
    assert.equal((await f.app.inject({method: "POST", url: "/agent-triggers/deployment/", payload: "denied"})).statusCode, 403)
    assert.deepEqual(f.requests, [])
    f.deny(0)
    assert.equal((await f.app.inject({method: "POST", url: "/agent-triggers/deployment/", payload: "x".repeat(65_537)})).statusCode, 413)
    assert.deepEqual(f.requests, [])
    for (let i = 0; i < 3; i++) assert.equal((await f.app.inject({method: "POST", url: "/agent-triggers/deployment/", payload: "ok"})).statusCode, 202)
    assert.equal((await f.app.inject({method: "POST", url: "/agent-triggers/deployment/", payload: "limited"})).statusCode, 429)
    assert.equal(f.requests.length, 3)
  } finally {await f.close()}
})
